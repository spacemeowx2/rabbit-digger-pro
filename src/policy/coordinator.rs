use std::{
    collections::{HashMap, HashSet, VecDeque},
    time::SystemTime,
};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const POLICY_SCHEMA_VERSION: u32 = 1;
const ACTION_LOG_LIMIT: usize = 64;
const PROMOTION_THRESHOLD: u32 = 2;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SuggestionStatus {
    Pending,
    Approved,
    Rejected,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicySuggestion {
    pub id: String,
    pub created_at: u64,
    pub updated_at: u64,
    pub domain: String,
    pub select_net: String,
    pub current_target: String,
    pub suggested_target: String,
    pub evidence_count: u32,
    pub status: SuggestionStatus,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PolicyActionKind {
    ObservationDropped,
    ObservationLagged,
    CooldownSkipped,
    ProbeFailed,
    TemporarySwitch,
    OverlayApplied,
    OverlayFailed,
    SuggestionCreated,
    SuggestionApproved,
    SuggestionRejected,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PolicyActionOutcome {
    Succeeded,
    Failed,
    Skipped,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyActionRecord {
    pub id: String,
    pub at: u64,
    pub kind: PolicyActionKind,
    pub outcome: PolicyActionOutcome,
    pub domain: Option<String>,
    pub select_net: Option<String>,
    pub current_target: Option<String>,
    pub candidate: Option<String>,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CooldownState {
    pub key: String,
    pub until: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct PolicyStateView {
    pub version: u32,
    pub running: bool,
    pub pending_suggestions: usize,
    pub approved_suggestions: usize,
    pub recent_action_count: usize,
    pub cooldowns: Vec<CooldownState>,
}

#[derive(Debug, Clone)]
pub struct SelectCandidate {
    pub name: String,
}

pub struct PolicyCoordinator {
    suggestions: Vec<PolicySuggestion>,
    actions: VecDeque<PolicyActionRecord>,
    evidence: HashMap<String, u32>,
    cooldowns: HashMap<String, u64>,
}

impl PolicyCoordinator {
    pub fn from_persisted(
        suggestions: Vec<PolicySuggestion>,
        actions: Vec<PolicyActionRecord>,
    ) -> Self {
        Self {
            suggestions,
            actions: actions.into_iter().collect(),
            evidence: HashMap::new(),
            cooldowns: HashMap::new(),
        }
    }

    pub fn snapshot(&self, running: bool) -> PolicyStateView {
        let now = now_ts();
        PolicyStateView {
            version: POLICY_SCHEMA_VERSION,
            running,
            pending_suggestions: self
                .suggestions
                .iter()
                .filter(|suggestion| suggestion.status == SuggestionStatus::Pending)
                .count(),
            approved_suggestions: self
                .suggestions
                .iter()
                .filter(|suggestion| suggestion.status == SuggestionStatus::Approved)
                .count(),
            recent_action_count: self.actions.len(),
            cooldowns: self
                .cooldowns
                .iter()
                .filter(|(_, until)| **until > now)
                .map(|(key, until)| CooldownState {
                    key: key.clone(),
                    until: *until,
                })
                .collect(),
        }
    }

    pub fn actions(&self) -> Vec<PolicyActionRecord> {
        self.actions.iter().cloned().collect()
    }

    pub fn suggestions(&self) -> Vec<PolicySuggestion> {
        self.suggestions.clone()
    }

    pub fn approved_target_for(&self, domain: &str, select_net: &str) -> Option<String> {
        self.suggestions
            .iter()
            .rev()
            .find(|suggestion| {
                suggestion.domain == domain
                    && suggestion.select_net == select_net
                    && suggestion.status == SuggestionStatus::Approved
            })
            .map(|suggestion| suggestion.suggested_target.clone())
    }

    pub fn latest_approved_suggestions(&self) -> Vec<PolicySuggestion> {
        let mut seen = HashSet::new();
        let mut approved = Vec::new();

        for suggestion in self.suggestions.iter().rev() {
            if suggestion.status != SuggestionStatus::Approved {
                continue;
            }
            let key = format!("{}|{}", suggestion.domain, suggestion.select_net);
            if seen.insert(key) {
                approved.push(suggestion.clone());
            }
        }

        approved.reverse();
        approved
    }

    pub fn in_cooldown(&mut self, key: &str, now: u64) -> bool {
        self.cooldowns.retain(|_, until| *until > now);
        self.cooldowns
            .get(key)
            .map(|until| *until > now)
            .unwrap_or(false)
    }

    pub fn set_cooldown(&mut self, key: &str, until: u64) {
        self.cooldowns.insert(key.to_string(), until);
    }

    pub fn record_observation_dropped(&mut self, detail: String) {
        self.push_action(PolicyActionRecord {
            id: Uuid::new_v4().to_string(),
            at: now_ts(),
            kind: PolicyActionKind::ObservationDropped,
            outcome: PolicyActionOutcome::Skipped,
            domain: None,
            select_net: None,
            current_target: None,
            candidate: None,
            detail,
        });
    }

    pub fn record_observation_lag(&mut self, skipped: u64) {
        self.push_action(PolicyActionRecord {
            id: Uuid::new_v4().to_string(),
            at: now_ts(),
            kind: PolicyActionKind::ObservationLagged,
            outcome: PolicyActionOutcome::Skipped,
            domain: None,
            select_net: None,
            current_target: None,
            candidate: None,
            detail: format!("lagged observation stream, skipped {skipped} events"),
        });
    }

    pub fn record_cooldown_skip(
        &mut self,
        domain: String,
        select_net: String,
        current_target: String,
    ) {
        self.push_action(PolicyActionRecord {
            id: Uuid::new_v4().to_string(),
            at: now_ts(),
            kind: PolicyActionKind::CooldownSkipped,
            outcome: PolicyActionOutcome::Skipped,
            domain: Some(domain),
            select_net: Some(select_net),
            current_target: Some(current_target),
            candidate: None,
            detail: "cooldown is active, skip duplicate recovery".to_string(),
        });
    }

    pub fn record_probe_failure(
        &mut self,
        domain: String,
        select_net: String,
        current_target: String,
        detail: String,
    ) {
        self.push_action(PolicyActionRecord {
            id: Uuid::new_v4().to_string(),
            at: now_ts(),
            kind: PolicyActionKind::ProbeFailed,
            outcome: PolicyActionOutcome::Failed,
            domain: Some(domain),
            select_net: Some(select_net),
            current_target: Some(current_target),
            candidate: None,
            detail,
        });
    }

    pub fn record_temporary_switch(
        &mut self,
        domain: String,
        select_net: String,
        current_target: String,
        suggested_target: String,
    ) {
        let action_detail = format!(
            "temporary recovery switched {select_net} from {current_target} to {suggested_target}"
        );
        self.push_action(PolicyActionRecord {
            id: Uuid::new_v4().to_string(),
            at: now_ts(),
            kind: PolicyActionKind::TemporarySwitch,
            outcome: PolicyActionOutcome::Succeeded,
            domain: Some(domain.clone()),
            select_net: Some(select_net.clone()),
            current_target: Some(current_target.clone()),
            candidate: Some(suggested_target.clone()),
            detail: action_detail,
        });

        let evidence_key = evidence_key(&domain, &select_net, &suggested_target);
        let evidence_count = {
            let value = self.evidence.entry(evidence_key).or_insert(0);
            *value += 1;
            *value
        };
        if evidence_count < PROMOTION_THRESHOLD {
            return;
        }
        if self.suggestions.iter().any(|suggestion| {
            suggestion.domain == domain
                && suggestion.select_net == select_net
                && suggestion.suggested_target == suggested_target
        }) {
            return;
        }

        let suggestion = PolicySuggestion {
            id: Uuid::new_v4().to_string(),
            created_at: now_ts(),
            updated_at: now_ts(),
            domain: domain.clone(),
            select_net: select_net.clone(),
            current_target,
            suggested_target: suggested_target.clone(),
            evidence_count,
            status: SuggestionStatus::Pending,
            reason: format!(
                "temporary recovery for {domain} succeeded {evidence_count} times via {suggested_target}"
            ),
        };
        self.suggestions.push(suggestion.clone());
        self.push_action(PolicyActionRecord {
            id: Uuid::new_v4().to_string(),
            at: now_ts(),
            kind: PolicyActionKind::SuggestionCreated,
            outcome: PolicyActionOutcome::Succeeded,
            domain: Some(domain),
            select_net: Some(select_net),
            current_target: Some(suggestion.current_target.clone()),
            candidate: Some(suggested_target),
            detail: format!("created pending suggestion {}", suggestion.id),
        });
    }

    pub fn record_overlay_applied(
        &mut self,
        domain: String,
        select_net: String,
        current_target: String,
        suggested_target: String,
        rule_nets: &[String],
    ) {
        self.push_action(PolicyActionRecord {
            id: Uuid::new_v4().to_string(),
            at: now_ts(),
            kind: PolicyActionKind::OverlayApplied,
            outcome: PolicyActionOutcome::Succeeded,
            domain: Some(domain),
            select_net: Some(select_net),
            current_target: Some(current_target),
            candidate: Some(suggested_target),
            detail: format!(
                "applied runtime overlay to rule nets [{}]",
                rule_nets.join(", ")
            ),
        });
    }

    pub fn record_overlay_failed(
        &mut self,
        domain: String,
        select_net: String,
        current_target: String,
        suggested_target: String,
        detail: String,
    ) {
        self.push_action(PolicyActionRecord {
            id: Uuid::new_v4().to_string(),
            at: now_ts(),
            kind: PolicyActionKind::OverlayFailed,
            outcome: PolicyActionOutcome::Failed,
            domain: Some(domain),
            select_net: Some(select_net),
            current_target: Some(current_target),
            candidate: Some(suggested_target),
            detail,
        });
    }

    pub fn approve_suggestion(&mut self, id: &str) -> Option<PolicySuggestion> {
        let suggestion = self
            .suggestions
            .iter_mut()
            .find(|suggestion| suggestion.id == id)?;
        suggestion.status = SuggestionStatus::Approved;
        suggestion.updated_at = now_ts();
        let cloned = suggestion.clone();
        self.push_action(PolicyActionRecord {
            id: Uuid::new_v4().to_string(),
            at: now_ts(),
            kind: PolicyActionKind::SuggestionApproved,
            outcome: PolicyActionOutcome::Succeeded,
            domain: Some(cloned.domain.clone()),
            select_net: Some(cloned.select_net.clone()),
            current_target: Some(cloned.current_target.clone()),
            candidate: Some(cloned.suggested_target.clone()),
            detail: format!("approved suggestion {}", cloned.id),
        });
        Some(cloned)
    }

    pub fn reject_suggestion(&mut self, id: &str) -> Option<PolicySuggestion> {
        let suggestion = self
            .suggestions
            .iter_mut()
            .find(|suggestion| suggestion.id == id)?;
        suggestion.status = SuggestionStatus::Rejected;
        suggestion.updated_at = now_ts();
        let cloned = suggestion.clone();
        self.push_action(PolicyActionRecord {
            id: Uuid::new_v4().to_string(),
            at: now_ts(),
            kind: PolicyActionKind::SuggestionRejected,
            outcome: PolicyActionOutcome::Succeeded,
            domain: Some(cloned.domain.clone()),
            select_net: Some(cloned.select_net.clone()),
            current_target: Some(cloned.current_target.clone()),
            candidate: Some(cloned.suggested_target.clone()),
            detail: format!("rejected suggestion {}", cloned.id),
        });
        Some(cloned)
    }

    #[cfg(test)]
    pub fn insert_pending_suggestion(
        &mut self,
        domain: String,
        select_net: String,
        current_target: String,
        suggested_target: String,
    ) {
        self.suggestions.push(PolicySuggestion {
            id: Uuid::new_v4().to_string(),
            created_at: now_ts(),
            updated_at: now_ts(),
            domain,
            select_net,
            current_target,
            suggested_target,
            evidence_count: PROMOTION_THRESHOLD,
            status: SuggestionStatus::Pending,
            reason: "test suggestion".to_string(),
        });
    }

    fn push_action(&mut self, action: PolicyActionRecord) {
        self.actions.push_front(action);
        while self.actions.len() > ACTION_LOG_LIMIT {
            self.actions.pop_back();
        }
    }
}

fn evidence_key(domain: &str, select_net: &str, suggested_target: &str) -> String {
    format!("{domain}|{select_net}|{suggested_target}")
}

fn now_ts() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_temporary_switch_creates_suggestion_after_threshold() {
        let mut coordinator = PolicyCoordinator::from_persisted(vec![], vec![]);

        coordinator.record_temporary_switch(
            "x.com".to_string(),
            "proxy".to_string(),
            "node-a".to_string(),
            "node-b".to_string(),
        );
        assert_eq!(coordinator.suggestions.len(), 0);

        coordinator.record_temporary_switch(
            "x.com".to_string(),
            "proxy".to_string(),
            "node-a".to_string(),
            "node-b".to_string(),
        );
        assert_eq!(coordinator.suggestions.len(), 1);
        assert_eq!(coordinator.suggestions[0].status, SuggestionStatus::Pending);
    }

    #[test]
    fn test_approve_and_reject_suggestion() {
        let mut coordinator = PolicyCoordinator::from_persisted(vec![], vec![]);
        coordinator.insert_pending_suggestion(
            "x.com".to_string(),
            "proxy".to_string(),
            "node-a".to_string(),
            "node-b".to_string(),
        );
        let suggestion_id = coordinator.suggestions[0].id.clone();

        let approved = coordinator.approve_suggestion(&suggestion_id).unwrap();
        assert_eq!(approved.status, SuggestionStatus::Approved);

        let rejected = coordinator.reject_suggestion(&suggestion_id).unwrap();
        assert_eq!(rejected.status, SuggestionStatus::Rejected);
    }

    #[test]
    fn test_cooldown_and_approved_target_lookup() {
        let mut coordinator = PolicyCoordinator::from_persisted(vec![], vec![]);
        coordinator.set_cooldown("x.com@proxy", now_ts() + 60);
        assert!(coordinator.in_cooldown("x.com@proxy", now_ts()));

        coordinator.insert_pending_suggestion(
            "x.com".to_string(),
            "proxy".to_string(),
            "node-a".to_string(),
            "node-b".to_string(),
        );
        let suggestion_id = coordinator.suggestions[0].id.clone();
        coordinator.approve_suggestion(&suggestion_id);

        assert_eq!(
            coordinator.approved_target_for("x.com", "proxy"),
            Some("node-b".to_string())
        );
    }
}
