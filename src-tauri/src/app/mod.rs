pub mod account;
pub mod appx_volume;
pub mod atomic_file;
pub mod capability_pack;
// These manager paths are compiled for macOS CI but active only in the Windows USB build.
#[cfg_attr(not(windows), allow(dead_code))]
pub mod ccswitch;
pub mod channels;
pub mod codex_agents_hint;
pub mod codex_config_guard;
#[cfg(test)]
pub(crate) mod codex_config_readback;
#[cfg(target_os = "windows")]
pub mod codex_entry_ipc;
pub mod codex_self_update;
#[cfg_attr(not(windows), allow(dead_code))]
pub mod codex_theme;
pub mod config_health;
pub mod diagnostics;
pub mod disk;
pub mod gateway_model_probe;
pub mod install_tx;
pub mod logging;
pub mod mac_update;
pub mod media_generation;
pub mod model_capability;
#[cfg_attr(not(windows), allow(dead_code))]
pub mod model_catalog;
pub mod model_diagnostics;
pub(crate) mod model_native_probe;
pub mod model_probe;
pub(crate) mod model_process;
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) mod model_selection_client;
pub mod msix_policy_tx;
pub mod offline_payload;
pub mod op_phase;
pub mod operation_outcome;
pub mod oplock;
pub mod paths;
pub mod provenance;
pub mod release_install;
pub mod runtime;
pub mod settings_store;
pub mod shell;
pub mod skill_repo_update;
pub mod staging;
pub mod uninstall_cleanup;
pub mod update_channel;
pub mod url_guard;
#[cfg_attr(not(windows), allow(dead_code))]
pub mod usb_bundle;
#[cfg_attr(not(windows), allow(dead_code))]
pub mod usb_guard;
pub mod usb_media_cleanup;
#[cfg_attr(not(windows), allow(dead_code))]
pub mod usb_revocation;
#[cfg_attr(not(windows), allow(dead_code))]
pub mod usb_revocation_cleanup;
pub mod win_elevation;
pub mod win_update;
pub mod window_mode;
