use super::*;
use crate::app::model_capability::CapabilityChecks;

const MODEL_ID: &str = "fixture/vendor-model";
const OWNER: &str = "fixture-owner";
const RUNTIME_VERSION: &str = "fixture-native-runtime-1";
const EXECUTABLE_SHA256: &str = "fixture-executable-sha256-a";

fn fixture_context() -> CapabilityContext {
    CapabilityContext {
        binding_fingerprint: "fixture-binding".into(),
        codex_version: Some("fixture-codex".into()),
        native_runtime_version: Some(RUNTIME_VERSION.into()),
        adapter_version: Some("fixture-adapter".into()),
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

fn gateway_model(checked_at: u64) -> ModelCapabilityReport {
    let mut model = ModelCapabilityReport::discovered(MODEL_ID.into(), Some(OWNER.into()));
    model.state = ModelState::Unverified;
    model.checks = verified_checks();
    model.evidence_scope = EvidenceScope::Gateway;
    model.checked_at = Some(checked_at);
    model
}

fn catalog_report(checked_at: u64) -> NativeModelCatalogReport {
    let mut report = super::report("fixture-run", "ok", "fixture");
    report.models = vec![gateway_model(checked_at)];
    report
}

fn native_proof(
    context: &CapabilityContext,
    model_id: &str,
    owned_by: Option<&str>,
    checked_at: u64,
) -> NativeRuntimeEvidence {
    NativeRuntimeEvidence {
        context: context.clone(),
        model_id: model_id.into(),
        owned_by: owned_by.map(str::to_owned),
        checked_at,
        checks: verified_checks(),
    }
}

fn probe_result(
    context: &CapabilityContext,
    model_id: &str,
    owned_by: Option<&str>,
    checked_at: u64,
    slug: &str,
    runtime_version: &str,
    executable_sha256: &str,
) -> NativeProbeResult {
    NativeProbeResult {
        proof: native_proof(context, model_id, owned_by, checked_at),
        catalog_entry: serde_json::json!({
            "slug": slug,
            "display_name": "Fixture model",
        }),
        runtime_version: runtime_version.into(),
        executable_sha256: executable_sha256.into(),
    }
}

fn valid_probe_result(checked_at: u64) -> NativeProbeResult {
    let context = fixture_context();
    probe_result(
        &context,
        MODEL_ID,
        Some(OWNER),
        checked_at,
        MODEL_ID,
        RUNTIME_VERSION,
        EXECUTABLE_SHA256,
    )
}

fn publish_batch(checked_at: u64) -> NativeBatch {
    NativeBatch {
        results: vec![valid_probe_result(checked_at)],
        selection: Some(SelectionReply {
            schema_version: 3,
            request_id: "snapshot-fixture".into(),
            ok: true,
            code: "ok".into(),
            revision: Some("a".repeat(64)),
            selected_model: Some("previous-model".into()),
            config_sha256: None,
            applied: false,
        }),
        executable: Some(std::path::PathBuf::from("fixture-codex.exe")),
        error: None,
    }
}

struct FixtureTempDir(std::path::PathBuf);

impl FixtureTempDir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "codex-usb-publish-fixture-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4().simple()
        ));
        std::fs::create_dir(&path).expect("fixture directory should be created");
        Self(path)
    }

    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for FixtureTempDir {
    fn drop(&mut self) {
        let temp_root = std::env::temp_dir();
        if self.0.parent() == Some(temp_root.as_path())
            && self
                .0
                .file_name()
                .and_then(std::ffi::OsStr::to_str)
                .is_some_and(|name| name.starts_with("codex-usb-publish-fixture-"))
        {
            std::fs::remove_dir_all(&self.0).ok();
        }
    }
}

fn ready_publish_fixture() -> (
    CapabilityContext,
    NativeBatch,
    NativeModelCatalogReport,
    PreparedPublish,
) {
    let checked_at = unix_seconds();
    let context = fixture_context();
    let batch = publish_batch(checked_at);
    let mut result = catalog_report(checked_at);
    result.models[0].evaluate(&context, Some(&batch.results[0].proof), checked_at);
    let cancelled = AtomicBool::new(false);
    let prepared = prepare_publish(
        &batch,
        &result,
        &context,
        MODEL_ID,
        &cancelled,
        Instant::now() + Duration::from_secs(30),
        |_, _, _| Ok(EXECUTABLE_SHA256.into()),
        |_, bytes, _, _| {
            let catalog: serde_json::Value = serde_json::from_slice(bytes).unwrap();
            assert_eq!(catalog["models"][0]["slug"], MODEL_ID);
            Ok(())
        },
    )
    .expect("fixture should pass publish preparation");
    (context, batch, result, prepared)
}

fn fixture_bridge_reply(
    request: &SelectionRequest<'_>,
    selected_model: Option<&str>,
    revision: &str,
    config_sha256: Option<String>,
    applied: bool,
) -> SelectionReply {
    SelectionReply {
        schema_version: 3,
        request_id: request.request_id.to_owned(),
        ok: true,
        code: "ok".into(),
        revision: Some(revision.into()),
        selected_model: selected_model.map(str::to_owned),
        config_sha256,
        applied,
    }
}

fn fixture_bridge_apply(
    request: SelectionRequest<'_>,
    home: &std::path::Path,
    revision: &str,
) -> SelectionReply {
    assert_eq!(request.operation, "apply");
    assert_eq!(request.model, Some(MODEL_ID));
    let catalog_path = std::path::Path::new(request.catalog_path.expect("catalog path"));
    assert!(catalog_path.starts_with(home));
    let catalog = std::fs::read(catalog_path).expect("candidate catalog should exist");
    assert_eq!(request.catalog_sha256, Some(digest_hex(&catalog).as_str()));
    let config = format!(
        "model_provider = \"404token\"\nmodel = \"{}\"\n\n[model_providers.404token]\nbase_url = \"https://404token.xyz/v1\"\nwire_api = \"responses\"\nrequires_openai_auth = true\n",
        request.model.unwrap()
    );
    std::fs::write(home.join("config.toml"), config.as_bytes()).unwrap();
    std::fs::write(
        home.join("auth.json"),
        br#"{"OPENAI_API_KEY":"fixture-key"}"#,
    )
    .unwrap();
    fixture_bridge_reply(
        &request,
        Some(MODEL_ID),
        revision,
        Some(digest_hex(config.as_bytes())),
        true,
    )
}

fn fixture_bridge_snapshot(
    request: SelectionRequest<'_>,
    home: &std::path::Path,
    selected_model: Option<&str>,
    revision: &str,
) -> SelectionReply {
    assert_eq!(request.operation, "snapshot");
    let config_sha256 = std::fs::read(home.join("config.toml"))
        .ok()
        .map(|config| digest_hex(&config));
    fixture_bridge_reply(&request, selected_model, revision, config_sha256, false)
}

#[test]
fn publish_transaction_confirms_lost_apply_ack_against_files_and_snapshot() {
    let temp = FixtureTempDir::new();
    let home = temp.path().to_path_buf();
    let (context, batch, mut result, prepared) = ready_publish_fixture();
    let revision = "c".repeat(64);
    let apply_home = home.clone();
    let apply_revision = revision.clone();
    let apply = move |request: SelectionRequest<'_>| -> Result<SelectionReply, &'static str> {
        let _ = fixture_bridge_apply(request, &apply_home, &apply_revision);
        Err("selection-timeout")
    };
    let snapshot_home = home.clone();
    let snapshot_revision = revision.clone();
    let snapshot = move |request: SelectionRequest<'_>| -> Result<SelectionReply, &'static str> {
        Ok(fixture_bridge_snapshot(
            request,
            &snapshot_home,
            Some(MODEL_ID),
            &snapshot_revision,
        ))
    };
    let cancelled = AtomicBool::new(false);
    let confirmed = commit_prepared_publish(
        &batch,
        &prepared,
        &mut result,
        "fixture-key",
        codex_config_readback::USB_404TOKEN_PROVIDER_ID,
        "https://404token.xyz/v1",
        &home,
        &cancelled,
        Instant::now() + Duration::from_secs(30),
        apply,
        snapshot,
        || true,
        |_, _, _| Ok(EXECUTABLE_SHA256.into()),
    )
    .expect("matching snapshot plus local file checks should recover a lost ack");

    assert_eq!(confirmed.selected_model, MODEL_ID);
    assert_eq!(confirmed.selected_owner.as_deref(), Some(OWNER));
    let catalog_sha256 = digest_hex(&std::fs::read(home.join(CATALOG_FILE_NAME)).unwrap());
    assert_eq!(catalog_sha256, confirmed.catalog_sha256);
    assert_eq!(
        confirmed.config_sha256,
        digest_hex(&std::fs::read(home.join("config.toml")).unwrap())
    );
    assert_eq!(
        confirmed.auth_sha256,
        digest_hex(&std::fs::read(home.join("auth.json")).unwrap())
    );
    assert!(!context.binding_fingerprint.is_empty());
}

#[test]
fn publish_transaction_compensates_unapplied_bridge_rejection() {
    let temp = FixtureTempDir::new();
    let home = temp.path().to_path_buf();
    let (_context, batch, mut result, prepared) = ready_publish_fixture();
    let snapshot = |request: SelectionRequest<'_>| -> Result<SelectionReply, &'static str> {
        Ok(fixture_bridge_reply(
            &request,
            None,
            &"d".repeat(64),
            None,
            false,
        ))
    };
    let cancelled = AtomicBool::new(false);
    let outcome = commit_prepared_publish(
        &batch,
        &prepared,
        &mut result,
        "fixture-key",
        codex_config_readback::USB_404TOKEN_PROVIDER_ID,
        "https://404token.xyz/v1",
        &home,
        &cancelled,
        Instant::now() + Duration::from_secs(30),
        |_request: SelectionRequest<'_>| -> Result<SelectionReply, &'static str> {
            Err("selection-bridge-failed")
        },
        snapshot,
        || panic!("binding must not be accepted after an unconfirmed apply"),
        |_, _, _| panic!("runtime must not be rechecked after a rejected apply"),
    );

    assert!(matches!(outcome, Err("selection-bridge-failed")));
    assert_eq!(result.activation_status, "incomplete");
    assert!(!home.join(CATALOG_FILE_NAME).exists());
    assert!(!home.join("config.toml").exists());
    assert!(!home.join("auth.json").exists());
}

#[test]
fn publish_transaction_preserves_uncertain_files_when_lost_apply_is_not_confirmed() {
    let temp = FixtureTempDir::new();
    let home = temp.path().to_path_buf();
    let (_context, batch, mut result, prepared) = ready_publish_fixture();
    let revision = "f".repeat(64);
    let apply_home = home.clone();
    let apply_revision = revision.clone();
    let apply = move |request: SelectionRequest<'_>| -> Result<SelectionReply, &'static str> {
        let _ = fixture_bridge_apply(request, &apply_home, &apply_revision);
        Err("selection-timeout")
    };
    let snapshot_home = home.clone();
    let snapshot_revision = revision.clone();
    let snapshot = move |request: SelectionRequest<'_>| -> Result<SelectionReply, &'static str> {
        Ok(fixture_bridge_snapshot(
            request,
            &snapshot_home,
            Some("previous-model"),
            &snapshot_revision,
        ))
    };
    let cancelled = AtomicBool::new(false);
    let outcome = commit_prepared_publish(
        &batch,
        &prepared,
        &mut result,
        "fixture-key",
        codex_config_readback::USB_404TOKEN_PROVIDER_ID,
        "https://404token.xyz/v1",
        &home,
        &cancelled,
        Instant::now() + Duration::from_secs(30),
        apply,
        snapshot,
        || panic!("unconfirmed bridge outcome must stop before binding promotion"),
        |_, _, _| panic!("unconfirmed bridge outcome must stop before runtime promotion"),
    );

    assert!(matches!(outcome, Err("selection-outcome-unconfirmed")));
    assert_eq!(result.activation_status, "outcome_unconfirmed");
    assert!(home.join(CATALOG_FILE_NAME).exists());
    assert!(home.join("config.toml").exists());
    assert!(home.join("auth.json").exists());
}

#[test]
fn publish_transaction_rejects_binding_change_after_matching_snapshot() {
    let temp = FixtureTempDir::new();
    let home = temp.path().to_path_buf();
    let (_context, batch, mut result, prepared) = ready_publish_fixture();
    let revision = "9".repeat(64);
    let apply_home = home.clone();
    let apply_revision = revision.clone();
    let apply = move |request: SelectionRequest<'_>| -> Result<SelectionReply, &'static str> {
        Ok(fixture_bridge_apply(request, &apply_home, &apply_revision))
    };
    let snapshot_home = home.clone();
    let snapshot_revision = revision.clone();
    let snapshot = move |request: SelectionRequest<'_>| -> Result<SelectionReply, &'static str> {
        Ok(fixture_bridge_snapshot(
            request,
            &snapshot_home,
            Some(MODEL_ID),
            &snapshot_revision,
        ))
    };
    let cancelled = AtomicBool::new(false);
    let outcome = commit_prepared_publish(
        &batch,
        &prepared,
        &mut result,
        "fixture-key",
        codex_config_readback::USB_404TOKEN_PROVIDER_ID,
        "https://404token.xyz/v1",
        &home,
        &cancelled,
        Instant::now() + Duration::from_secs(30),
        apply,
        snapshot,
        || false,
        |_, _, _| panic!("changed binding must stop before runtime promotion"),
    );

    assert!(matches!(outcome, Err("selection-binding-changed")));
    assert_eq!(result.activation_status, "outcome_unconfirmed");
}

#[test]
fn publish_transaction_does_not_confirm_a_snapshot_for_another_model() {
    let temp = FixtureTempDir::new();
    let home = temp.path().to_path_buf();
    let (_context, batch, mut result, prepared) = ready_publish_fixture();
    let revision = "e".repeat(64);
    let apply_home = home.clone();
    let apply_revision = revision.clone();
    let apply = move |request: SelectionRequest<'_>| -> Result<SelectionReply, &'static str> {
        Ok(fixture_bridge_apply(request, &apply_home, &apply_revision))
    };
    let snapshot_home = home.clone();
    let snapshot_revision = revision.clone();
    let snapshot = move |request: SelectionRequest<'_>| -> Result<SelectionReply, &'static str> {
        Ok(fixture_bridge_snapshot(
            request,
            &snapshot_home,
            Some("different-model"),
            &snapshot_revision,
        ))
    };
    let cancelled = AtomicBool::new(false);
    let outcome = commit_prepared_publish(
        &batch,
        &prepared,
        &mut result,
        "fixture-key",
        codex_config_readback::USB_404TOKEN_PROVIDER_ID,
        "https://404token.xyz/v1",
        &home,
        &cancelled,
        Instant::now() + Duration::from_secs(30),
        apply,
        snapshot,
        || panic!("mismatched snapshot must stop before binding promotion"),
        |_, _, _| panic!("mismatched snapshot must stop before runtime promotion"),
    );

    assert!(matches!(outcome, Err("selection-readback-mismatch")));
    assert_eq!(result.activation_status, "outcome_unconfirmed");
    assert!(home.join(CATALOG_FILE_NAME).exists());
    assert!(home.join("config.toml").exists());
    assert!(home.join("auth.json").exists());
}

#[test]
fn publish_preflight_binds_runtime_catalog_and_preferred_native_proof() {
    let checked_at = unix_seconds();
    let context = fixture_context();
    let result = catalog_report(checked_at);
    let batch = publish_batch(checked_at);
    let cancelled = AtomicBool::new(false);
    let runtime_calls = std::cell::Cell::new(0);
    let catalog_calls = std::cell::Cell::new(0);

    let prepared = prepare_publish(
        &batch,
        &result,
        &context,
        MODEL_ID,
        &cancelled,
        Instant::now() + Duration::from_secs(30),
        |path, _, _| {
            runtime_calls.set(runtime_calls.get() + 1);
            assert_eq!(path, std::path::Path::new("fixture-codex.exe"));
            Ok(EXECUTABLE_SHA256.into())
        },
        |path, bytes, _, _| {
            catalog_calls.set(catalog_calls.get() + 1);
            assert_eq!(path, std::path::Path::new("fixture-codex.exe"));
            let catalog: serde_json::Value = serde_json::from_slice(bytes).unwrap();
            assert_eq!(catalog["models"][0]["slug"], MODEL_ID);
            Ok(())
        },
    )
    .expect("complete current native proof should pass publish preflight");

    assert_eq!(runtime_calls.get(), 1);
    assert_eq!(catalog_calls.get(), 1);
    assert_eq!(prepared.selected_model, MODEL_ID);
    assert_eq!(
        prepared.snapshot.selected_model.as_deref(),
        Some("previous-model")
    );
    assert_eq!(prepared.expected_executable_sha256, EXECUTABLE_SHA256);
}

#[test]
fn publish_preflight_rejects_missing_proof_runtime_drift_and_catalog_failure() {
    let checked_at = unix_seconds();
    let context = fixture_context();
    let result = catalog_report(checked_at);
    let cancelled = AtomicBool::new(false);
    let deadline = Instant::now() + Duration::from_secs(30);

    let mut missing_proof = publish_batch(checked_at);
    missing_proof.results.clear();
    assert!(matches!(
        prepare_publish(
            &missing_proof,
            &result,
            &context,
            MODEL_ID,
            &cancelled,
            deadline,
            |_, _, _| panic!("runtime must not be checked without native proof"),
            |_, _, _, _| panic!("catalog must not be validated without native proof"),
        ),
        Err("native-evidence-missing")
    ));

    let batch = publish_batch(checked_at);
    assert!(matches!(
        prepare_publish(
            &batch,
            &result,
            &context,
            MODEL_ID,
            &cancelled,
            deadline,
            |_, _, _| Ok("different-executable-hash".into()),
            |_, _, _, _| panic!("catalog must not be validated after runtime drift"),
        ),
        Err("native-executable-changed")
    ));

    assert!(matches!(
        prepare_publish(
            &batch,
            &result,
            &context,
            MODEL_ID,
            &cancelled,
            deadline,
            |_, _, _| Ok(EXECUTABLE_SHA256.into()),
            |_, _, _, _| Err("native-catalog-unsupported"),
        ),
        Err("native-catalog-unsupported")
    ));
}

#[test]
fn post_apply_snapshot_must_match_the_applied_model_revision_and_config_hash() {
    let revision = "a".repeat(64);
    let config_sha256 = "b".repeat(64);
    let applied = SelectionReply {
        schema_version: 3,
        request_id: "apply-fixture".into(),
        ok: true,
        code: "ok".into(),
        revision: Some(revision.clone()),
        selected_model: Some(MODEL_ID.into()),
        config_sha256: Some(config_sha256.clone()),
        applied: true,
    };
    let snapshot = |model: Option<&str>, revision: Option<&str>, config: Option<&str>, applied| {
        SelectionReply {
            schema_version: 3,
            request_id: "snapshot-fixture".into(),
            ok: true,
            code: "ok".into(),
            revision: revision.map(str::to_owned),
            selected_model: model.map(str::to_owned),
            config_sha256: config.map(str::to_owned),
            applied,
        }
    };
    let matching = snapshot(Some(MODEL_ID), Some(&revision), Some(&config_sha256), false);
    assert!(post_apply_snapshot_matches(&applied, &matching, MODEL_ID));

    for mismatched in [
        snapshot(
            Some("another-model"),
            Some(&revision),
            Some(&config_sha256),
            false,
        ),
        snapshot(
            Some(MODEL_ID),
            Some(&"c".repeat(64)),
            Some(&config_sha256),
            false,
        ),
        snapshot(
            Some(MODEL_ID),
            Some(&revision),
            Some(&"d".repeat(64)),
            false,
        ),
        snapshot(Some(MODEL_ID), Some(&revision), Some(&config_sha256), true),
        snapshot(None, Some(&revision), Some(&config_sha256), false),
    ] {
        assert!(!post_apply_snapshot_matches(
            &applied,
            &mismatched,
            MODEL_ID
        ));
    }
}

#[test]
fn post_apply_snapshot_protocol_uses_a_fresh_metadata_only_request_and_fails_closed() {
    let revision = "e".repeat(64);
    let config_sha256 = "f".repeat(64);
    let applied = SelectionReply {
        schema_version: 3,
        request_id: "apply-fixture".into(),
        ok: true,
        code: "ok".into(),
        revision: Some(revision.clone()),
        selected_model: Some(MODEL_ID.into()),
        config_sha256: Some(config_sha256.clone()),
        applied: true,
    };
    let seen_request_id = std::cell::RefCell::new(None);
    let readback = request_post_apply_snapshot(&applied, MODEL_ID, |request| {
        assert_eq!(request.schema_version, 3);
        assert!(request.request_id.starts_with("snapshot-"));
        assert_eq!(request.operation, "snapshot");
        assert_eq!(request.expected_revision, None);
        assert_eq!(request.model, None);
        assert_eq!(request.catalog_path, None);
        assert_eq!(request.catalog_sha256, None);
        *seen_request_id.borrow_mut() = Some(request.request_id.to_owned());
        Ok(SelectionReply {
            schema_version: 3,
            request_id: request.request_id.to_owned(),
            ok: true,
            code: "ok".into(),
            revision: Some(revision.clone()),
            selected_model: Some(MODEL_ID.into()),
            config_sha256: Some(config_sha256.clone()),
            applied: false,
        })
    })
    .unwrap();
    assert_eq!(readback.selected_model.as_deref(), Some(MODEL_ID));
    assert!(seen_request_id.borrow().is_some());

    let mismatch = request_post_apply_snapshot(&applied, MODEL_ID, |request| {
        Ok(SelectionReply {
            schema_version: 3,
            request_id: request.request_id.into(),
            ok: true,
            code: "ok".into(),
            revision: Some(revision.clone()),
            selected_model: Some("another-model".into()),
            config_sha256: Some(config_sha256.clone()),
            applied: false,
        })
    });
    assert_eq!(mismatch.err().unwrap(), "selection-readback-mismatch");

    let failed = request_post_apply_snapshot(&applied, MODEL_ID, |_| Err("selection-timeout"));
    assert_eq!(failed.err().unwrap(), "selection-timeout");
}

#[test]
fn failed_apply_compensation_restores_only_when_config_auth_and_candidate_are_unchanged() {
    type StateSnapshot = std::collections::BTreeMap<SelectionStateFile, Option<Vec<u8>>>;

    fn run(
        previous_catalog: Option<Vec<u8>>,
        previous_config: Vec<u8>,
        previous_auth: Vec<u8>,
        current: StateSnapshot,
        fail_write: bool,
        fail_remove: bool,
    ) -> (Result<(), &'static str>, StateSnapshot) {
        let state = std::cell::RefCell::new(current);
        let outcome = compensate_apply_failure(
            b"candidate-catalog",
            previous_catalog.as_deref(),
            Some(previous_config.as_slice()),
            Some(previous_auth.as_slice()),
            |file| Ok(state.borrow().get(&file).cloned().flatten()),
            |file, bytes| {
                if fail_write {
                    return Err("fixture-write-failed");
                }
                state.borrow_mut().insert(file, Some(bytes.to_vec()));
                Ok(())
            },
            |file| {
                if fail_remove {
                    return Err("fixture-remove-failed");
                }
                state.borrow_mut().insert(file, None);
                Ok(())
            },
        );
        (outcome, state.into_inner())
    }

    fn files(
        config: &[u8],
        auth: &[u8],
        catalog: Option<&[u8]>,
    ) -> std::collections::BTreeMap<SelectionStateFile, Option<Vec<u8>>> {
        std::collections::BTreeMap::from([
            (SelectionStateFile::Config, Some(config.to_vec())),
            (SelectionStateFile::Auth, Some(auth.to_vec())),
            (SelectionStateFile::Catalog, catalog.map(<[u8]>::to_vec)),
        ])
    }

    let old_catalog = b"previous-catalog";
    let old_config = b"previous-config";
    let old_auth = b"previous-auth";
    let (restored, restored_files) = run(
        Some(old_catalog.to_vec()),
        old_config.to_vec(),
        old_auth.to_vec(),
        files(old_config, old_auth, Some(b"candidate-catalog")),
        false,
        false,
    );
    assert_eq!(restored, Ok(()));
    assert_eq!(
        restored_files[&SelectionStateFile::Catalog].as_deref(),
        Some(old_catalog.as_slice())
    );

    let (removed, removed_files) = run(
        None,
        old_config.to_vec(),
        old_auth.to_vec(),
        files(old_config, old_auth, Some(b"candidate-catalog")),
        false,
        false,
    );
    assert_eq!(removed, Ok(()));
    assert_eq!(removed_files[&SelectionStateFile::Catalog], None);

    for changed_file in [SelectionStateFile::Config, SelectionStateFile::Auth] {
        let mut changed = files(old_config, old_auth, Some(b"candidate-catalog"));
        changed.insert(changed_file, Some(b"external-change".to_vec()));
        let (outcome, unchanged_catalog) = run(
            Some(old_catalog.to_vec()),
            old_config.to_vec(),
            old_auth.to_vec(),
            changed,
            false,
            false,
        );
        assert_eq!(outcome, Err("selection-outcome-unconfirmed"));
        assert_eq!(
            unchanged_catalog[&SelectionStateFile::Catalog].as_deref(),
            Some(b"candidate-catalog".as_slice())
        );
    }

    let (catalog_conflict, conflicting_files) = run(
        Some(old_catalog.to_vec()),
        old_config.to_vec(),
        old_auth.to_vec(),
        files(old_config, old_auth, Some(b"external-catalog")),
        false,
        false,
    );
    assert_eq!(catalog_conflict, Err("selection-rollback-conflict"));
    assert_eq!(
        conflicting_files[&SelectionStateFile::Catalog].as_deref(),
        Some(b"external-catalog".as_slice())
    );

    let (restore_failed, retained) = run(
        Some(old_catalog.to_vec()),
        old_config.to_vec(),
        old_auth.to_vec(),
        files(old_config, old_auth, Some(b"candidate-catalog")),
        true,
        false,
    );
    assert_eq!(restore_failed, Err("selection-rollback-incomplete"));
    assert_eq!(
        retained[&SelectionStateFile::Catalog].as_deref(),
        Some(b"candidate-catalog".as_slice())
    );

    let (remove_failed, retained) = run(
        None,
        old_config.to_vec(),
        old_auth.to_vec(),
        files(old_config, old_auth, Some(b"candidate-catalog")),
        false,
        true,
    );
    assert_eq!(remove_failed, Err("selection-rollback-incomplete"));
    assert_eq!(
        retained[&SelectionStateFile::Catalog].as_deref(),
        Some(b"candidate-catalog".as_slice())
    );
}

#[test]
fn selection_apply_finalizer_orders_snapshot_and_compensation_by_apply_outcome() {
    fn reply(model: &str, revision: &str, config_hash: &str, applied: bool) -> SelectionReply {
        SelectionReply {
            schema_version: 3,
            request_id: if applied {
                "apply-fixture"
            } else {
                "snapshot-fixture"
            }
            .into(),
            ok: true,
            code: "ok".into(),
            revision: Some(revision.into()),
            selected_model: Some(model.into()),
            config_sha256: Some(config_hash.into()),
            applied,
        }
    }

    let snapshot_calls = std::cell::Cell::new(0);
    let compensation_calls = std::cell::Cell::new(0);
    let confirmed = finalize_selection_apply(
        "apply-fixture",
        Ok(reply(MODEL_ID, &"a".repeat(64), &"b".repeat(64), true)),
        MODEL_ID,
        |request| {
            snapshot_calls.set(snapshot_calls.get() + 1);
            assert_eq!(request.operation, "snapshot");
            let mut snapshot = reply(MODEL_ID, &"a".repeat(64), &"b".repeat(64), false);
            snapshot.request_id = request.request_id.into();
            Ok(snapshot)
        },
        || {
            compensation_calls.set(compensation_calls.get() + 1);
            Ok(())
        },
    );
    let (_, snapshot) = confirmed.unwrap();
    assert_eq!(snapshot.selected_model.as_deref(), Some(MODEL_ID));
    assert_eq!(snapshot_calls.get(), 1);
    assert_eq!(compensation_calls.get(), 0);

    let snapshot_calls = std::cell::Cell::new(0);
    let compensated = finalize_selection_apply::<_, _>(
        "apply-fixture",
        Err("selection-bridge-failed"),
        MODEL_ID,
        |request| {
            snapshot_calls.set(snapshot_calls.get() + 1);
            let mut snapshot = reply("previous-model", &"a".repeat(64), &"b".repeat(64), false);
            snapshot.request_id = request.request_id.into();
            Ok(snapshot)
        },
        || Ok(()),
    );
    assert_eq!(
        compensated.err().unwrap(),
        SelectionApplyFailure::ApplyRejected("selection-bridge-failed")
    );
    assert_eq!(snapshot_calls.get(), 1);

    let compensation_failure = finalize_selection_apply::<_, _>(
        "apply-fixture",
        Err("selection-timeout"),
        MODEL_ID,
        |_| Err("unexpected-snapshot"),
        || Err("selection-outcome-unconfirmed"),
    );
    assert_eq!(
        compensation_failure.err().unwrap(),
        SelectionApplyFailure::CompensationFailed("selection-outcome-unconfirmed")
    );

    let readback_failure = finalize_selection_apply(
        "apply-fixture",
        Ok(reply(MODEL_ID, &"c".repeat(64), &"d".repeat(64), true)),
        MODEL_ID,
        |request| {
            let mut snapshot = reply("changed-model", &"c".repeat(64), &"d".repeat(64), false);
            snapshot.request_id = request.request_id.into();
            Ok(snapshot)
        },
        || panic!("a successful apply must not run failure compensation"),
    );
    assert_eq!(
        readback_failure.err().unwrap(),
        SelectionApplyFailure::ReadbackFailed("selection-readback-mismatch")
    );
}

#[test]
fn selection_apply_with_compensation_runs_the_combined_mock_state_machine() {
    fn reply(model: &str, revision: &str, config_hash: &str, applied: bool) -> SelectionReply {
        SelectionReply {
            schema_version: 3,
            request_id: if applied {
                "apply-fixture"
            } else {
                "snapshot-fixture"
            }
            .into(),
            ok: true,
            code: "ok".into(),
            revision: Some(revision.into()),
            selected_model: Some(model.into()),
            config_sha256: Some(config_hash.into()),
            applied,
        }
    }

    type MockFiles = std::collections::BTreeMap<SelectionStateFile, Option<Vec<u8>>>;
    fn files(config: &[u8], auth: &[u8], catalog: &[u8]) -> MockFiles {
        std::collections::BTreeMap::from([
            (SelectionStateFile::Config, Some(config.to_vec())),
            (SelectionStateFile::Auth, Some(auth.to_vec())),
            (SelectionStateFile::Catalog, Some(catalog.to_vec())),
        ])
    }
    fn run(
        apply_result: Result<SelectionReply, &'static str>,
        snapshot_result: Result<SelectionReply, &'static str>,
        current: MockFiles,
    ) -> (
        Result<(SelectionReply, SelectionReply), SelectionApplyFailure>,
        MockFiles,
        usize,
    ) {
        let state = std::cell::RefCell::new(current);
        let snapshot_calls = std::cell::Cell::new(0);
        let outcome = finalize_selection_apply_with_compensation(
            "apply-fixture",
            apply_result,
            MODEL_ID,
            b"candidate-catalog",
            Some(b"previous-catalog"),
            Some(b"previous-config"),
            Some(b"previous-auth"),
            |request| {
                snapshot_calls.set(snapshot_calls.get() + 1);
                snapshot_result.map(|mut snapshot| {
                    snapshot.request_id = request.request_id.into();
                    snapshot
                })
            },
            |file| Ok(state.borrow().get(&file).cloned().flatten()),
            |file, bytes| {
                state.borrow_mut().insert(file, Some(bytes.to_vec()));
                Ok(())
            },
            |file| {
                state.borrow_mut().insert(file, None);
                Ok(())
            },
        );
        (outcome, state.into_inner(), snapshot_calls.get())
    }

    let revision = "a".repeat(64);
    let config_hash = "b".repeat(64);
    let (confirmed, confirmed_files, snapshot_calls) = run(
        Ok(reply(MODEL_ID, &revision, &config_hash, true)),
        Ok(reply(MODEL_ID, &revision, &config_hash, false)),
        files(b"applied-config", b"previous-auth", b"candidate-catalog"),
    );
    assert!(confirmed.is_ok());
    assert_eq!(snapshot_calls, 1);
    assert_eq!(
        confirmed_files[&SelectionStateFile::Catalog].as_deref(),
        Some(b"candidate-catalog".as_slice())
    );

    let (rejected, restored_files, snapshot_calls) = run(
        Err("selection-bridge-failed"),
        Ok(reply("previous-model", &revision, &config_hash, false)),
        files(b"previous-config", b"previous-auth", b"candidate-catalog"),
    );
    assert_eq!(
        rejected.err().unwrap(),
        SelectionApplyFailure::ApplyRejected("selection-bridge-failed")
    );
    assert_eq!(snapshot_calls, 1);
    assert_eq!(
        restored_files[&SelectionStateFile::Catalog].as_deref(),
        Some(b"previous-catalog".as_slice())
    );

    let (reconciled, reconciled_files, snapshot_calls) = run(
        Err("selection-timeout"),
        Ok(reply(MODEL_ID, &revision, &config_hash, false)),
        files(b"applied-config", b"applied-auth", b"candidate-catalog"),
    );
    let (confirmed_apply, confirmed_snapshot) = reconciled.unwrap();
    assert!(confirmed_apply.applied);
    assert_eq!(confirmed_apply.request_id, "apply-fixture");
    assert_eq!(confirmed_snapshot.selected_model.as_deref(), Some(MODEL_ID));
    assert_eq!(snapshot_calls, 1);
    assert_eq!(
        reconciled_files[&SelectionStateFile::Catalog].as_deref(),
        Some(b"candidate-catalog".as_slice())
    );

    let (uncertain, retained_files, snapshot_calls) = run(
        Err("selection-timeout"),
        Err("selection-snapshot-timeout"),
        files(b"changed-config", b"previous-auth", b"candidate-catalog"),
    );
    assert_eq!(
        uncertain.err().unwrap(),
        SelectionApplyFailure::CompensationFailed("selection-outcome-unconfirmed")
    );
    assert_eq!(snapshot_calls, 1);
    assert_eq!(
        retained_files[&SelectionStateFile::Catalog].as_deref(),
        Some(b"candidate-catalog".as_slice())
    );

    let (readback_mismatch, retained_files, snapshot_calls) = run(
        Ok(reply(MODEL_ID, &revision, &config_hash, true)),
        Ok(reply("changed-model", &revision, &config_hash, false)),
        files(b"applied-config", b"previous-auth", b"candidate-catalog"),
    );
    assert_eq!(
        readback_mismatch.err().unwrap(),
        SelectionApplyFailure::ReadbackFailed("selection-readback-mismatch")
    );
    assert_eq!(snapshot_calls, 1);
    assert_eq!(
        retained_files[&SelectionStateFile::Catalog].as_deref(),
        Some(b"candidate-catalog".as_slice())
    );
}

#[test]
fn confirmed_selection_and_snapshot_must_precede_complete_checkpoint_and_launch_proof() {
    use std::cell::{Cell, RefCell};

    let model_id = "glm-5.2";
    let clock = super::super::SetupClock {
        boot_identity: "2026-09-25T12:00:00.0000000Z".into(),
        monotonic_ms: 1_000,
        utc_now: "2026-09-25T16:00:00Z".into(),
        unix_now: 1_790_352_000,
    };
    let candidates = vec![model_id.to_owned()];
    let revision = "a".repeat(64);
    let config_hash = "b".repeat(64);
    let mut checkpoint = super::super::new_auto_setup_checkpoint(
        "apply-fixture",
        &candidates,
        "c4d48cf753c62f166d848c06c1fc25a23430316d",
        &"c".repeat(64),
        &clock,
    )
    .unwrap();
    checkpoint.phase = AutoSetupPhase::ApplyPending;
    checkpoint.candidates[0].status = AutoSetupCandidateStatus::NativeComplete;
    checkpoint.candidates[0].result_code = Some("native-complete".into());

    let mut snapshot_dispatches = 0;
    let snapshot_confirmed = Cell::new(false);
    let selection = finalize_selection_apply(
        "apply-fixture",
        Err("selection-timeout"),
        model_id,
        |request| {
            snapshot_dispatches += 1;
            assert_eq!(request.operation, "snapshot");
            snapshot_confirmed.set(true);
            Ok(SelectionReply {
                schema_version: 3,
                request_id: request.request_id.into(),
                ok: true,
                code: "ok".into(),
                revision: Some(revision.clone()),
                selected_model: Some(model_id.into()),
                config_sha256: Some(config_hash.clone()),
                applied: false,
            })
        },
        || panic!("confirmed apply does not run compensation"),
    );
    assert!(selection.is_ok());
    assert_eq!(snapshot_dispatches, 1);

    let context = CapabilityContext {
        binding_fingerprint: "fixture-binding".into(),
        codex_version: Some("fixture-codex".into()),
        native_runtime_version: Some(RUNTIME_VERSION.into()),
        adapter_version: Some("fixture-adapter".into()),
    };
    let models = vec![ModelCapabilityReport::discovered(
        model_id.into(),
        Some("zhipu".into()),
    )];
    let pending = b"apply-pending-sidecar".to_vec();
    let mut expected_sidecar = Some(pending.clone());
    let sidecar_store = RefCell::new(Some(pending));
    let launch_proof_promoted = Cell::new(false);
    super::super::complete_auto_setup_after_apply(
        selection.is_ok(),
        Some(&mut checkpoint),
        Some(model_id),
        &context,
        clock.unix_now,
        &models,
        &mut expected_sidecar,
        || Ok(sidecar_store.borrow().clone()),
        |bytes| {
            let sidecar: serde_json::Value = serde_json::from_slice(bytes).unwrap();
            assert_eq!(sidecar["autoSetup"]["phase"], "complete");
            assert!(snapshot_confirmed.get());
            *sidecar_store.borrow_mut() = Some(bytes.to_vec());
            Ok(())
        },
        || {
            assert!(snapshot_confirmed.get());
            let sidecar: serde_json::Value = serde_json::from_slice(
                sidecar_store
                    .borrow()
                    .as_deref()
                    .expect("complete sidecar persisted"),
            )
            .unwrap();
            assert_eq!(sidecar["autoSetup"]["phase"], "complete");
            launch_proof_promoted.set(true);
            Ok(())
        },
    )
    .unwrap();
    assert!(launch_proof_promoted.get());
    assert!(super::launch_proof_is_current(
        launch_proof_promoted.get(),
        true
    ));
    assert_eq!(checkpoint.phase, AutoSetupPhase::Complete);

    let mut failed_checkpoint = checkpoint.clone();
    failed_checkpoint.phase = AutoSetupPhase::ApplyPending;
    failed_checkpoint.candidates[0].result_code = Some("native-complete".into());
    let failed_selection = finalize_selection_apply(
        "apply-fixture",
        Err("selection-timeout"),
        model_id,
        |request| {
            Ok(SelectionReply {
                schema_version: 3,
                request_id: request.request_id.into(),
                ok: true,
                code: "ok".into(),
                revision: Some(revision.clone()),
                selected_model: None,
                config_sha256: None,
                applied: false,
            })
        },
        || Ok(()),
    );
    assert!(failed_selection.is_err());
    let promoted_after_failure = Cell::new(false);
    super::super::complete_auto_setup_after_apply(
        failed_selection.is_ok(),
        Some(&mut failed_checkpoint),
        Some(model_id),
        &context,
        clock.unix_now,
        &models,
        &mut expected_sidecar,
        || panic!("rejected apply must not inspect a completion checkpoint"),
        |_| panic!("rejected apply must not write a completion checkpoint"),
        || {
            promoted_after_failure.set(true);
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(failed_checkpoint.phase, AutoSetupPhase::ApplyPending);
    assert!(!promoted_after_failure.get());
    assert!(!super::launch_proof_is_current(
        promoted_after_failure.get(),
        true
    ));
}

fn batch(results: Vec<NativeProbeResult>) -> NativeBatch {
    NativeBatch {
        results,
        selection: None,
        executable: None,
        error: None,
    }
}

#[test]
fn catalog_gate_requires_complete_gateway_evidence() {
    let checked_at = unix_seconds();
    let missing_gateway_check = [
        {
            let mut checks = verified_checks();
            checks.text = CapabilityState::Unknown;
            checks
        },
        {
            let mut checks = verified_checks();
            checks.single_tool = CapabilityState::Unknown;
            checks
        },
        {
            let mut checks = verified_checks();
            checks.tool_result = CapabilityState::Unknown;
            checks
        },
        {
            let mut checks = verified_checks();
            checks.multiple_tools = CapabilityState::Unknown;
            checks
        },
        {
            let mut checks = verified_checks();
            checks.tool_error = CapabilityState::Unknown;
            checks
        },
        {
            let mut checks = verified_checks();
            checks.stream = CapabilityState::Unknown;
            checks
        },
        {
            let mut checks = verified_checks();
            checks.multi_turn = CapabilityState::Unknown;
            checks
        },
    ];

    for checks in missing_gateway_check {
        let mut report = catalog_report(checked_at);
        report.models[0].checks = checks;
        assert!(!complete_gateway(&report.models[0]));
        assert!(matches!(
            catalog_bytes(
                &batch(vec![valid_probe_result(checked_at)]),
                &report,
                &fixture_context()
            ),
            Err("native-evidence-missing")
        ));
    }

    let mut report = catalog_report(checked_at);
    report.models[0].error_code = Some("fixture-error".into());
    assert!(!complete_gateway(&report.models[0]));
    assert!(matches!(
        catalog_bytes(
            &batch(vec![valid_probe_result(checked_at)]),
            &report,
            &fixture_context()
        ),
        Err("native-evidence-missing")
    ));
}

#[test]
fn discovery_mock_and_cached_gateway_evidence_cannot_activate_catalog() {
    let checked_at = unix_seconds();
    for scope in [
        EvidenceScope::Discovery,
        EvidenceScope::Mock,
        EvidenceScope::CachedGateway,
    ] {
        let mut report = catalog_report(checked_at);
        report.models[0].evidence_scope = scope;
        assert!(matches!(
            catalog_bytes(
                &batch(vec![valid_probe_result(checked_at)]),
                &report,
                &fixture_context()
            ),
            Err("native-evidence-missing")
        ));
    }
}

#[test]
fn valid_live_evidence_produces_a_valid_native_batch_catalog() {
    let checked_at = unix_seconds();
    let context = fixture_context();
    let bytes = catalog_bytes(
        &batch(vec![valid_probe_result(checked_at)]),
        &catalog_report(checked_at),
        &context,
    )
    .expect("matching fresh native evidence should produce a catalog");
    let catalog: serde_json::Value =
        serde_json::from_slice(&bytes).expect("catalog bytes should be valid JSON");

    assert_eq!(catalog["schema_version"], serde_json::json!(1));
    assert_eq!(catalog["identity"], serde_json::json!(CATALOG_IDENTITY));
    assert_eq!(
        catalog["client_version"],
        serde_json::json!(RUNTIME_VERSION)
    );
    assert_eq!(
        catalog["models"].as_array().map(|models| models.len()),
        Some(1)
    );
    assert_eq!(catalog["models"][0]["slug"], serde_json::json!(MODEL_ID));
}

#[test]
fn native_evidence_requires_matching_live_context_identity_and_freshness() {
    let checked_at = unix_seconds();
    let report = catalog_report(checked_at);
    let context = fixture_context();

    let change_context: [fn(&mut CapabilityContext); 3] = [
        |changed: &mut CapabilityContext| changed.binding_fingerprint = "other-binding".into(),
        |changed: &mut CapabilityContext| changed.codex_version = Some("other-codex".into()),
        |changed: &mut CapabilityContext| changed.adapter_version = Some("other-adapter".into()),
    ];
    for change_context in change_context {
        let mut changed = context.clone();
        change_context(&mut changed);
        let result = probe_result(
            &context,
            MODEL_ID,
            Some(OWNER),
            checked_at,
            MODEL_ID,
            RUNTIME_VERSION,
            EXECUTABLE_SHA256,
        );
        assert!(matches!(
            catalog_bytes(&batch(vec![result]), &report, &changed),
            Err("native-evidence-missing")
        ));
    }

    let wrong_model = probe_result(
        &context,
        "fixture/other-model",
        Some(OWNER),
        checked_at,
        MODEL_ID,
        RUNTIME_VERSION,
        EXECUTABLE_SHA256,
    );
    assert!(matches!(
        catalog_bytes(&batch(vec![wrong_model]), &report, &context),
        Err("native-evidence-mismatch")
    ));

    let wrong_owner = probe_result(
        &context,
        MODEL_ID,
        Some("other-owner"),
        checked_at,
        MODEL_ID,
        RUNTIME_VERSION,
        EXECUTABLE_SHA256,
    );
    assert!(matches!(
        catalog_bytes(&batch(vec![wrong_owner]), &report, &context),
        Err("native-evidence-mismatch")
    ));

    let stale = checked_at.saturating_sub(crate::app::model_capability::EVIDENCE_TTL_SECONDS + 1);
    let stale_proof = probe_result(
        &context,
        MODEL_ID,
        Some(OWNER),
        stale,
        MODEL_ID,
        RUNTIME_VERSION,
        EXECUTABLE_SHA256,
    );
    assert!(matches!(
        catalog_bytes(&batch(vec![stale_proof]), &report, &context),
        Err("native-evidence-missing")
    ));

    let fresh_proof = valid_probe_result(checked_at);
    let stale_report = catalog_report(stale);
    assert!(matches!(
        catalog_bytes(&batch(vec![fresh_proof]), &stale_report, &context),
        Err("native-evidence-missing")
    ));
}

#[test]
fn catalog_rejects_slug_duplicates_runtime_and_hash_conflicts() {
    let checked_at = unix_seconds();
    let context = fixture_context();
    let report = catalog_report(checked_at);

    let mut wrong_slug = valid_probe_result(checked_at);
    wrong_slug.catalog_entry["slug"] = serde_json::json!("fixture/wrong-slug");
    assert!(matches!(
        catalog_bytes(&batch(vec![wrong_slug]), &report, &context),
        Err("native-catalog-ambiguous")
    ));

    assert!(matches!(
        catalog_bytes(
            &batch(vec![
                valid_probe_result(checked_at),
                valid_probe_result(checked_at)
            ]),
            &report,
            &context,
        ),
        Err("native-catalog-ambiguous")
    ));

    let different_hash = probe_result(
        &context,
        MODEL_ID,
        Some(OWNER),
        checked_at,
        MODEL_ID,
        RUNTIME_VERSION,
        "fixture-executable-sha256-b",
    );
    assert!(matches!(
        catalog_bytes(
            &batch(vec![valid_probe_result(checked_at), different_hash]),
            &report,
            &context,
        ),
        Err("native-executable-changed")
    ));

    let different_runtime = probe_result(
        &context,
        MODEL_ID,
        Some(OWNER),
        checked_at,
        MODEL_ID,
        "fixture-native-runtime-2",
        EXECUTABLE_SHA256,
    );
    assert!(matches!(
        catalog_bytes(
            &batch(vec![valid_probe_result(checked_at), different_runtime]),
            &report,
            &context,
        ),
        Err("native-executable-changed")
    ));
}

#[test]
fn absent_native_proof_cannot_produce_a_catalog() {
    let checked_at = unix_seconds();
    assert!(matches!(
        catalog_bytes(
            &batch(Vec::new()),
            &catalog_report(checked_at),
            &fixture_context()
        ),
        Err("native-evidence-missing")
    ));
}

#[test]
fn publish_transaction_rejects_wrong_provider_in_config_readback() {
    let temp = FixtureTempDir::new();
    let home = temp.path().to_path_buf();
    let (_context, batch, mut result, prepared) = ready_publish_fixture();
    let revision = "a".repeat(64);
    let apply_home = home.clone();
    let apply_revision = revision.clone();
    let apply = move |request: SelectionRequest<'_>| -> Result<SelectionReply, &'static str> {
        let mut reply = fixture_bridge_apply(request, &apply_home, &apply_revision);
        let config = format!(
            "model_provider = \"wrong-provider\"\nmodel = \"{}\"\n\n[model_providers.wrong-provider]\nbase_url = \"https://404token.xyz/v1\"\nwire_api = \"responses\"\nrequires_openai_auth = true\n",
            MODEL_ID
        );
        std::fs::write(apply_home.join("config.toml"), config.as_bytes()).unwrap();
        reply.config_sha256 = Some(digest_hex(config.as_bytes()));
        Ok(reply)
    };
    let snapshot_home = home.clone();
    let snapshot_revision = revision.clone();
    let snapshot = move |request: SelectionRequest<'_>| -> Result<SelectionReply, &'static str> {
        Ok(fixture_bridge_snapshot(
            request,
            &snapshot_home,
            Some(MODEL_ID),
            &snapshot_revision,
        ))
    };
    let cancelled = AtomicBool::new(false);
    let outcome = commit_prepared_publish(
        &batch,
        &prepared,
        &mut result,
        "fixture-key",
        codex_config_readback::USB_404TOKEN_PROVIDER_ID,
        "https://404token.xyz/v1",
        &home,
        &cancelled,
        Instant::now() + Duration::from_secs(30),
        apply,
        snapshot,
        || true,
        |_, _, _| panic!("config readback must fail before runtime promotion"),
    );
    assert!(matches!(outcome, Err("selection-config-readback-mismatch")));
    assert_eq!(result.activation_status, "outcome_unconfirmed");
}

#[test]
fn publish_transaction_rejects_wrong_endpoint_in_config_readback() {
    let temp = FixtureTempDir::new();
    let home = temp.path().to_path_buf();
    let (_context, batch, mut result, prepared) = ready_publish_fixture();
    let revision = "b".repeat(64);
    let apply_home = home.clone();
    let apply_revision = revision.clone();
    let apply = move |request: SelectionRequest<'_>| -> Result<SelectionReply, &'static str> {
        let mut reply = fixture_bridge_apply(request, &apply_home, &apply_revision);
        let config = format!(
            "model_provider = \"404token\"\nmodel = \"{}\"\n\n[model_providers.404token]\nbase_url = \"https://wrong.example/v1\"\nwire_api = \"responses\"\nrequires_openai_auth = true\n",
            MODEL_ID
        );
        std::fs::write(apply_home.join("config.toml"), config.as_bytes()).unwrap();
        reply.config_sha256 = Some(digest_hex(config.as_bytes()));
        Ok(reply)
    };
    let snapshot_home = home.clone();
    let snapshot_revision = revision.clone();
    let snapshot = move |request: SelectionRequest<'_>| -> Result<SelectionReply, &'static str> {
        Ok(fixture_bridge_snapshot(
            request,
            &snapshot_home,
            Some(MODEL_ID),
            &snapshot_revision,
        ))
    };
    let cancelled = AtomicBool::new(false);
    let outcome = commit_prepared_publish(
        &batch,
        &prepared,
        &mut result,
        "fixture-key",
        codex_config_readback::USB_404TOKEN_PROVIDER_ID,
        "https://404token.xyz/v1",
        &home,
        &cancelled,
        Instant::now() + Duration::from_secs(30),
        apply,
        snapshot,
        || true,
        |_, _, _| panic!("config readback must fail before runtime promotion"),
    );
    assert!(matches!(outcome, Err("selection-config-readback-mismatch")));
    assert_eq!(result.activation_status, "outcome_unconfirmed");
}
