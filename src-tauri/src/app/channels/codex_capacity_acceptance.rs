// Opt-in real-file capacity test, included only inside codex::tests.
// The caller creates an empty ~8GB NTFS test volume and copies a complete
// previous release into it. No physical disk is created/formatted by this test.
#[test]
#[ignore = "requires CODEX_8GB_TEST_ROOT, CODEX_8GB_NEW_MSIX and CODEX_8GB_EVIDENCE on an isolated 8GB volume"]
fn real_8gb_payload_upgrade_repeat_and_interruption_recovery() {
    use codex_win_engine::{PortableInstallOptions, PortableFault};
    let root = PathBuf::from(std::env::var_os("CODEX_8GB_TEST_ROOT").expect("isolated bundle root"));
    let package = PathBuf::from(std::env::var_os("CODEX_8GB_NEW_MSIX").expect("verified new MSIX"));
    let evidence = PathBuf::from(std::env::var_os("CODEX_8GB_EVIDENCE").expect("evidence directory"));
    let install = root.join("apps/Codex");
    assert!(!install.exists(), "test requires a fresh release copy");
    let minimum_free = std::cell::Cell::new(u64::MAX);
    let observations = std::cell::RefCell::new(Vec::new());
    let observe = |phase: &str| {
        let free = crate::app::disk::available_space(&root).unwrap().unwrap();
        minimum_free.set(minimum_free.get().min(free));
        observations.borrow_mut().push(serde_json::json!({"phase":phase,"free":free}));
        println!("capacity phase={phase} free={free}");
    };
    let initial = crate::app::disk::available_space(&root).unwrap().unwrap();
    assert!(initial < 6_100_000_000 && initial > 5_500_000_000,
        "full old release on the intended 8GB volume should leave about 5.9GB, got {initial}");
    let local = require_capability(&root).unwrap();
    assert_eq!(local.current_version, "26.1016.62342");
    let signature = codex_win_engine::verify_openai_authenticode(&local.payload_path).unwrap();
    assert!(signature.is_valid_openai());
    let options = || PortableInstallOptions {
        usb_entry_shim: Some(root.join("CodexEntryShim.exe")),
        ..PortableInstallOptions::usb_offline()
    };
    let record_progress = |_: codex_win_engine::PortableExtractionProgress| {
        let free = crate::app::disk::available_space(&root).unwrap().unwrap();
        minimum_free.set(minimum_free.get().min(free));
        Ok(())
    };
    observe("old-release-copied");
    codex_win_engine::install_portable_from_msix_with_observer_options_and_progress(
        &local.payload_path, &install, options(), &mut |_| Ok(()), Some(&record_progress),
    ).unwrap();
    observe("old-app-installed");
    let identity = read_msix_identity(&package).unwrap();
    let app_version = codex_win_engine::read_codex_app_version_from_msix(&package).unwrap();
    assert_eq!(app_version, "26.1111.21308");
    let hash = sha256_file(&package).unwrap();
    let moniker = codex_win_engine::canonical_codex_package_moniker(&identity).unwrap();
    let checksums = format!("{hash}  {moniker}.Msix\n");
    let mut fresh = fake_resolved_release();
    fresh.app_version = app_version.clone();
    fresh.identity.app_version = app_version.clone();
    fresh.identity.msix_version = identity.version;
    fresh.identity.package_moniker = moniker;
    fresh.identity.github_digest = Some(hash.clone());
    fresh.msix.expected_sha256 = Some(hash);
    fresh.msix.size = fs::metadata(&package).unwrap().len();
    fresh.identity.msix_size = fresh.msix.size;
    // Only download is replaced by a local copy of the already downloaded
    // original signed asset. Production digest, Authenticode and identity
    // checks, metadata commit and recovery are all executed unmodified.
    let mut transport = production_apply_transport(&NetworkConfig::direct(), false);
    transport.download = Box::new(move |_, dest, _, progress| {
        let copied = fs::copy(&package, dest).map_err(|e| e.to_string())?;
        progress(copied);
        Ok(())
    });
    transport.fetch_text = Box::new(move |_| Ok(checksums.clone()));
    let stable_anchor = || {
        crate::app::usb_guard::stable_anchor_sha256_from_bytes(
            &fs::read(root.join(ROOT_MANIFEST_NAME)).unwrap(),
            &fs::read(root.join(USB_MANIFEST_NAME)).unwrap(),
            &fs::read(root.join(SHA256SUMS_NAME)).unwrap(),
        ).unwrap()
    };
    let anchor_before = stable_anchor();
    for phase in ["interrupted-metadata-commit", "upgrade", "repeat-same-verified-payload"] {
        let (local, tx) = prepare_transaction(&root);
        let expected = UpdateInfo {
            current_version: local.current_version.clone(),
            latest_version: app_version.clone(),
            payload: codex_update_payload(&root, &local, &fresh.identity).unwrap(),
            ..fake_update_info(&root, &local)
        };
        if phase == "interrupted-metadata-commit" {
            fs::create_dir(tx.join("next").join(USB_MANIFEST_NAME)).unwrap();
        }
        let outcome = apply_transaction_with_transport(
            &root, &tx, &local, &fresh, &expected, &|_, _| observe("downloaded"), &transport,
        );
        if phase == "interrupted-metadata-commit" {
            assert!(outcome.is_err());
            assert!(tx.join(TRANSACTION_PAYLOAD_BACKUP).is_file());
            recover_pending_transactions(&root).unwrap();
            let restored = require_capability(&root).unwrap();
            assert_eq!(restored.current_version, local.current_version);
            assert_eq!(stable_anchor(), anchor_before);
            observe("metadata-interruption-recovered");
            continue;
        }
        outcome.unwrap();
        assert!(!tx.exists());
        assert_eq!(fs::read_dir(root.join("payload/_backup")).unwrap().count(), 0);
        assert_eq!(stable_anchor(), anchor_before);
        observe("payload-committed-old-msix-released");
        let current = require_capability(&root).unwrap();
        if phase == "upgrade" {
            codex_win_engine::inject_portable_fault(Some(PortableFault::OnMoveNew));
            let failed = codex_win_engine::install_portable_from_msix_with_observer_options_and_progress(
                &current.payload_path, &install, options(), &mut |_| Ok(()), Some(&record_progress),
            );
            assert!(failed.is_err());
            assert_eq!(codex_win_engine::read_codex_app_version_from_install_root(&install).unwrap(), "26.1016.62342");
            observe("application-replace-failure-rolled-back");
        }
        codex_win_engine::install_portable_from_msix_with_observer_options_and_progress(
            &current.payload_path, &install, options(), &mut |_| Ok(()), Some(&record_progress),
        ).unwrap();
        assert_eq!(codex_win_engine::read_codex_app_version_from_install_root(&install).unwrap(), app_version);
        assert!(!root.join("apps/.codex-app-manager-staging").exists());
        assert!(!fs::read_dir(root.join("apps")).unwrap().any(|e| e.unwrap().file_name().to_string_lossy().starts_with("Codex.rollback-")));
        observe(phase);
    }
    let sums = fs::read(root.join(SHA256SUMS_NAME)).unwrap();
    verify_sha256_sums(&root, &sums, &BTreeMap::new()).unwrap();
    fs::write(evidence.join("real-8gb-result.json"), serde_json::to_vec_pretty(&serde_json::json!({
        "success":true,"root":root,"min_free_observed_bytes":minimum_free.get(),
        "observations":observations.into_inner(),"stable_anchor_unchanged":true,
        "repeat_scope":"same verified signed MSIX through the full transaction and deployment; version advancement is covered by the 2.0.0/3.0.0 unit case",
        "download_scope":"local copy of cached official MSIX; production validation and commit unchanged"
    })).unwrap()).unwrap();
}
