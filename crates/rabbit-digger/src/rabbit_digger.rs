use std::{cell::RefCell, collections::BTreeMap, fmt, mem::replace, time::Duration};

use crate::{
    config::{self, init_default_net},
    rabbit_digger::running::{RunningNet, RunningServer, RunningServerNet},
    registry::{Registry, RegistrySchema},
};
use anyhow::{anyhow, Context, Result};
use futures::{
    future::{try_select, Either},
    stream::FuturesUnordered,
    Stream, StreamExt, TryStreamExt,
};
use rd_interface::{
    config::{
        serialize_with_fields, CompactVecString, NetRef, VisitorContext, ALL_SERIALIZE_FIELDS,
    },
    registry::NetGetter,
    Arc, Error, IntoDyn, Net, Server, Value,
};
use tokio::{
    pin,
    sync::RwLock,
    task::{yield_now, JoinError},
    time::timeout,
};
use uuid::Uuid;

use self::connection_manager::{ConnectionManager, ConnectionState};

mod connection_manager;
mod event;
mod running;

struct RunningEntities {
    nets: BTreeMap<String, Arc<RunningNet>>,
    servers: BTreeMap<String, ServerInfo>,
}

struct SerializedConfig {
    id: String,
    all_fields: String,
    simple_fields: String,
}

#[allow(dead_code)]
struct Running {
    config: RwLock<SerializedConfig>,
    entities: RunningEntities,
}

enum State {
    WaitConfig,
    Running(Running),
}

impl State {
    fn running(&self) -> Option<&Running> {
        match self {
            State::Running(running) => Some(running),
            _ => None,
        }
    }
}

struct Inner {
    state: RwLock<State>,
    conn_mgr: ConnectionManager,
}

impl Drop for Inner {
    fn drop(&mut self) {
        self.conn_mgr.stop()
    }
}

#[derive(Clone)]
pub struct RabbitDigger {
    inner: Arc<Inner>,
    registry: Arc<Registry>,
}

impl fmt::Debug for RabbitDigger {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RabbitDigger").finish()
    }
}

impl RabbitDigger {
    pub async fn new(registry: Registry) -> Result<RabbitDigger> {
        let manager = ConnectionManager::new();

        let inner = Inner {
            state: RwLock::new(State::WaitConfig),
            conn_mgr: manager,
        };

        Ok(RabbitDigger {
            inner: Arc::new(inner),
            registry: Arc::new(registry),
        })
    }
    pub async fn stop(&self) -> Result<()> {
        let inner = &self.inner;
        let state = inner.state.read().await;

        match &*state {
            State::Running(Running {
                entities: RunningEntities { servers, .. },
                ..
            }) => {
                for i in servers.values() {
                    i.running_server.stop().await?;
                }
            }
            _ => {}
        };
        // release the lock to allow other join tasks to write the state
        drop(state);

        self.join().await?;

        Ok(())
    }
    pub async fn join(&self) -> Result<()> {
        let inner = &self.inner;

        match &*inner.state.read().await {
            State::WaitConfig => return Ok(()),
            State::Running(Running {
                entities: RunningEntities { servers, .. },
                ..
            }) => {
                let mut race = FuturesUnordered::new();
                for (name, i) in servers {
                    race.push(async move {
                        i.running_server.join().await;
                        if let Some(result) = i.running_server.take_result().await {
                            (name, result)
                        } else {
                            tracing::warn!("Failed to take result. This shouldn't happend");
                            (name, Ok(()))
                        }
                    });
                }

                while let Some((name, r)) = race.next().await {
                    match r {
                        Err(e) if !e.is::<JoinError>() => {
                            tracing::warn!("Server {} stopped with error: {:?}", name, e);
                        }
                        _ => {}
                    }
                }
            }
        };

        let state = &mut *inner.state.write().await;
        *state = State::WaitConfig;

        Ok(())
    }

    // get current connection state
    pub async fn connection<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&ConnectionState) -> R,
    {
        self.inner.conn_mgr.borrow_state(f)
    }

    // get state
    pub async fn state_str(&self) -> Result<&'static str> {
        let state = self.inner.state.read().await;
        Ok(match &*state {
            State::WaitConfig => "WaitConfig",
            State::Running { .. } => "Running",
        })
    }

    // get registry schema
    pub async fn registry<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&RegistrySchema) -> R,
    {
        f(&self.registry.get_registry_schema())
    }

    // start all server, all server run in background.
    pub async fn start(&self, mut config: config::Config) -> Result<()> {
        let inner = &self.inner;

        tracing::debug!("Registry:\n{}", self.registry);

        let entities = self
            .registry
            .build_entities(&mut config, &inner.conn_mgr)
            .context("Failed to build server")?;
        tracing::debug!(
            "net and server are built. net count: {}, server count: {}",
            entities.nets.len(),
            entities.servers.len()
        );

        tracing::info!("Server:\n{}", ServerList(&entities.servers));

        self.stop().await?;
        let state = &mut *inner.state.write().await;

        for ServerInfo { running_server, .. } in entities.servers.values() {
            running_server.start().await?;
        }

        *state = State::Running(Running {
            config: RwLock::new(SerializedConfig {
                all_fields: serialize_with_fields(ALL_SERIALIZE_FIELDS.to_vec(), || {
                    serde_json::to_string(&config)
                })?,
                simple_fields: serde_json::to_string(&config)?,
                id: config.id,
            }),
            entities,
        });

        Ok(())
    }

    pub async fn is_running(&self) -> bool {
        matches!(*self.inner.state.read().await, State::Running { .. })
    }

    pub async fn start_stream<S>(self, config_stream: S) -> Result<()>
    where
        S: Stream<Item = Result<config::Config>>,
    {
        futures::pin_mut!(config_stream);

        let mut config = match timeout(Duration::from_secs(30), config_stream.try_next()).await {
            Ok(Ok(Some(cfg))) => cfg,
            Ok(Err(e)) => return Err(e.context("Failed to get first config.")),
            Err(_) | Ok(Ok(None)) => {
                return Err(anyhow!(
                    "Waiting too long for first config_stream, can not start."
                ))
            }
        };

        let reason = loop {
            tracing::info!("rabbit digger is starting...");

            self.start(config).await?;

            let new_config = {
                let join_fut = self.join();
                pin!(join_fut);

                match try_select(join_fut, config_stream.try_next()).await {
                    Ok(Either::Left((_, cfg_fut))) => {
                        tracing::info!("Exited normally, waiting for next config...");
                        cfg_fut.await
                    }
                    Ok(Either::Right((cfg, _))) => Ok(cfg),
                    Err(Either::Left((e, cfg_fut))) => {
                        tracing::error!(
                            "Rabbit digger went to error: {:?}, waiting for next config...",
                            e
                        );
                        cfg_fut.await
                    }
                    Err(Either::Right((e, _))) => Err(e),
                }
            };

            config = match new_config {
                Ok(Some(v)) => v,
                Ok(None) => break Ok(()),
                Err(e) => break Err(e),
            };

            self.stop().await?;
        };

        tracing::info!(
            "rabbit digger is exiting... reason: {:?} active connections: {}",
            reason,
            self.inner.conn_mgr.borrow_state(|s| s.connection_count())
        );

        self.stop().await?;

        let mut close_count = 0;

        while self.inner.conn_mgr.borrow_state(|s| s.connection_count()) > 0 {
            close_count += self.inner.conn_mgr.stop_connections();
            // Wait connections to exit.
            yield_now().await;
        }

        tracing::info!("{} connections are closed.", close_count);

        Ok(())
    }

    pub async fn get_net(&self, name: &str) -> Result<Option<Arc<RunningNet>>> {
        let state = self.inner.state.read().await;
        match &*state {
            State::Running(Running {
                entities: RunningEntities { nets, .. },
                ..
            }) => Ok(nets.get(name).cloned()),
            _ => Err(anyhow!("Not running")),
        }
    }

    // Update net when running.
    pub async fn update_net<F>(&self, net_name: &str, update: F) -> Result<()>
    where
        F: FnOnce(&mut config::Net),
    {
        let state = self.inner.state.read().await;
        match &*state {
            State::Running(Running {
                config,
                entities: RunningEntities { nets, .. },
                ..
            }) => {
                let mut serialized_config = config.write().await;
                let mut config: config::Config =
                    serde_json::from_str(&serialized_config.all_fields)?;

                if let (Some(cfg), Some(running_net)) =
                    (config.net.get_mut(net_name), nets.get(net_name))
                {
                    let mut new_cfg = cfg.clone();
                    update(&mut new_cfg);

                    let net = self
                        .registry
                        .build_net(net_name, &mut new_cfg, &mut |key, _| {
                            let name = key
                                .represent()
                                .as_str()
                                .ok_or_else(|| Error::other(format!("Net not found")))?;
                            nets.get(name)
                                .map(|i| i.as_net())
                                .ok_or_else(|| Error::NotFound(name.to_string()))
                        })?;
                    running_net.update_net(net);

                    *cfg = new_cfg;
                    serialized_config.all_fields =
                        serialize_with_fields(ALL_SERIALIZE_FIELDS.to_vec(), || {
                            serde_json::to_string(&config)
                        })?;
                    serialized_config.simple_fields = serde_json::to_string(&config)?;
                }
                return Ok(());
            }
            _ => {
                return Err(anyhow!("Not running"));
            }
        };
    }

    pub async fn get_id(&self) -> Option<String> {
        let state = self.inner.state.read().await;
        match &*state {
            State::Running(Running { config, .. }) => Some(config.read().await.id.clone()),
            _ => None,
        }
    }

    pub async fn get_config<F, R>(&self, f: F) -> Result<R>
    where
        F: FnOnce(&str) -> R,
    {
        let state = self.inner.state.read().await;
        match state.running() {
            Some(i) => Ok(f(&*i.config.read().await.simple_fields)),
            None => Err(anyhow!("Not running")),
        }
    }

    // Stop the connection by uuid
    pub async fn stop_connection(&self, uuid: Uuid) -> Result<bool> {
        Ok(self.inner.conn_mgr.stop_connection(uuid))
    }

    // Stop all connections
    pub async fn stop_connections(&self) -> Result<usize> {
        Ok(self.inner.conn_mgr.stop_connections())
    }
}

pub struct ServerInfo {
    name: String,
    running_server: RunningServer,
    config: Value,
}

struct ServerList<'a>(&'a BTreeMap<String, ServerInfo>);

impl fmt::Display for ServerInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.name, self.config)
    }
}

impl<'a> fmt::Display for ServerList<'a> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for i in self.0.values() {
            writeln!(f, "\t{}", i)?;
        }
        Ok(())
    }
}

impl Registry {
    fn build_net(
        &self,
        name: &str,
        i: &mut config::Net,
        getter: NetGetter,
    ) -> rd_interface::Result<Net> {
        let net_item = self.get_net(&i.net_type)?;

        let net = rd_interface::error::ErrorContext::context(
            net_item.build(getter, &mut i.opt),
            format!("Failed to build net {:?}. Please check your config.", name),
        )?;

        Ok(net)
    }

    fn build_server(
        &self,
        name: &str,
        i: &mut config::Server,
        getter: NetGetter,
    ) -> rd_interface::Result<Server> {
        let server_item = self.get_server(&i.server_type)?;

        let server = rd_interface::error::ErrorContext::context(
            server_item.build(getter, &mut i.opt),
            format!(
                "Failed to build server {:?}. Please check your config.",
                name
            ),
        )?;

        Ok(server)
    }

    // Build all net and server, the nested net will be flatten. So the config may change.
    fn build_entities(
        &self,
        config: &mut config::Config,
        conn_mgr: &ConnectionManager,
    ) -> Result<RunningEntities> {
        let config::Config { net, server, .. } = config;
        init_default_net(net)?;
        let build_context = BuildContext::new(&self, net);

        let mut servers = BTreeMap::new();

        for (name, mut i) in server.iter_mut() {
            let server_name = &name;

            let mut load_server = || {
                let server = self.build_server(server_name, &mut i, &|name, ctx| {
                    build_context.get_server_net(
                        name,
                        ctx,
                        server_name.to_string(),
                        conn_mgr.clone(),
                    )
                })?;
                let server =
                    RunningServer::new(server_name.to_string(), i.server_type.clone(), server);
                servers.insert(
                    server_name.to_string(),
                    ServerInfo {
                        name: server_name.to_string(),
                        running_server: server,
                        config: i.opt.clone(),
                    },
                );
                Ok(()) as Result<()>
            };

            load_server().context(format!("Loading server {}", server_name))?;
        }

        Ok(RunningEntities {
            nets: build_context.take_net(),
            servers,
        })
    }
}

struct BuildContext<'a> {
    config: RefCell<&'a mut config::ConfigNet>,
    registry: &'a Registry,
    net_cache: RefCell<BTreeMap<String, Arc<RunningNet>>>,
    delimiter: &'a str,
}

impl<'a> BuildContext<'a> {
    fn new(registry: &'a Registry, config: &'a mut config::ConfigNet) -> Self {
        BuildContext {
            config: RefCell::new(config),
            registry,
            net_cache: RefCell::new(BTreeMap::new()),
            delimiter: "/",
        }
    }
    fn take_net(&self) -> BTreeMap<String, Arc<RunningNet>> {
        self.net_cache.replace(BTreeMap::new())
    }
    fn get_net(
        &self,
        net_ref: &mut NetRef,
        ctx: &VisitorContext,
        prefix: &CompactVecString,
    ) -> rd_interface::Result<Net> {
        let placeholder: config::Net = config::Net::new("circular reference", Value::Null);

        let name = match net_ref.represent() {
            Value::String(name) => name,
            net_cfg => {
                let mut key = prefix.clone();
                key.extend(ctx.path());

                let generated_name = key.join(self.delimiter);
                self.config.borrow_mut().insert(
                    generated_name.to_string(),
                    serde_json::from_value(net_cfg.clone())?,
                );

                *net_ref.represent_mut() = Value::String(generated_name);

                net_ref.represent().as_str().expect("Impossible")
            }
        };
        if let Some(net) = self.net_cache.borrow().get(name) {
            return Ok(net.as_net());
        }

        let mut cfg = self
            .config
            .borrow_mut()
            .get_mut(name)
            .map(|i| replace(i, placeholder))
            .ok_or(Error::NotFound(format!(
                "Failed to find net in config file: {}",
                name
            )))?;

        let prefix = ["net", name].iter().copied().collect();
        let net = RunningNet::new(
            name.to_string(),
            self.registry.build_net(name, &mut cfg, &|name, ctx| {
                self.get_net(name, ctx, &prefix)
            })?,
        );

        *self
            .config
            .borrow_mut()
            .get_mut(name)
            .expect("It must exist") = cfg;

        self.net_cache
            .borrow_mut()
            .insert(name.to_string(), net.clone());

        Ok(net.as_net())
    }
    fn get_server_net(
        &self,
        net_ref: &mut NetRef,
        ctx: &VisitorContext,
        server_name: String,
        conn_mgr: ConnectionManager,
    ) -> rd_interface::Result<Net> {
        let prefix = ["server", &server_name].iter().copied().collect();
        Ok(
            RunningServerNet::new(server_name, self.get_net(net_ref, ctx, &prefix)?, conn_mgr)
                .into_dyn(),
        )
    }
}
