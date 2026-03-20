mod coordinator;
mod store;

use std::time::{Duration, Instant};

use anyhow::Result;
use rabbit_digger::{ConnectFailureObservation, ObservationEvent, RabbitDigger};
use rd_interface::{context::common_field::DestDomain, Address, Context as RdContext};
use tokio::sync::Mutex;

use crate::{config::apply_selected_net, config::ConfigManager};

pub use self::coordinator::{
    CooldownState, PolicyActionKind, PolicyActionOutcome, PolicyActionRecord, PolicyCoordinator,
    PolicyStateView, PolicySuggestion, SuggestionStatus, POLICY_SCHEMA_VERSION,
};
use self::{
    coordinator::SelectCandidate,
    store::{PersistedPolicyState, PolicyStore},
};

const COOLDOWN_SECS: u64 = 30;
const PROBE_TIMEOUT_SECS: u64 = 3;
const MAX_PROBE_CANDIDATES: usize = 3;

#[derive(Clone)]
pub struct PolicyRuntime {
    inner: std::sync::Arc<Inner>,
}

struct Inner {
    rd: RabbitDigger,
    cfg_mgr: ConfigManager,
    store: PolicyStore,
    coordinator: Mutex<PolicyCoordinator>,
}

#[derive(Debug, Clone)]
struct SelectRouteContext {
    domain: String,
    select_net: String,
    current_target: String,
    candidates: Vec<String>,
    addr: Address,
}

impl PolicyRuntime {
    pub async fn new(rd: RabbitDigger, cfg_mgr: ConfigManager) -> Result<Self> {
        let store = PolicyStore::new_data().await?;
        Self::new_with_store(rd, cfg_mgr, store).await
    }

    async fn new_with_store(
        rd: RabbitDigger,
        cfg_mgr: ConfigManager,
        store: PolicyStore,
    ) -> Result<Self> {
        let persisted = store.load().await?;
        let runtime = Self {
            inner: std::sync::Arc::new(Inner {
                rd,
                cfg_mgr,
                store,
                coordinator: Mutex::new(PolicyCoordinator::from_persisted(
                    persisted.suggestions,
                    persisted.actions,
                )),
            }),
        };
        runtime.spawn_observer();
        Ok(runtime)
    }

    fn spawn_observer(&self) {
        let this = self.clone();
        let mut rx = this.inner.rd.subscribe_observations();
        tokio::spawn(async move {
            loop {
                match rx.recv().await {
                    Ok(event) => {
                        if let Err(error) = this.handle_observation(event).await {
                            tracing::warn!("policy observation handling failed: {error:?}");
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                        if let Err(error) = this.record_lagged(skipped).await {
                            tracing::warn!("policy lag handling failed: {error:?}");
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });
    }

    pub async fn state(&self) -> Result<PolicyStateView> {
        let coordinator = self.inner.coordinator.lock().await;
        Ok(coordinator.snapshot(self.inner.rd.is_running().await))
    }

    pub async fn actions(&self) -> Result<Vec<PolicyActionRecord>> {
        let coordinator = self.inner.coordinator.lock().await;
        Ok(coordinator.actions())
    }

    pub async fn suggestions(&self) -> Result<Vec<PolicySuggestion>> {
        let coordinator = self.inner.coordinator.lock().await;
        Ok(coordinator.suggestions())
    }

    pub async fn approve_suggestion(&self, id: &str) -> Result<Option<PolicySuggestion>> {
        let updated = {
            let mut coordinator = self.inner.coordinator.lock().await;
            coordinator.approve_suggestion(id)
        };
        self.persist().await?;
        Ok(updated)
    }

    pub async fn reject_suggestion(&self, id: &str) -> Result<Option<PolicySuggestion>> {
        let updated = {
            let mut coordinator = self.inner.coordinator.lock().await;
            coordinator.reject_suggestion(id)
        };
        self.persist().await?;
        Ok(updated)
    }

    async fn record_lagged(&self, skipped: u64) -> Result<()> {
        {
            let mut coordinator = self.inner.coordinator.lock().await;
            coordinator.record_observation_lag(skipped);
        }
        self.persist().await
    }

    async fn handle_observation(&self, event: ObservationEvent) -> Result<()> {
        match event {
            ObservationEvent::TcpConnectFailure(failure) => {
                self.handle_connect_failure(failure).await?;
            }
        }
        Ok(())
    }

    async fn handle_connect_failure(&self, failure: ConnectFailureObservation) -> Result<()> {
        let route = match self.extract_route_context(&failure).await? {
            Some(route) => route,
            None => {
                let mut coordinator = self.inner.coordinator.lock().await;
                coordinator.record_observation_dropped(
                    "failure event is missing a domain-backed select route".to_string(),
                );
                drop(coordinator);
                self.persist().await?;
                return Ok(());
            }
        };

        let cooldown_key = format!("{}@{}", route.domain, route.select_net);
        {
            let mut coordinator = self.inner.coordinator.lock().await;
            if coordinator.in_cooldown(&cooldown_key, failure.observed_at) {
                coordinator.record_cooldown_skip(
                    route.domain.clone(),
                    route.select_net.clone(),
                    route.current_target.clone(),
                );
                drop(coordinator);
                self.persist().await?;
                return Ok(());
            }
        }

        let preferred = {
            let coordinator = self.inner.coordinator.lock().await;
            coordinator.approved_target_for(&route.domain, &route.select_net)
        };
        let candidates = reorder_candidates(route.candidates.clone(), preferred.as_deref());
        let selected = self
            .probe_candidates(&route.addr, &candidates)
            .await?
            .map(|candidate| candidate.name);

        {
            let mut coordinator = self.inner.coordinator.lock().await;
            coordinator.set_cooldown(&cooldown_key, failure.observed_at + COOLDOWN_SECS);
        }

        if let Some(candidate) = selected {
            apply_selected_net(
                &self.inner.rd,
                self.inner.cfg_mgr.select_storage(),
                &route.select_net,
                &candidate,
            )
            .await?;
            {
                let mut coordinator = self.inner.coordinator.lock().await;
                coordinator.record_temporary_switch(
                    route.domain,
                    route.select_net,
                    route.current_target,
                    candidate,
                );
            }
        } else {
            let mut coordinator = self.inner.coordinator.lock().await;
            coordinator.record_probe_failure(
                route.domain,
                route.select_net,
                route.current_target,
                "no healthy candidate found during confirmation probe".to_string(),
            );
        }

        self.persist().await
    }

    async fn probe_candidates(
        &self,
        addr: &Address,
        candidates: &[String],
    ) -> Result<Option<SelectCandidate>> {
        for candidate in candidates.iter().take(MAX_PROBE_CANDIDATES) {
            if self.probe_candidate(candidate, addr).await?.is_some() {
                return Ok(Some(SelectCandidate {
                    name: candidate.clone(),
                }));
            }
        }
        Ok(None)
    }

    async fn probe_candidate(&self, net_name: &str, addr: &Address) -> Result<Option<u64>> {
        let Some(net) = self
            .inner
            .rd
            .get_net(net_name)
            .await?
            .map(|net| net.as_net())
        else {
            return Ok(None);
        };

        let start = Instant::now();
        let probe = async {
            let _socket = net.tcp_connect(&mut RdContext::new(), addr).await?;
            Ok::<u64, rd_interface::Error>(start.elapsed().as_millis() as u64)
        };
        match tokio::time::timeout(Duration::from_secs(PROBE_TIMEOUT_SECS), probe).await {
            Ok(Ok(latency_ms)) => Ok(Some(latency_ms)),
            Ok(Err(_)) | Err(_) => Ok(None),
        }
    }

    async fn extract_route_context(
        &self,
        failure: &ConnectFailureObservation,
    ) -> Result<Option<SelectRouteContext>> {
        let ctx = match RdContext::from_value(failure.ctx.clone()) {
            Ok(ctx) => ctx,
            Err(_) => return Ok(None),
        };
        let Some(domain) = extract_domain(&ctx, &failure.addr)? else {
            return Ok(None);
        };
        let config = self
            .inner
            .rd
            .get_config(|raw| raw.to_string())
            .await
            .ok()
            .and_then(|raw| serde_json::from_str::<rabbit_digger::config::Config>(&raw).ok());
        let Some(config) = config else {
            return Ok(None);
        };

        let mut select_net = None;
        for name in ctx.net_list() {
            if config
                .net
                .get(name)
                .map(|net| net.net_type.as_str() == "select")
                .unwrap_or(false)
            {
                select_net = Some(name.to_string());
            }
        }

        let Some(select_net) = select_net else {
            return Ok(None);
        };
        let Some(select_cfg) = config.net.get(&select_net) else {
            return Ok(None);
        };
        let Some(opt) = select_cfg.opt.as_object() else {
            return Ok(None);
        };
        let current_target = opt
            .get("selected")
            .and_then(|value| value.as_str())
            .map(ToOwned::to_owned);
        let candidates = opt
            .get("list")
            .and_then(|value| value.as_array())
            .map(|items| {
                items
                    .iter()
                    .filter_map(|value| value.as_str())
                    .map(ToOwned::to_owned)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let Some(current_target) = current_target else {
            return Ok(None);
        };
        let candidates = candidates
            .into_iter()
            .filter(|candidate| candidate != &current_target)
            .collect::<Vec<_>>();
        if candidates.is_empty() {
            return Ok(None);
        }

        Ok(Some(SelectRouteContext {
            domain,
            select_net,
            current_target,
            candidates,
            addr: failure.addr.clone(),
        }))
    }

    async fn persist(&self) -> Result<()> {
        let persisted = {
            let coordinator = self.inner.coordinator.lock().await;
            PersistedPolicyState::new(coordinator.suggestions(), coordinator.actions())
        };
        self.inner.store.save(&persisted).await
    }

    #[cfg(test)]
    pub async fn add_pending_suggestion_for_test(
        &self,
        domain: &str,
        select_net: &str,
        current_target: &str,
        suggested_target: &str,
    ) -> Result<()> {
        {
            let mut coordinator = self.inner.coordinator.lock().await;
            coordinator.insert_pending_suggestion(
                domain.to_string(),
                select_net.to_string(),
                current_target.to_string(),
                suggested_target.to_string(),
            );
        }
        self.persist().await
    }

    #[cfg(test)]
    pub async fn new_for_test(
        rd: RabbitDigger,
        cfg_mgr: ConfigManager,
        storage: std::sync::Arc<dyn crate::storage::Storage>,
    ) -> Result<Self> {
        Self::new_with_store(rd, cfg_mgr, PolicyStore::new(storage)).await
    }
}

fn reorder_candidates(candidates: Vec<String>, preferred: Option<&str>) -> Vec<String> {
    let Some(preferred) = preferred else {
        return candidates;
    };

    let mut ordered = Vec::with_capacity(candidates.len());
    if candidates.iter().any(|candidate| candidate == preferred) {
        ordered.push(preferred.to_string());
    }
    for candidate in candidates {
        if candidate != preferred {
            ordered.push(candidate);
        }
    }
    ordered
}

fn extract_domain(ctx: &RdContext, addr: &Address) -> Result<Option<String>> {
    if let Some(dest) = ctx.get_common::<DestDomain>()? {
        return Ok(Some(dest.0.domain));
    }
    match addr {
        Address::Domain(domain, _) => Ok(Some(domain.clone())),
        Address::SocketAddr(_) => Ok(None),
    }
}
