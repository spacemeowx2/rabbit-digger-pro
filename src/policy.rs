mod coordinator;
mod store;

use std::time::{Duration, Instant};

use anyhow::Result;
use rabbit_digger::{ConnectFailureObservation, ObservationEvent, RabbitDigger};
use rd_interface::{
    config::{serialize_with_fields, CompactVecString, NetRef, ALL_SERIALIZE_FIELDS},
    context::common_field::DestDomain,
    Address, Context as RdContext,
};
use rd_std::rule::config::{DomainMatcher, DomainMatcherMethod, Matcher, RuleItem, RuleNetConfig};
use serde_json::Value;
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
const OVERLAY_RECONCILE_INTERVAL_SECS: u64 = 1;

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
        runtime.spawn_overlay_reconciler();
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

    fn spawn_overlay_reconciler(&self) {
        let this = self.clone();
        tokio::spawn(async move {
            let mut ticker =
                tokio::time::interval(Duration::from_secs(OVERLAY_RECONCILE_INTERVAL_SECS));
            let mut last_config = None;
            loop {
                ticker.tick().await;
                let current_config = match this.current_runtime_config_raw().await {
                    Ok(config) => config,
                    Err(error) => {
                        tracing::warn!("policy overlay fingerprint read failed: {error:?}");
                        continue;
                    }
                };
                if current_config == last_config {
                    continue;
                }
                if current_config.is_some() {
                    if let Err(error) = this.reconcile_overlays_now().await {
                        tracing::warn!("policy overlay reconcile failed: {error:?}");
                    }
                }
                last_config = current_config;
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
        if updated.is_some() {
            if let Err(error) = self.reconcile_overlays_now().await {
                tracing::warn!("policy overlay reconcile after approval failed: {error:?}");
            }
        }
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

    async fn current_runtime_config_raw(&self) -> Result<Option<String>> {
        match self
            .inner
            .rd
            .get_config_all_fields(|raw| raw.to_string())
            .await
        {
            Ok(raw) => Ok(Some(raw)),
            Err(_) => Ok(None),
        }
    }

    async fn running_config_all_fields(&self) -> Result<Option<rabbit_digger::config::Config>> {
        let Some(raw) = self.current_runtime_config_raw().await? else {
            return Ok(None);
        };
        Ok(Some(serde_json::from_str(&raw)?))
    }

    async fn reconcile_overlays_now(&self) -> Result<()> {
        let suggestions = {
            let coordinator = self.inner.coordinator.lock().await;
            coordinator.latest_approved_suggestions()
        };
        if suggestions.is_empty() {
            return Ok(());
        }

        let mut changed = false;
        for suggestion in suggestions {
            changed |= self.reconcile_overlay_for_suggestion(&suggestion).await?;
        }

        if changed {
            self.persist().await?;
        }
        Ok(())
    }

    async fn reconcile_overlay_for_suggestion(
        &self,
        suggestion: &PolicySuggestion,
    ) -> Result<bool> {
        let Some(config) = self.running_config_all_fields().await? else {
            return Ok(false);
        };

        if !config.net.contains_key(&suggestion.suggested_target) {
            let mut coordinator = self.inner.coordinator.lock().await;
            coordinator.record_overlay_failed(
                suggestion.domain.clone(),
                suggestion.select_net.clone(),
                suggestion.current_target.clone(),
                suggestion.suggested_target.clone(),
                format!(
                    "approved target {} is not present in the running config",
                    suggestion.suggested_target
                ),
            );
            return Ok(true);
        }

        let mut relevant_rule_net_count = 0usize;
        let mut updates = Vec::new();

        for (rule_net_name, net) in &config.net {
            match inspect_overlay_rule_net(
                net,
                &suggestion.domain,
                &suggestion.select_net,
                &suggestion.suggested_target,
            )? {
                OverlayRuleNetState::NotRelevant => {}
                OverlayRuleNetState::AlreadyApplied => {
                    relevant_rule_net_count += 1;
                }
                OverlayRuleNetState::NeedsUpdate(opt) => {
                    relevant_rule_net_count += 1;
                    updates.push((rule_net_name.clone(), opt));
                }
            }
        }

        if relevant_rule_net_count == 0 {
            let mut coordinator = self.inner.coordinator.lock().await;
            coordinator.record_overlay_failed(
                suggestion.domain.clone(),
                suggestion.select_net.clone(),
                suggestion.current_target.clone(),
                suggestion.suggested_target.clone(),
                format!(
                    "no rule net references select net {} in the running config",
                    suggestion.select_net
                ),
            );
            return Ok(true);
        }

        if updates.is_empty() {
            return Ok(false);
        }

        let mut applied_rule_nets = Vec::new();
        let mut failures = Vec::new();
        for (rule_net_name, new_opt) in updates {
            let update_result = self
                .inner
                .rd
                .update_net(&rule_net_name, |net| {
                    net.opt = new_opt.clone();
                })
                .await;
            match update_result {
                Ok(()) => applied_rule_nets.push(rule_net_name),
                Err(error) => failures.push(format!("{rule_net_name}: {error:#}")),
            }
        }

        let mut coordinator = self.inner.coordinator.lock().await;
        if failures.is_empty() {
            coordinator.record_overlay_applied(
                suggestion.domain.clone(),
                suggestion.select_net.clone(),
                suggestion.current_target.clone(),
                suggestion.suggested_target.clone(),
                &applied_rule_nets,
            );
        } else {
            let detail = if applied_rule_nets.is_empty() {
                format!("failed to apply runtime overlay: {}", failures.join("; "))
            } else {
                format!(
                    "partially applied runtime overlay to [{}], but failed on {}",
                    applied_rule_nets.join(", "),
                    failures.join("; ")
                )
            };
            coordinator.record_overlay_failed(
                suggestion.domain.clone(),
                suggestion.select_net.clone(),
                suggestion.current_target.clone(),
                suggestion.suggested_target.clone(),
                detail,
            );
        }

        Ok(true)
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

    #[cfg(test)]
    pub async fn reconcile_overlays_for_test(&self) -> Result<()> {
        self.reconcile_overlays_now().await
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

enum OverlayRuleNetState {
    NotRelevant,
    AlreadyApplied,
    NeedsUpdate(Value),
}

fn inspect_overlay_rule_net(
    net: &rabbit_digger::config::Net,
    domain: &str,
    select_net: &str,
    suggested_target: &str,
) -> Result<OverlayRuleNetState> {
    if net.net_type != "rule" {
        return Ok(OverlayRuleNetState::NotRelevant);
    }

    let mut rule_config: RuleNetConfig = serde_json::from_value(net.opt.clone())?;
    let references_select = rule_config
        .rule
        .iter()
        .any(|rule| rule.target.represent().as_str() == Some(select_net));
    if !references_select {
        return Ok(OverlayRuleNetState::NotRelevant);
    }

    if rule_config
        .rule
        .iter()
        .any(|rule| is_exact_overlay_rule(rule, domain, suggested_target))
    {
        return Ok(OverlayRuleNetState::AlreadyApplied);
    }

    rule_config
        .rule
        .insert(0, build_exact_overlay_rule(domain, suggested_target));
    let new_opt = serialize_with_fields(ALL_SERIALIZE_FIELDS.to_vec(), || {
        serde_json::to_value(rule_config)
    })?;
    Ok(OverlayRuleNetState::NeedsUpdate(new_opt))
}

fn build_exact_overlay_rule(domain: &str, suggested_target: &str) -> RuleItem {
    RuleItem {
        target: NetRef::new(Value::String(suggested_target.to_string())),
        matcher: Matcher::Domain(DomainMatcher {
            method: DomainMatcherMethod::Match,
            domain: CompactVecString::from(domain.to_string()),
        }),
    }
}

fn is_exact_overlay_rule(rule: &RuleItem, domain: &str, suggested_target: &str) -> bool {
    let Matcher::Domain(domain_matcher) = &rule.matcher else {
        return false;
    };
    matches!(domain_matcher.method, DomainMatcherMethod::Match)
        && domain_matcher.domain == vec![domain]
        && rule.target.represent().as_str() == Some(suggested_target)
}

#[cfg(test)]
mod tests {
    use rabbit_digger::config::{Config, Net, Server};
    use serde_json::json;

    use super::*;

    fn overlay_test_config() -> Config {
        let mut cfg = Config::default();
        cfg.id = "overlay-test".to_string();
        rabbit_digger::config::init_default_net(&mut cfg.net).unwrap();
        cfg.net.insert(
            "proxy".to_string(),
            Net::new(
                "select",
                json!({
                    "selected": "local",
                    "list": ["local", "noop"]
                }),
            ),
        );
        cfg.net.insert(
            "route".to_string(),
            Net::new(
                "rule",
                json!({
                    "rule": [
                        {
                            "type": "any",
                            "target": "proxy"
                        }
                    ]
                }),
            ),
        );
        cfg.server.insert(
            "socks".to_string(),
            Server::new(
                "socks5",
                json!({
                    "bind": "127.0.0.1:0",
                    "listen": "local",
                    "net": "route"
                }),
            ),
        );
        cfg
    }

    async fn running_config_all_fields(rd: &RabbitDigger) -> Config {
        let raw = rd
            .get_config_all_fields(|raw| raw.to_string())
            .await
            .unwrap();
        serde_json::from_str(&raw).unwrap()
    }

    fn overlay_rule_count(cfg: &Config, rule_net: &str, domain: &str, target: &str) -> usize {
        let rule_cfg: RuleNetConfig =
            serde_json::from_value(cfg.net[rule_net].opt.clone()).unwrap();
        rule_cfg
            .rule
            .iter()
            .filter(|rule| is_exact_overlay_rule(rule, domain, target))
            .count()
    }

    #[tokio::test]
    async fn test_approve_suggestion_applies_overlay_without_duplicates() {
        let app = crate::App::new().await.unwrap();
        app.rd.start(overlay_test_config()).await.unwrap();

        app.policy
            .add_pending_suggestion_for_test("x.com", "proxy", "local", "noop")
            .await
            .unwrap();
        let suggestion_id = app.policy.suggestions().await.unwrap()[0].id.clone();

        let approved = app
            .policy
            .approve_suggestion(&suggestion_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(approved.status, SuggestionStatus::Approved);

        let cfg = running_config_all_fields(&app.rd).await;
        assert_eq!(overlay_rule_count(&cfg, "route", "x.com", "noop"), 1);

        app.policy.reconcile_overlays_for_test().await.unwrap();

        let cfg = running_config_all_fields(&app.rd).await;
        assert_eq!(overlay_rule_count(&cfg, "route", "x.com", "noop"), 1);
    }

    #[tokio::test]
    async fn test_reconcile_reapplies_overlay_after_restart() {
        let app = crate::App::new().await.unwrap();
        let cfg = overlay_test_config();
        app.rd.start(cfg.clone()).await.unwrap();

        app.policy
            .add_pending_suggestion_for_test("x.com", "proxy", "local", "noop")
            .await
            .unwrap();
        let suggestion_id = app.policy.suggestions().await.unwrap()[0].id.clone();
        app.policy.approve_suggestion(&suggestion_id).await.unwrap();

        app.rd.stop().await.unwrap();
        app.rd.start(cfg).await.unwrap();

        let restarted_cfg = running_config_all_fields(&app.rd).await;
        assert_eq!(
            overlay_rule_count(&restarted_cfg, "route", "x.com", "noop"),
            0
        );

        app.policy.reconcile_overlays_for_test().await.unwrap();

        let reconciled_cfg = running_config_all_fields(&app.rd).await;
        assert_eq!(
            overlay_rule_count(&reconciled_cfg, "route", "x.com", "noop"),
            1
        );
    }
}
