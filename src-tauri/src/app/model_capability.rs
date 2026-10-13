//! Local observations are not interchangeable with native Codex certification.
use serde::{Deserialize, Serialize};

pub const SIDECAR_IDENTITY: &str = "codex-usb-model-capabilities-v1";
pub const EVIDENCE_TTL_SECONDS: u64 = 6 * 60 * 60;

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityState {
    #[default]
    Unknown,
    Verified,
    Disabled,
    Quarantined,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CapabilityChecks {
    pub text: CapabilityState,
    pub single_tool: CapabilityState,
    pub tool_result: CapabilityState,
    pub multiple_tools: CapabilityState,
    pub tool_error: CapabilityState,
    pub stream: CapabilityState,
    pub multi_turn: CapabilityState,
    pub shell: CapabilityState,
    pub skills: CapabilityState,
    pub native_catalog: CapabilityState,
}

impl CapabilityChecks {
    fn values(&self) -> [CapabilityState; 10] {
        [
            self.text,
            self.single_tool,
            self.tool_result,
            self.multiple_tools,
            self.tool_error,
            self.stream,
            self.multi_turn,
            self.shell,
            self.skills,
            self.native_catalog,
        ]
    }

    pub fn has_failure(&self) -> bool {
        self.values().contains(&CapabilityState::Quarantined)
    }

    pub fn all_verified(&self) -> bool {
        self.values()
            .iter()
            .all(|state| *state == CapabilityState::Verified)
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceScope {
    #[default]
    Discovery,
    Gateway,
    CachedGateway,
    Mock,
    CodexRuntime,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ModelState {
    #[default]
    Unverified,
    Quarantined,
    Ready,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CapabilityContext {
    pub binding_fingerprint: String,
    pub codex_version: Option<String>,
    #[serde(default)]
    pub native_runtime_version: Option<String>,
    pub adapter_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ModelCapabilityReport {
    pub model_id: String,
    pub owned_by: Option<String>,
    pub state: ModelState,
    pub checks: CapabilityChecks,
    pub evidence_scope: EvidenceScope,
    pub checked_at: Option<u64>,
    pub error_code: Option<String>,
}

/// In-process native observations, deliberately not Deserialize/Serialize.
/// Neither a sidecar nor a Tauri caller can supply this as a certificate.
/// The native-runtime producer is a separate integration boundary from the
/// gateway probe; it requires the real Codex/Skills/Shell acceptance run.
#[derive(Debug, Clone)]
pub struct NativeRuntimeEvidence {
    pub(crate) context: CapabilityContext,
    pub(crate) model_id: String,
    pub(crate) owned_by: Option<String>,
    pub(crate) checked_at: u64,
    pub(crate) checks: CapabilityChecks,
}

pub fn evidence_is_fresh(observed: u64, now: u64) -> bool {
    observed <= now && now - observed <= EVIDENCE_TTL_SECONDS
}

impl ModelCapabilityReport {
    pub fn discovered(model_id: String, owned_by: Option<String>) -> Self {
        Self {
            model_id,
            owned_by,
            state: ModelState::Unverified,
            checks: CapabilityChecks::default(),
            evidence_scope: EvidenceScope::Discovery,
            checked_at: None,
            error_code: None,
        }
    }

    pub fn evaluate(
        &mut self,
        context: &CapabilityContext,
        native: Option<&NativeRuntimeEvidence>,
        now: u64,
    ) {
        self.checks.shell = CapabilityState::Unknown;
        self.checks.skills = CapabilityState::Unknown;
        self.checks.native_catalog = CapabilityState::Unknown;
        if self.evidence_scope == EvidenceScope::CodexRuntime {
            // Re-evaluation still requires the same live proof below. Disk
            // loads are separately stripped by discard_cached_authority.
            self.evidence_scope = EvidenceScope::Gateway;
        }
        if self.evidence_scope == EvidenceScope::Gateway && self.error_code.is_none() {
            if let Some(proof) = native.filter(|proof| {
                proof.context == *context
                    && proof.model_id == self.model_id
                    && proof.owned_by == self.owned_by
                    && context
                        .codex_version
                        .as_ref()
                        .is_some_and(|v| !v.is_empty())
                    && context
                        .native_runtime_version
                        .as_ref()
                        .is_some_and(|v| !v.is_empty())
                    && context
                        .adapter_version
                        .as_ref()
                        .is_some_and(|v| !v.is_empty())
                    && evidence_is_fresh(proof.checked_at, now)
                    && self.checked_at.is_some_and(|at| evidence_is_fresh(at, now))
                    && proof.checks.all_verified()
            }) {
                self.checks.shell = proof.checks.shell;
                self.checks.skills = proof.checks.skills;
                self.checks.native_catalog = proof.checks.native_catalog;
                if self.checks.all_verified() {
                    self.evidence_scope = EvidenceScope::CodexRuntime;
                    self.state = ModelState::Ready;
                    return;
                }
            }
        }
        self.state = if self.checks.has_failure() {
            ModelState::Quarantined
        } else {
            ModelState::Unverified
        };
    }

    pub fn discard_cached_authority(&mut self) {
        // A file can retain observations for scheduling/diagnosis only. Do
        // not import a stored Ready or native-runtime claim into live state.
        if matches!(
            self.evidence_scope,
            EvidenceScope::CodexRuntime | EvidenceScope::Discovery
        ) {
            self.checks = CapabilityChecks::default();
            self.evidence_scope = EvidenceScope::Discovery;
        } else if self.evidence_scope == EvidenceScope::Gateway {
            self.evidence_scope = EvidenceScope::CachedGateway;
        }
        self.checks.shell = CapabilityState::Unknown;
        self.checks.skills = CapabilityState::Unknown;
        self.checks.native_catalog = CapabilityState::Unknown;
        self.state = if self.checks.has_failure() {
            ModelState::Quarantined
        } else {
            ModelState::Unverified
        };
    }
}

/// Recompute eligibility instead of trusting a serialized state or model name.
pub fn select_ready_default(
    models: &[ModelCapabilityReport],
    context: &CapabilityContext,
    native: &[NativeRuntimeEvidence],
    now: u64,
) -> Option<String> {
    models.iter().find_map(|model| {
        // Codex selects by raw ID, so two owners (or duplicate rows) sharing
        // that ID cannot be disambiguated by a supposedly ready default.
        if models
            .iter()
            .filter(|other| other.model_id == model.model_id)
            .count()
            != 1
        {
            return None;
        }
        let mut matching = native
            .iter()
            .filter(|e| e.model_id == model.model_id && e.owned_by == model.owned_by);
        let proof = matching.next()?;
        if matching.next().is_some() {
            return None;
        }
        let mut candidate = model.clone();
        candidate.evaluate(context, Some(proof), now);
        (candidate.state == ModelState::Ready).then_some(candidate.model_id)
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CapabilitySidecar {
    pub schema_version: u32,
    pub identity: String,
    pub context: CapabilityContext,
    pub fetched_at: u64,
    pub models: Vec<ModelCapabilityReport>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_setup: Option<AutoSetupCheckpoint>,
}

/// Bounded scheduler state for resuming a one-click operation. It never
/// contains a credential or an authority-bearing model proof.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AutoSetupCheckpoint {
    pub operation_id: String,
    pub phase: AutoSetupPhase,
    pub service_revision: String,
    pub authorization_scope_sha256: String,
    pub started_at_utc: String,
    pub deadline_utc: String,
    pub monotonic_clock_source: String,
    pub boot_identity_source: String,
    pub boot_identity: String,
    pub started_monotonic_ms: u64,
    pub deadline_monotonic_ms: u64,
    pub candidates: Vec<AutoSetupCandidate>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AutoSetupPhase {
    GatewayVerification,
    NativeVerification,
    ApplyPending,
    Complete,
    Incomplete,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AutoSetupCandidate {
    pub model_id: String,
    pub status: AutoSetupCandidateStatus,
    pub result_code: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AutoSetupCandidateStatus {
    Untouched,
    Inflight,
    GatewayComplete,
    NativeComplete,
    Failed,
    Uncertain,
}

#[cfg(test)]
mod tests {
    use super::*;
    fn context() -> CapabilityContext {
        CapabilityContext {
            binding_fingerprint: "fixture-binding".into(),
            codex_version: Some("fixture-codex".into()),
            native_runtime_version: Some("fixture-runtime".into()),
            adapter_version: Some("fixture-adapter".into()),
        }
    }
    fn checks() -> CapabilityChecks {
        let v = CapabilityState::Verified;
        CapabilityChecks {
            text: v,
            single_tool: v,
            tool_result: v,
            multiple_tools: v,
            tool_error: v,
            stream: v,
            multi_turn: v,
            shell: v,
            skills: v,
            native_catalog: v,
        }
    }
    #[test]
    fn discovery_mock_and_cache_never_grant_ready() {
        for source in [
            EvidenceScope::Discovery,
            EvidenceScope::Mock,
            EvidenceScope::CachedGateway,
        ] {
            let mut m = ModelCapabilityReport::discovered("arbitrary/model".into(), None);
            m.evidence_scope = source;
            m.checks = checks();
            m.state = ModelState::Ready;
            m.evaluate(&context(), None, 100);
            assert_eq!(m.state, ModelState::Unverified);
            assert_eq!(m.checks.shell, CapabilityState::Unknown);
        }
    }
    #[test]
    fn ready_requires_current_in_process_native_evidence() {
        let mut m = ModelCapabilityReport::discovered("arbitrary/model".into(), None);
        m.evidence_scope = EvidenceScope::Gateway;
        m.checked_at = Some(100);
        m.checks = checks();
        let proof = NativeRuntimeEvidence {
            context: context(),
            model_id: m.model_id.clone(),
            owned_by: None,
            checked_at: 100,
            checks: checks(),
        };
        assert_eq!(
            select_ready_default(&[m.clone()], &context(), std::slice::from_ref(&proof), 101),
            Some("arbitrary/model".into())
        );
        assert_eq!(
            select_ready_default(&[m.clone()], &context(), &[], 101),
            None
        );
        let mut changed = context();
        changed.binding_fingerprint = "other".into();
        assert_eq!(
            select_ready_default(&[m.clone()], &changed, std::slice::from_ref(&proof), 101),
            None
        );
        assert_eq!(
            select_ready_default(&[m.clone()], &context(), &[proof], 99),
            None
        );
        m.discard_cached_authority();
        assert_ne!(m.state, ModelState::Ready);
    }
    #[test]
    fn defaults_reject_duplicate_ids_owners_and_duplicate_live_proofs() {
        let mut model =
            ModelCapabilityReport::discovered("arbitrary/model".into(), Some("owner-a".into()));
        model.evidence_scope = EvidenceScope::Gateway;
        model.checked_at = Some(100);
        model.checks = checks();
        let proof = NativeRuntimeEvidence {
            context: context(),
            model_id: model.model_id.clone(),
            owned_by: model.owned_by.clone(),
            checked_at: 100,
            checks: checks(),
        };
        let mut other_owner = model.clone();
        other_owner.owned_by = Some("owner-b".into());
        assert_eq!(
            select_ready_default(
                &[model.clone(), other_owner],
                &context(),
                std::slice::from_ref(&proof),
                101
            ),
            None
        );
        assert_eq!(
            select_ready_default(
                &[model.clone(), model.clone()],
                &context(),
                std::slice::from_ref(&proof),
                101
            ),
            None
        );
        assert_eq!(
            select_ready_default(
                &[model.clone()],
                &context(),
                &[proof.clone(), proof.clone()],
                101
            ),
            None
        );
        assert_eq!(
            select_ready_default(&[model], &context(), &[proof], 101),
            Some("arbitrary/model".into())
        );
    }

    #[test]
    fn a_failed_model_does_not_quarantine_another() {
        let mut failed = ModelCapabilityReport::discovered("one".into(), None);
        failed.checks.tool_result = CapabilityState::Quarantined;
        failed.evaluate(&context(), None, 1);
        let mut other = ModelCapabilityReport::discovered("two".into(), None);
        other.evaluate(&context(), None, 1);
        assert_eq!(failed.state, ModelState::Quarantined);
        assert_eq!(other.state, ModelState::Unverified);
    }
    #[test]
    fn rejects_truthy_and_unknown_capability_values() {
        for value in ["true", "1", "\"ready\"", "\"false\""] {
            assert!(serde_json::from_str::<CapabilityState>(value).is_err());
        }
    }

    #[test]
    fn reevaluation_is_stable_only_while_native_proof_remains_valid() {
        let mut m = ModelCapabilityReport::discovered("arbitrary/model".into(), None);
        m.evidence_scope = EvidenceScope::Gateway;
        m.checked_at = Some(100);
        m.checks = checks();
        let proof = NativeRuntimeEvidence {
            context: context(),
            model_id: m.model_id.clone(),
            owned_by: None,
            checked_at: 100,
            checks: checks(),
        };
        m.evaluate(&context(), Some(&proof), 101);
        assert_eq!(m.state, ModelState::Ready);
        m.evaluate(&context(), Some(&proof), 102);
        assert_eq!(m.state, ModelState::Ready);
        m.error_code = Some("fixture-error".into());
        m.evaluate(&context(), Some(&proof), 103);
        assert_ne!(m.state, ModelState::Ready);
        m.error_code = None;
        m.evaluate(&context(), None, 104);
        assert_ne!(m.state, ModelState::Ready);
    }
}
