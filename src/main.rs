use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use cfg_if::cfg_if;
use clap::Parser;
use futures::{
    pin_mut,
    stream::{select, TryStreamExt},
    StreamExt,
};
use rabbit_digger_pro::{config::ImportSource, schema, util::exit_stream, ApiServer, App};
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
}

impl ApiServerArgs {
    fn to_api_server(&self) -> ApiServer {
        ApiServer {
            bind: self.bind.clone(),
            access_token: self.access_token.clone(),
            web_ui: self.web_ui.clone(),
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

    app.run_api_server(args.api_server.to_api_server()).await?;

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
            let tracer = opentelemetry_jaeger::new_agent_pipeline()
                .with_service_name("rabbit_digger_pro")
                .install_batch(opentelemetry::runtime::Tokio)?;
            // only for debug
            // let tracer = opentelemetry::sdk::export::trace::stdout::new_pipeline().install_simple();
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

    tr.with(
        tracing_subscriber::fmt::layer()
            .with_writer(std::io::stdout)
            .with_filter(dynamic_filter_fn(move |metadata, ctx| {
                log_filter.enabled(metadata, ctx.clone())
            })),
    )
    .with(json_layer)
    .init();

    match &args.cmd {
        Some(Command::GenerateSchema { path }) => {
            if let Some(path) = path {
                schema::write_schema(path).await?;
            } else {
                let s = schema::generate_schema().await?;
                println!("{}", serde_json::to_string(&s)?);
            }
            return Ok(());
        }
        Some(Command::Server { api_server }) => {
            let app = App::new().await?;

            app.run_api_server(api_server.to_api_server()).await?;

            tokio::signal::ctrl_c().await?;

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
