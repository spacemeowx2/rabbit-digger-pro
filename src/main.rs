use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use cfg_if::cfg_if;
use clap::Parser;
use futures::{
    pin_mut,
    stream::{select, TryStreamExt},
    StreamExt,
};
use rabbit_digger_pro::{config::ImportSource, schema, util::exit_stream, ApiServerConfig, App};
use tracing_subscriber::filter::dynamic_filter_fn;

#[cfg(feature = "telemetry")]
mod tracing_helper;

#[derive(Parser)]
struct ApiServerArgs {
    /// HTTP endpoint bind address.
    #[clap(short, long, env = "RD_BIND")]
    bind: Option<String>,

    /// Access token.
    #[structopt(long, env = "RD_ACCESS_TOKEN")]
    access_token: Option<String>,

    /// Web UI. Folder path.
    #[structopt(long, env = "RD_WEB_UI")]
    web_ui: Option<String>,
}

#[derive(Parser)]
struct Args {
    /// Path to config file
    #[clap(short, long, env = "RD_CONFIG", default_value = "config.yaml")]
    config: PathBuf,

    #[clap(flatten)]
    api_server: ApiServerArgs,

    /// Write generated config to path
    #[clap(long)]
    write_config: Option<PathBuf>,

    #[clap(subcommand)]
    cmd: Option<Command>,
}

#[derive(Parser)]
enum Command {
    /// Generate schema to path, if not present, output to stdout
    GenerateSchema { path: Option<PathBuf> },
    /// Run in server mode
    Server {
        #[clap(flatten)]
        api_server: ApiServerArgs,
    },
    /// Manage system service
    Service {
        #[clap(subcommand)]
        action: rabbit_digger_pro::service::ServiceAction,
    },
}

impl ApiServerArgs {
    fn to_api_server_config(&self) -> ApiServerConfig {
        ApiServerConfig {
            bind: self.bind.clone(),
            access_token: self.access_token.clone(),
            web_ui: self.web_ui.clone(),
            source_sender: None,
            log_file_path: None,
        }
    }
}

async fn write_config(path: impl AsRef<Path>, cfg: &rabbit_digger::Config) -> Result<()> {
    let content = serde_yaml::to_string(cfg)?;
    tokio::fs::write(path, content.as_bytes()).await?;
    Ok(())
}

async fn real_main(args: Args) -> Result<()> {
    let app = App::new().await?;

    app.run_api_server(args.api_server.to_api_server_config())
        .await?;

    let config_path = args.config.clone();
    let write_config_path = args.write_config;

    let config_stream = app
        .cfg_mgr
        .config_stream(ImportSource::Path(config_path))
        .await?
        .and_then(|c: rabbit_digger::Config| async {
            if let Some(path) = &write_config_path {
                write_config(path, &c).await?;
            };
            Ok(c)
        });
    let exit_stream = exit_stream().map(|i| {
        let r: Result<rabbit_digger::Config> = match i {
            Ok(_) => Err(rd_interface::Error::AbortedByUser.into()),
            Err(e) => Err(e.into()),
        };
        r
    });

    let stream = select(config_stream, exit_stream);

    pin_mut!(stream);
    app.rd
        .start_stream(stream)
        .await
        .context("Failed to run RabbitDigger")?;

    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    use tracing_subscriber::{layer::SubscriberExt, prelude::*, EnvFilter};
    if std::env::var_os("RUST_LOG").is_none() {
        std::env::set_var(
            "RUST_LOG",
            "rabbit_digger=debug,rabbit_digger_pro=debug,rd_std=debug,raw=debug,ss=debug,tower_http=info",
        )
    }
    let tr = tracing_subscriber::registry();

    cfg_if! {
        if #[cfg(feature = "console")] {
            let (layer, server) = console_subscriber::ConsoleLayer::builder().with_default_env().build();
            tokio::spawn(server.serve());
            let tr = tr.with(layer);
        }
    }

    cfg_if! {
        if #[cfg(feature = "telemetry")] {
            // NOTE: Jaeger agent pipeline was removed because it depends on an old
            // opentelemetry version and breaks the dependency graph after upgrades.
            // This keeps telemetry wiring compiling (noop tracer by default).
            let tracer = opentelemetry::global::tracer("rabbit_digger_pro");
            let tracer_filter =
                EnvFilter::new("rabbit_digger=trace,rabbit_digger_pro=trace,rd_std=trace");
            let opentelemetry = tracing_opentelemetry::layer().with_tracer(tracer);
            let tr = tr.with(
                opentelemetry.with_filter(dynamic_filter_fn(move |metadata, ctx| {
                    tracer_filter.enabled(metadata, ctx.clone())
                })),
            );
        }
    }

    let log_filter = EnvFilter::from_default_env();
    let log_writer_filter = EnvFilter::new(
        "rabbit_digger=debug,rabbit_digger_pro=debug,rd_std=debug,raw=debug,ss=debug",
    );
    let json_layer = tracing_subscriber::fmt::layer().json();
    #[cfg(feature = "telemetry")]
    let json_layer = json_layer.event_format(tracing_helper::TraceIdFormat);
    let json_layer = json_layer
        .with_writer(rabbit_digger_pro::log::LogWriter::new)
        .with_filter(dynamic_filter_fn(move |metadata, ctx| {
            log_writer_filter.enabled(metadata, ctx.clone())
        }));

    // In daemon mode, also log to a file
    let is_daemon = matches!(
        &args.cmd,
        Some(Command::Service {
            action: rabbit_digger_pro::service::ServiceAction::Run { .. }
        })
    );
    let file_layer = if is_daemon {
        let log_dir = dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("rabbit_digger_pro");
        std::fs::create_dir_all(&log_dir).ok();
        let log_path = log_dir.join("daemon.log");
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .expect("Failed to open daemon log file");
        let file_filter = EnvFilter::new(
            "rabbit_digger=info,rabbit_digger_pro=info,rd_std=info,raw=info,ss=info",
        );
        Some(
            tracing_subscriber::fmt::layer()
                .json()
                .with_ansi(false)
                .with_writer(std::sync::Mutex::new(file))
                .with_filter(dynamic_filter_fn(move |metadata, ctx| {
                    file_filter.enabled(metadata, ctx.clone())
                })),
        )
    } else {
        None
    };

    tr.with(
        tracing_subscriber::fmt::layer()
            .with_writer(std::io::stdout)
            .with_filter(dynamic_filter_fn(move |metadata, ctx| {
                log_filter.enabled(metadata, ctx.clone())
            })),
    )
    .with(json_layer)
    .with(file_layer)
    .init();

    match args.cmd {
        Some(Command::GenerateSchema { ref path }) => {
            if let Some(path) = path {
                schema::write_schema(path).await?;
            } else {
                let s = schema::generate_schema().await?;
                println!("{}", serde_json::to_string(&s)?);
            }
            return Ok(());
        }
        Some(Command::Server { ref api_server }) => {
            let app = App::new().await?;

            app.run_api_server(api_server.to_api_server_config())
                .await?;

            tokio::signal::ctrl_c().await?;

            return Ok(());
        }
        Some(Command::Service { action }) => {
            rabbit_digger_pro::service::run(action).await?;
            return Ok(());
        }
        None => {}
    }

    match real_main(args).await {
        Ok(()) => {}
        Err(e) => tracing::error!("Process exit: {:?}", e),
    }

    Ok(())
}

#[cfg(feature = "jemalloc")]
#[global_allocator]
static ALLOC: jemallocator::Jemalloc = jemallocator::Jemalloc;

#[cfg(feature = "mimalloc")]
#[global_allocator]
static ALLOC: mimalloc::MiMalloc = mimalloc::MiMalloc;
