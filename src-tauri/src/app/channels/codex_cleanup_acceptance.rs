// Real-file regression on an externally provisioned, isolated small NTFS VHD.
// No physical disk operations are performed by these tests.
fn cleanup_acceptance_install(
    root: &Path,
    block_backup: bool,
    crash_extraction: bool,
    evidence: &Path,
) -> (
    Result<codex_win_engine::PortableInstallReport, codex_win_engine::EngineError>,
    Vec<String>,
    Option<fs::File>,
) {
    use crate::app::install_tx::ActiveInstallTx;
    use codex_win_engine::{PortableBoundary, PortableInstallOptions};
    use std::os::windows::fs::OpenOptionsExt;
    let mut tx = None::<ActiveInstallTx>;
    let mut locked = None;
    let mut observer = |boundary| {
        let result = match boundary {
            PortableBoundary::BeforePrepare {
                install_root,
                payload,
                backup,
                had_previous,
            } => {
                let active = ActiveInstallTx::begin_portable_preparation(
                    &install_root,
                    &payload,
                    &backup,
                    had_previous,
                )?;
                if crash_extraction {
                    fs::write(
                        evidence.join("extraction-paths.json"),
                        serde_json::to_vec(&serde_json::json!({"payload":payload,"backup":backup}))
                            .unwrap(),
                    )
                    .map_err(|e| AppError::Internal(e.to_string()))?;
                }
                tx = Some(active);
                Ok(())
            }
            PortableBoundary::BeforeMoveOld { .. } => tx.as_mut().unwrap().mark_prepared(),
            PortableBoundary::AfterMoveOld {
                backup,
                had_previous,
                ..
            } => {
                tx.as_mut().unwrap().mark_old_moved()?;
                if block_backup && had_previous {
                    // Acquire after the directory rename; obstruct deletion, not the swap.
                    locked = Some(
                        fs::OpenOptions::new()
                            .read(true)
                            .share_mode(1)
                            .open(backup.join("resources/app.asar"))
                            .map_err(|e| AppError::Internal(e.to_string()))?,
                    );
                }
                Ok(())
            }
            PortableBoundary::BeforeMoveNew { .. } => Ok(()),
            PortableBoundary::AfterMoveNew { .. } => tx.as_mut().unwrap().mark_new_installed(),
            PortableBoundary::BeforeRollback { .. } => tx.as_mut().unwrap().mark_rollback_started(),
            PortableBoundary::RollbackCompleted { .. } => tx.take().unwrap().mark_rolled_back(),
        };
        result
    };
    let progress = |value: codex_win_engine::PortableExtractionProgress| {
        if crash_extraction && value.completed_bytes >= 160 * 1024 * 1024 {
            fs::write(
                evidence.join("crashed-extraction-bytes.txt"),
                value.completed_bytes.to_string(),
            )
            .unwrap();
            // Exit without Rust destructors/installer cleanup, matching abrupt process death.
            std::process::exit(91);
        }
        Ok(())
    };
    let result = codex_win_engine::install_portable_from_msix_with_observer_options_and_progress(
        &root.join("payload/OpenAI.Codex_x64.msix"),
        &root.join("apps/Codex"),
        PortableInstallOptions {
            usb_entry_shim: Some(root.join("CodexEntryShim.exe")),
            ..PortableInstallOptions::usb_offline()
        },
        &mut |boundary| {
            observer(boundary)
                .map_err(|e: AppError| codex_win_engine::EngineError::Io(e.to_string()))
        },
        Some(&progress),
    );
    let warnings = if result.is_ok() {
        tx.take().unwrap().succeed()
    } else {
        Vec::new()
    };
    (result, warnings, locked)
}

#[test]
#[ignore = "child only: requires explicitly isolated test volume and manager data directory"]
fn real_small_usb_extraction_crash_child() {
    assert_eq!(std::env::var("USB_CLEANUP_CRASH_CHILD").unwrap(), "1");
    let root = PathBuf::from(std::env::var_os("CODEX_8GB_TEST_ROOT").unwrap());
    let evidence = PathBuf::from(std::env::var_os("CODEX_8GB_EVIDENCE").unwrap());
    let _ = cleanup_acceptance_install(&root, false, true, &evidence);
    panic!("expected abrupt exit inside real extraction");
}

#[test]
#[ignore = "requires a verified isolated <=7.49GiB NTFS volume, full release fixture and smoke data directory"]
fn real_small_usb_continuous_update_and_cleanup_recovery() {
    use crate::app::install_tx::{self, InstallTransaction, InstallTxStep};
    let root =
        PathBuf::from(std::env::var_os("CODEX_8GB_TEST_ROOT").expect("isolated bundle root"));
    let package = PathBuf::from(std::env::var_os("CODEX_8GB_NEW_MSIX").unwrap());
    let evidence = PathBuf::from(std::env::var_os("CODEX_8GB_EVIDENCE").unwrap());
    let data = crate::app::paths::data_dir().unwrap();
    assert!(data
        .file_name()
        .unwrap()
        .to_string_lossy()
        .starts_with("codex-app-manager-smoke-"));
    assert_eq!(
        data,
        PathBuf::from(std::env::var_os("CAM_PACKAGED_SMOKE_DATA_DIR").unwrap())
            .canonicalize()
            .unwrap()
    );
    let install = root.join("apps/Codex");
    assert!(!install.exists(), "fresh complete release fixture required");
    // User files live beside the signed bundle, not inside its manifest-owned root.
    let user_data = root.parent().unwrap().join("usb-update-cleanup-1116-user-keep.txt");
    fs::write(&user_data, "user files remain untouched").unwrap();
    let settings = data.join("settings-sentinel.toml");
    fs::write(
        &settings,
        "model = 'gpt-6.1-sol'\n[desktop]\nconversationDetailMode = 'STEPS_COMMANDS'\n",
    )
    .unwrap();
    let settings_before = fs::read(&settings).unwrap();
    let observations = std::cell::RefCell::new(Vec::new());
    let observe = |phase: &str| {
        let free = crate::app::disk::available_space(&root).unwrap().unwrap();
        println!("cleanup capacity phase={phase} free={free}");
        observations
            .borrow_mut()
            .push(serde_json::json!({"phase":phase,"free":free}));
    };
    let pending = || -> Vec<InstallTransaction> {
        let dir = install_tx::transactions_dir().unwrap();
        if !dir.exists() {
            return vec![];
        }
        fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.extension().and_then(|v| v.to_str()) == Some("json"))
            .map(|p| InstallTransaction::load_from_path(&p).unwrap())
            .collect()
    };
    assert!(pending().is_empty());
    observe("complete-release-copied");
    let (result, warnings, _) = cleanup_acceptance_install(&root, false, false, &evidence);
    result.unwrap();
    assert!(warnings.is_empty());
    assert!(pending().is_empty());
    let baseline_free = crate::app::disk::available_space(&root).unwrap().unwrap();
    observe("fresh-install-complete");

    let crashed = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "app::channels::codex::tests::real_small_usb_extraction_crash_child",
            "--ignored",
            "--nocapture",
        ])
        .env("USB_CLEANUP_CRASH_CHILD", "1")
        .status()
        .unwrap();
    assert_eq!(crashed.code(), Some(91));
    let journals = pending();
    assert_eq!(journals.len(), 1);
    assert_eq!(journals[0].step, InstallTxStep::Extracting);
    assert!(Path::new(&journals[0].new_path).is_dir());
    observe("abrupt-extraction-exit");
    install_tx::recover_portable_before_update(&install).unwrap();
    assert!(pending().is_empty());
    assert!(!Path::new(&journals[0].new_path).parent().unwrap().exists());
    observe("extraction-recovered-before-download");

    // Preserve the established rollback behavior and prove the original tree wins.
    fs::write(install.join("rollback-sentinel.txt"), "old program").unwrap();
    codex_win_engine::inject_portable_fault(Some(codex_win_engine::PortableFault::OnMoveNew));
    assert!(cleanup_acceptance_install(&root, false, false, &evidence)
        .0
        .is_err());
    assert_eq!(
        fs::read_to_string(install.join("rollback-sentinel.txt")).unwrap(),
        "old program"
    );
    assert!(pending().is_empty());
    observe("failed-swap-rolled-back");

    let identity = read_msix_identity(&package).unwrap();
    let version = codex_win_engine::read_codex_app_version_from_msix(&package).unwrap();
    let hash = sha256_file(&package).unwrap();
    let moniker = codex_win_engine::canonical_codex_package_moniker(&identity).unwrap();
    let checksums = format!("{hash}  {moniker}.Msix\n");
    let mut fresh = fake_resolved_release();
    fresh.app_version = version.clone();
    fresh.identity.app_version = version.clone();
    fresh.identity.msix_version = identity.version;
    fresh.identity.package_moniker = moniker;
    fresh.identity.github_digest = Some(hash.clone());
    fresh.msix.expected_sha256 = Some(hash);
    fresh.msix.size = fs::metadata(&package).unwrap().len();
    fresh.identity.msix_size = fresh.msix.size;
    let mut transport = production_apply_transport(&NetworkConfig::direct(), false);
    transport.download = Box::new(move |_, dest, _, progress| {
        progress(fs::copy(&package, dest).map_err(|e| e.to_string())?);
        Ok(())
    });
    transport.fetch_text = Box::new(move |_| Ok(checksums.clone()));
    for pass in 1..=2 {
        install_tx::recover_portable_before_update(&install).unwrap();
        let (local, tx) = prepare_transaction(&root);
        let expected = UpdateInfo {
            current_version: local.current_version.clone(),
            latest_version: version.clone(),
            payload: codex_update_payload(&root, &local, &fresh.identity).unwrap(),
            ..fake_update_info(&root, &local)
        };
        apply_transaction_with_transport(
            &root,
            &tx,
            &local,
            &fresh,
            &expected,
            &|_, _| observe("payload-downloaded"),
            &transport,
        )
        .unwrap();
        assert!(!tx.exists());
        observe("payload-committed-old-msix-released");
        let (result, warnings, lock) =
            cleanup_acceptance_install(&root, pass == 1, false, &evidence);
        let report = result.unwrap();
        if pass == 1 {
            let backup = report
                .backup_path
                .expect("real sharing violation must leave old directory");
            assert!(!warnings.is_empty());
            assert_eq!(pending().len(), 1);
            assert_eq!(pending()[0].step, InstallTxStep::Completed);
            assert!(Path::new(&backup).exists());
            observe("old-directory-delete-failed-journal-retained");
            assert!(install_tx::recover_portable_before_update(&install).is_err());
            assert_eq!(pending().len(), 1);
            drop(lock);
            install_tx::recover_portable_before_update(&install).unwrap();
            assert!(!Path::new(&backup).exists());
            observe("old-directory-recovered-before-next-update");
        } else {
            assert!(warnings.is_empty());
        }
        assert!(pending().is_empty());
        assert!(!root.join("apps/.codex-app-manager-staging").exists());
        assert!(!fs::read_dir(root.join("apps")).unwrap().any(|e| e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with("Codex.rollback-")));
        assert_eq!(
            fs::read_dir(root.join("payload/_backup")).unwrap().count(),
            0
        );
        assert_eq!(
            codex_win_engine::read_codex_app_version_from_install_root(&install).unwrap(),
            version
        );
        assert_eq!(fs::read(&settings).unwrap(), settings_before);
        assert_eq!(
            fs::read_to_string(&user_data).unwrap(),
            "user files remain untouched"
        );
        observe(&format!("continuous-update-{pass}-complete"));
    }
    let final_free = crate::app::disk::available_space(&root).unwrap().unwrap();
    assert!(
        final_free + 1024 * 1024 >= baseline_free,
        "successive updates must not accumulate old trees"
    );
    fs::write(evidence.join("real-small-usb-cleanup-result.json"), serde_json::to_vec_pretty(&serde_json::json!({
        "success":true,"root":root,"version":version,"baseline_free":baseline_free,"final_free":final_free,
        "observations":observations.into_inner(),"extraction_exit_code":91,"old_directory_real_sharing_violation":true,
        "user_data_unchanged":true,"settings_sentinel_unchanged":true,"pending_journals":0,
        "scope":"two full transactions using the same original signed MSIX; local copy replaces download only; production validation/commit unchanged; no future-size guarantee"
    })).unwrap()).unwrap();
}
