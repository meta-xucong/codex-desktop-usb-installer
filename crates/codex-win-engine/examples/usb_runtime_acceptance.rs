//! Opt-in developer acceptance driver. Not included in the installer.
//! Uses the production USB staging and launch APIs with an isolated root/HOME.
use codex_win_engine as engine;
use std::{env, error::Error, fs, path::PathBuf};

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = env::args_os().collect();
    let action = args.get(1).and_then(|s| s.to_str()).ok_or("missing action")?;
    let root = PathBuf::from(args.get(2).ok_or("missing isolated root")?);
    let install = root.join("apps/Codex");
    let home = root.join("home");
    match action {
        "install" => {
            let package = PathBuf::from(args.get(3).ok_or("missing verified MSIX")?);
            let shim = PathBuf::from(args.get(4).ok_or("missing entry shim")?);
            let signature = engine::verify_openai_authenticode(&package)?;
            if !signature.is_valid_openai() {
                return Err("MSIX OpenAI Authenticode verification failed".into());
            }
            fs::create_dir_all(&root)?;
            let trusted_shim = root.join("CodexEntryShim.exe");
            fs::copy(shim, &trusted_shim)?;
            let before = fs::read(home.join("config.toml")).ok();
            let mut options = engine::PortableInstallOptions::usb_offline();
            options.usb_entry_shim = Some(trusted_shim);
            let report = engine::install_portable_from_msix_with_observer_options(
                &package, &install, options, &mut |boundary| {
                    println!("boundary={boundary:?}");
                    Ok(())
                },
            )?;
            assert!(report.success);
            assert_eq!(before, fs::read(home.join("config.toml")).ok(), "application transaction changed HOME config");
            let version = engine::read_codex_app_version_from_install_root(&install)
                .ok_or("installed app version unavailable")?;
            println!("installed_version={version} home_config_unchanged=true");
        }
        "launch" => {
            let helper = PathBuf::from(args.get(3).ok_or("missing existing USB helper")?);
            let port: u16 = args.get(4).and_then(|s| s.to_str())
                .ok_or("missing loopback diagnostic port")?.parse()?;
            let profile = root.join("profile");
            let mut environment = vec![("OAI_SKY_WINDOWS_BIN".into(), helper.to_string_lossy().into_owned())];
            for (name, relative) in [("APPDATA", "Roaming"), ("LOCALAPPDATA", "Local"), ("USERPROFILE", "User"), ("HOME", "User")] {
                let path = profile.join(relative);
                fs::create_dir_all(&path)?;
                env::set_var(name, &path);
                // The production desktop-token launcher builds the desktop
                // user's environment; pass fixture isolation explicitly too.
                environment.push((name.into(), path.to_string_lossy().into_owned()));
            }
            environment.push(("ELECTRON_ENABLE_LOGGING".into(), "1".into()));
            environment.push(("ELECTRON_LOG_FILE".into(), root.join("electron.log").to_string_lossy().into_owned()));
            for name in ["CODEX_CLI_PATH", "CODEX_NODE", "CODEX_MCP_NODE_PATH"] {
                env::remove_var(name);
            }
            fs::create_dir_all(&home)?;
            let identity = engine::parse_appx_manifest_xml(&fs::read_to_string(install.join("AppxManifest.xml"))?)?;
            let installed = engine::InstalledWindowsCodex {
                app_version_unavailable: false,
                path: install.to_string_lossy().into_owned(),
                version: identity.version,
                arch: Some(identity.processor_architecture),
                source: "portable".into(),
                package_family_name: None,
                package_volume: None,
                installed_at: None,
            };
            let report = engine::launch_codex_with_options(&installed, engine::LaunchOptions {
                disable_codex_self_updates: true,
                usb_integrity_transaction: true,
                require_usb_entry_overlay: true,
                codex_home: Some(home),
                remote_debugging_port: Some(port),
                locale_override: Some("zh-CN".into()),
                portable_launch_policy: engine::PortableLaunchPolicy::UsbCompatible,
                environment,
            })?;
            println!("launch_mode={} usable_window_confirmed={}", report.launch_mode, report.usable_window_confirmed);
            if !report.usable_window_confirmed {
                return Err("runtime did not expose a usable window".into());
            }
        }
        "close" => {
            engine::close_codex_gracefully_for_root(20, &install)?;
            println!("isolated_runtime_closed=true");
        }
        _ => return Err("action must be install, launch or close".into()),
    }
    Ok(())
}
