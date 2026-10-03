//! Deterministic storage-boundary tests for `model_catalog`.
//!
//! These tests deliberately call only path-parameterized helpers.  They do
//! not change `CODEX_HOME`, consult USB authorization, start a probe, or use
//! the process-wide reload marker.

use super::{
    owned_json, read_sidecar, remove_managed_config, unix_seconds, CapabilityContext,
    CapabilitySidecar, ModelCapabilityReport, ModelState, BODY_LIMIT, CONFIG_BEGIN, CONFIG_END,
    SIDECAR_IDENTITY,
};
use crate::app::model_capability::{
    evidence_is_fresh, CapabilityChecks, CapabilityState, EvidenceScope,
};
use serde_json::json;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(1);

struct TempDir {
    path: PathBuf,
}

impl TempDir {
    fn new(label: &str) -> Self {
        let root = std::env::temp_dir();

        loop {
            let id = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = root.join(format!(
                "codex-usb-model-catalog-storage-{label}-{}-{id}",
                std::process::id()
            ));
            match fs::create_dir(&path) {
                Ok(()) => return Self { path },
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("create unique storage-test directory: {error}"),
            }
        }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn synthetic_context(binding: &str, codex_version: Option<&str>) -> CapabilityContext {
    CapabilityContext {
        binding_fingerprint: binding.to_owned(),
        codex_version: codex_version.map(str::to_owned),
        native_runtime_version: None,
        adapter_version: Some("synthetic-adapter-1".into()),
    }
}

fn verified_checks() -> CapabilityChecks {
    let verified = CapabilityState::Verified;
    CapabilityChecks {
        text: verified,
        single_tool: verified,
        tool_result: verified,
        multiple_tools: verified,
        tool_error: verified,
        stream: verified,
        multi_turn: verified,
        shell: verified,
        skills: verified,
        native_catalog: verified,
    }
}

fn cached_ready_model(model_id: &str, scope: EvidenceScope) -> ModelCapabilityReport {
    let mut model =
        ModelCapabilityReport::discovered(model_id.to_owned(), Some("synthetic-owner".into()));
    model.state = ModelState::Ready;
    model.checks = verified_checks();
    model.evidence_scope = scope;
    model.checked_at = Some(1);
    model.error_code = Some("synthetic-error".into());
    model
}

fn write_sidecar(
    path: &Path,
    context: &CapabilityContext,
    fetched_at: u64,
    models: Vec<ModelCapabilityReport>,
) -> Vec<u8> {
    let sidecar = CapabilitySidecar {
        schema_version: 1,
        identity: SIDECAR_IDENTITY.into(),
        context: context.clone(),
        fetched_at,
        models,
        auto_setup: None,
    };
    let bytes = serde_json::to_vec(&sidecar).expect("serialize synthetic sidecar");
    fs::write(path, &bytes).expect("write synthetic sidecar");
    bytes
}

#[test]
fn sidecar_ownership_requires_the_exact_manager_identity() {
    let dir = TempDir::new("ownership");
    let path = dir.path().join("capabilities.json");

    let cases = [
        (
            serde_json::to_vec(&json!({"identity": SIDECAR_IDENTITY})).unwrap(),
            true,
        ),
        (
            serde_json::to_vec(&json!({"identity": "synthetic-foreign-owner"})).unwrap(),
            false,
        ),
        (b"{".to_vec(), false),
        (serde_json::to_vec(&json!([])).unwrap(), false),
    ];

    for (bytes, expected) in cases {
        fs::write(&path, &bytes).unwrap();
        assert_eq!(owned_json(&path, SIDECAR_IDENTITY), expected);
    }
}

#[test]
fn non_owned_sidecar_is_rejected_without_overwriting_the_file() {
    let dir = TempDir::new("foreign");
    let path = dir.path().join("capabilities.json");
    let context = synthetic_context("synthetic-binding", Some("synthetic-codex"));
    let bytes = serde_json::to_vec(&json!({
        "schemaVersion": 1,
        "identity": "synthetic-foreign-owner",
        "context": context,
        "fetchedAt": 1,
        "models": []
    }))
    .unwrap();
    fs::write(&path, &bytes).unwrap();

    assert!(read_sidecar(
        &path,
        &synthetic_context("synthetic-binding", Some("synthetic-codex"))
    )
    .is_err());
    assert_eq!(fs::read(&path).unwrap(), bytes);
}

#[test]
fn corrupt_and_schema_invalid_sidecars_are_rejected_and_preserved() {
    let dir = TempDir::new("schema");
    let path = dir.path().join("capabilities.json");
    let context = synthetic_context("synthetic-binding", Some("synthetic-codex"));
    let base = serde_json::to_value(CapabilitySidecar {
        schema_version: 1,
        identity: SIDECAR_IDENTITY.into(),
        context: context.clone(),
        fetched_at: 1,
        models: Vec::new(),
        auto_setup: None,
    })
    .unwrap();

    let mut wrong_version = base.clone();
    // Version 2 is now supported; future schemas must still be rejected.
    wrong_version["schemaVersion"] = json!(99);
    let mut unknown_field = base.clone();
    unknown_field["syntheticUnknownField"] = json!(true);
    let cases: [(&str, Vec<u8>); 3] = [
        ("corrupt-json", b"{not-json".to_vec()),
        (
            "wrong-schema-version",
            serde_json::to_vec(&wrong_version).unwrap(),
        ),
        (
            "unknown-schema-field",
            serde_json::to_vec(&unknown_field).unwrap(),
        ),
    ];

    for (label, bytes) in cases {
        fs::write(&path, &bytes).unwrap();
        assert!(
            read_sidecar(&path, &context).is_err(),
            "sidecar case should be rejected: {label}"
        );
        assert_eq!(fs::read(&path).unwrap(), bytes, "case: {label}");
    }
}

#[test]
fn sidecar_body_over_limit_is_rejected_and_preserved() {
    let dir = TempDir::new("oversize-body");
    let path = dir.path().join("capabilities.json");
    let context = synthetic_context("synthetic-binding", Some("synthetic-codex"));
    let bytes = vec![b'x'; BODY_LIMIT + 1];
    fs::write(&path, &bytes).unwrap();

    assert!(read_sidecar(&path, &context).is_err());
    assert_eq!(fs::read(&path).unwrap(), bytes);
}

#[test]
fn sidecar_model_count_over_limit_is_rejected() {
    let dir = TempDir::new("oversize-count");
    let path = dir.path().join("capabilities.json");
    let context = synthetic_context("synthetic-binding", Some("synthetic-codex"));
    let models = (0..=512)
        .map(|index| ModelCapabilityReport::discovered(format!("synthetic-model-{index}"), None))
        .collect();
    let bytes = write_sidecar(&path, &context, 1, models);
    assert!(bytes.len() <= BODY_LIMIT);
    assert!(read_sidecar(&path, &context).is_err());
    assert_eq!(fs::read(&path).unwrap(), bytes);
}

#[test]
fn sidecar_context_mismatch_by_binding_or_codex_version_is_not_reused() {
    let dir = TempDir::new("context");
    let path = dir.path().join("capabilities.json");
    let stored_context = synthetic_context("synthetic-binding-a", Some("synthetic-codex-a"));
    let bytes = write_sidecar(
        &path,
        &stored_context,
        1,
        vec![ModelCapabilityReport::discovered(
            "synthetic/model".into(),
            Some("synthetic-owner".into()),
        )],
    );

    for (label, requested_context) in [
        (
            "binding",
            synthetic_context("synthetic-binding-b", Some("synthetic-codex-a")),
        ),
        (
            "codex-version",
            synthetic_context("synthetic-binding-a", Some("synthetic-codex-b")),
        ),
    ] {
        assert!(
            read_sidecar(&path, &requested_context).unwrap().is_none(),
            "mismatched {label} must not reuse the sidecar"
        );
    }

    let loaded = read_sidecar(&path, &stored_context).unwrap().unwrap();
    assert_eq!(loaded.context, stored_context);
    assert_eq!(fs::read(&path).unwrap(), bytes);
}

#[test]
fn reading_cached_sidecar_discards_serialized_authority() {
    let dir = TempDir::new("authority");
    let path = dir.path().join("capabilities.json");
    let context = synthetic_context("synthetic-binding", Some("synthetic-codex"));
    write_sidecar(
        &path,
        &context,
        1,
        vec![
            cached_ready_model("synthetic-native", EvidenceScope::CodexRuntime),
            cached_ready_model("synthetic-gateway", EvidenceScope::Gateway),
        ],
    );

    let loaded = read_sidecar(&path, &context).unwrap().unwrap();
    let native = loaded
        .models
        .iter()
        .find(|model| model.model_id == "synthetic-native")
        .unwrap();
    assert_eq!(native.state, ModelState::Unverified);
    assert_eq!(native.evidence_scope, EvidenceScope::Discovery);
    assert_eq!(native.checks, CapabilityChecks::default());
    assert_eq!(native.error_code, None);

    let gateway = loaded
        .models
        .iter()
        .find(|model| model.model_id == "synthetic-gateway")
        .unwrap();
    assert_eq!(gateway.state, ModelState::Unverified);
    assert_eq!(gateway.evidence_scope, EvidenceScope::CachedGateway);
    assert_eq!(gateway.checks.shell, CapabilityState::Unknown);
    assert_eq!(gateway.checks.skills, CapabilityState::Unknown);
    assert_eq!(gateway.checks.native_catalog, CapabilityState::Unknown);
    assert_eq!(gateway.error_code, None);
}

#[test]
fn future_sidecar_timestamps_cannot_be_used_as_fresh_evidence() {
    let dir = TempDir::new("future");
    let path = dir.path().join("capabilities.json");
    let context = synthetic_context("synthetic-binding", Some("synthetic-codex"));
    let mut model = cached_ready_model("synthetic-future", EvidenceScope::Gateway);
    model.checked_at = Some(u64::MAX);
    write_sidecar(&path, &context, u64::MAX, vec![model]);

    let loaded = read_sidecar(&path, &context).unwrap().unwrap();
    assert_eq!(loaded.fetched_at, u64::MAX);
    assert!(!evidence_is_fresh(loaded.fetched_at, unix_seconds()));
    assert_eq!(loaded.models[0].checked_at, None);
    assert_eq!(loaded.models[0].state, ModelState::Unverified);
    assert_eq!(
        loaded.models[0].evidence_scope,
        EvidenceScope::CachedGateway
    );
}

#[test]
fn managed_config_removal_preserves_user_configuration_outside_owned_block() {
    let text = format!(
        "model = \"synthetic-user-model\"\r\n\
[profiles.synthetic]\r\n\
provider = \"synthetic-user-provider\"\r\n\
{CONFIG_BEGIN}\r\n\
# generated managed comment\r\n\
model_catalog_json = \"synthetic-owned-catalog.json\"\r\n\
{CONFIG_END}\r\n\
[other]\r\n\
user_setting = \"preserve-me\"\r\n"
    );
    let expected = "model = \"synthetic-user-model\"\r\n\
[profiles.synthetic]\r\n\
provider = \"synthetic-user-provider\"\r\n\
[other]\r\n\
user_setting = \"preserve-me\"\r\n";

    assert_eq!(remove_managed_config(&text).unwrap(), expected);
}

#[test]
fn managed_config_removal_refuses_to_delete_a_user_setting_inside_owned_block() {
    let text = format!(
        "{CONFIG_BEGIN}\n\
model_catalog_json = \"synthetic-owned-catalog.json\"\n\
model = \"synthetic-user-model\"\n\
{CONFIG_END}\n"
    );

    assert!(remove_managed_config(&text).is_err());
}

#[test]
fn a_directory_at_the_sidecar_path_is_not_accepted_as_storage() {
    let dir = TempDir::new("path-type");
    let path = dir.path().join("capabilities.json");
    fs::create_dir(&path).unwrap();
    let context = synthetic_context("synthetic-binding", Some("synthetic-codex"));

    assert!(read_sidecar(&path, &context).is_err());
    assert!(path.is_dir());
}
