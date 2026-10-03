use std::fs;
use std::io::{self, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::app_version::read_codex_app_version_from_install_root;
use crate::msix::{parse_appx_manifest_xml, MsixIdentity};
use crate::process::{
    hidden_command, spawn_and_require_liveness, LivenessResult, PORTABLE_LIVENESS_WINDOW,
};
#[cfg(windows)]
use crate::process::{run_capturing, RunLimits};
use crate::EngineError;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortableInstallReport {
    pub success: bool,
    pub install_root: String,
    pub executable_path: Option<String>,
    pub version: String,
    pub backup_path: Option<String>,
    pub shortcut_created: bool,
    pub uninstall_entry_created: bool,
    pub relaunched: bool,
    pub message: String,
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortableUninstallReport {
    pub success: bool,
    #[serde(default)]
    pub partial: bool,
    pub install_root: String,
    pub removed_files: bool,
    pub removed_shortcut: bool,
    pub removed_uninstall_entry: bool,
    pub purged_user_data: bool,
    pub message: String,
    pub notes: Vec<String>,
}

/// Controls the side effects around a portable payload swap.
///
/// The ordinary desktop manager keeps the historical behaviour: close the
/// existing process, launch-probe the new tree, optionally relaunch it, and
/// publish shell metadata.  The USB product deliberately uses a stricter
/// profile: the verified payload is installed on removable media without
/// starting Codex before CCSwitch is configured, and without publishing a
/// direct Start Menu/uninstall entry that could bypass the USB manager.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PortableInstallOptions {
    pub close_running: bool,
    pub launch_health_check: bool,
    pub relaunch: bool,
    pub create_shell_metadata: bool,
}

impl PortableInstallOptions {
    fn legacy(manage_process: bool, relaunch: bool) -> Self {
        Self {
            close_running: manage_process,
            launch_health_check: manage_process,
            relaunch: manage_process && relaunch,
            create_shell_metadata: true,
        }
    }

    pub const fn usb_offline() -> Self {
        Self {
            // The app layer closes the freshly detected previous install before
            // entering the transaction, including an old MSIX registration.
            close_running: false,
            // Do not start Codex until CCSwitch/API configuration is complete.
            // Static payload integrity is still checked below.
            launch_health_check: false,
            relaunch: false,
            // A direct shortcut would bypass the USB lease/watchdog. The fixed
            // bundle remains the supported launcher and uninstaller.
            create_shell_metadata: false,
        }
    }
}

struct PreparedPortable {
    payload_dir: PathBuf,
    identity: MsixIdentity,
}

/// Progress reported while the verified MSIX archive is being unpacked into
/// the transactional portable payload.  The byte total is calculated from the
/// ZIP central directory before the first write, so the UI can show a real
/// percentage even when the destination is a slow removable drive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PortableExtractionProgress {
    pub completed_bytes: u64,
    pub total_bytes: u64,
    pub files_completed: u64,
}

pub type PortableExtractionProgressObserver<'a> =
    dyn Fn(PortableExtractionProgress) -> Result<(), EngineError> + 'a;

fn io_err(context: &str, err: impl ToString) -> EngineError {
    EngineError::Io(format!("{context}: {}", err.to_string()))
}

const EXTRACT_FILE_CREATE_RETRIES: usize = 3;
const EXTRACTION_COPY_BUFFER_BYTES: usize = 1024 * 1024;
const EXTRACTION_PROGRESS_BYTES: u64 = 4 * 1024 * 1024;
const EXTRACTION_PROGRESS_INTERVAL: Duration = Duration::from_millis(200);

// Codex's packaged Electron shell queries the Windows MSIX package identity
// during bootstrap and initializes the in-app updater before it starts the
// actual UI.  A payload extracted from MSIX and launched from a USB directory
// has no package identity, so the native call can throw
// `APPMODEL_ERROR_NO_PACKAGE` and leave the user with a black window and a
// "ChatGPT failed to start" dialog.  The manager owns updates for a portable
// tree; package-dependent updater bootstrap must therefore be skipped for this
// launch form.
//
// Keep the replacement the same length as the minified call.  ASAR stores file
// contents at fixed offsets, so an in-place patch preserves the archive header
// and every other file offset without rebuilding (or re-signing) the payload.
const CODEX_ASAR_UPDATER_MARKER: &[u8] = b"getCurrentPackageFamily";
const CODEX_ASAR_UPDATER_CALLS: &[&[u8]] = &[
    b"await o.initialize();",
    // Keep a few minifier-safe fallbacks for upstream builds that rename the
    // destructured sparkle manager binding while leaving the bootstrap shape
    // unchanged.
    b"await t.initialize();",
    b"await e.initialize();",
    b"await i.initialize();",
];
const CODEX_ASAR_UPDATER_PATCH_PREFIX: &[u8] = b"await 0/*USB*/;";
const CODEX_ASAR_PACKAGE_FAMILY_CALL: &[u8] = b"OT()?.getCurrentPackageFamily()";
const CODEX_ASAR_PACKAGE_FAMILY_PATCH_PREFIX: &[u8] = b"void 0/*USB*/";

// The bundled Electron bridge starts the native Codex app-server with
// Node's `child_process.spawn`.  On Windows `resources/codex.exe` is a
// console-subsystem binary, so the spawn must carry `windowsHide: true` at
// the actual child-creation boundary.  `stdio: "pipe"` is equivalent to the
// original three-element pipe array and is one byte shorter than the complete
// replacement, which lets us patch the minified ASAR in place without moving
// offsets or invalidating the signed MSIX archive before extraction.
const CODEX_ASAR_CONSOLE_SPAWN_CALLS: &[(&[u8], &[u8])] = &[
    // Codex 26.917 adds an explicit cwd after the inherited environment.
    // Keep the replacement fixed-width so the extracted ASAR remains a
    // byte-for-byte layout-preserving patch on the removable install.
    (
        b"stdio:[`pipe`,`pipe`,`pipe`],env:this.options.env,cwd:this.options.cwd",
        b"stdio:\"pipe\",windowsHide:!0,env:this.options.env,cwd:this.options.cwd",
    ),
    (
        b"stdio:[\"pipe\",\"pipe\",\"pipe\"],env:this.options.env,cwd:this.options.cwd",
        b"stdio:\"pipe\",windowsHide:!0,env:this.options.env,cwd:this.options.cwd",
    ),
    (
        b"stdio:['pipe','pipe','pipe'],env:this.options.env,cwd:this.options.cwd",
        b"stdio:\"pipe\",windowsHide:!0,env:this.options.env,cwd:this.options.cwd",
    ),
    (
        b"stdio:[`pipe`,`pipe`,`pipe`],env:this.options.env",
        b"stdio:\"pipe\",windowsHide:!0,env:this.options.env",
    ),
    (
        b"stdio:[\"pipe\",\"pipe\",\"pipe\"],env:this.options.env",
        b"stdio:\"pipe\",windowsHide:!0,env:this.options.env",
    ),
    (
        b"stdio:['pipe','pipe','pipe'],env:this.options.env",
        b"stdio:\"pipe\",windowsHide:!0,env:this.options.env",
    ),
];
const CODEX_ASAR_CONSOLE_SPAWN_SENTINEL: &[u8] =
    b"stdio:\"pipe\",windowsHide:!0,env:this.options.env";
const CODEX_ASAR_CONSOLE_SPAWN_CWD_SENTINEL: &[u8] =
    b"stdio:\"pipe\",windowsHide:!0,env:this.options.env,cwd:this.options.cwd";

// Codex's open-in-targets helper wraps .cmd/.bat launches in cmd.exe.  The
// helper used to pass only `stdio` and `env`, so Windows could allocate a
// console for cmd.exe before the process-tree watcher had a chance to hide it.
// Keep the edit fixed-width while retaining Codex's Windows command resolver.
// That resolver applies PATH/PATHEXT and is required for custom targets whose
// configured command is a bare `.cmd`/`.bat` name. `windowsHide:1` is
// intentional; Node treats any truthy value as enabled and the one-character
// form fits the in-place extracted-ASAR patch.
const CODEX_ASAR_GENERIC_SPAWN_CALLS: &[(&[u8], &[u8])] = &[
    // This is the complete minified bridge around the generic Zi() helper.
    // Keep the replacement no wider than the source bytes so the extracted
    // ASAR can be repaired without rebuilding its header and file offsets.
    // `Ane` still receives the resolved command, the normalized environment's
    // ComSpec (with its existing `cmd.exe` fallback), and the original stdio
    // expression.  Calling the imported spawn function directly is equivalent
    // here: Node's child_process.spawn does not depend on a receiver object.
    (
        b"o=n.Bi(r?.env??process.env),s=Ane(process.platform,n.qn.resolve(e,{env:o})??e,t,process.env.ComSpec),c=(0,P.spawn)(s.command,s.args,{stdio:r?.stdin==null?`ignore`:[`pipe`,`ignore`,`ignore`],env:o})",
        b"o=n.Bi(r?.env??process.env),s=Ane(process.platform,n.qn.resolve(e,{env:o})??e,t,o.ComSpec),c=P.spawn(s.command,s.args,{windowsHide:1,stdio:r?.stdin==null?`ignore`:[`pipe`,`ignore`,`ignore`],env:o})",
    ),
];
const CODEX_ASAR_GENERIC_SPAWN_SENTINEL: &[u8] =
    b"o=n.Bi(r?.env??process.env),s=Ane(process.platform,n.qn.resolve(e,{env:o})??e,t,o.ComSpec),c=P.spawn(s.command,s.args,{windowsHide:1,stdio:r?.stdin==null?`ignore`:[`pipe`,`ignore`,`ignore`],env:o})";

// A few Codex sidecars are shipped as Windows console-subsystem PE files.
// Node's `windowsHide` option covers children created through the patched
// app-server bridge, but it cannot change a sidecar that creates its own
// console before that bridge is reached.  The USB payload is an extracted,
// manager-owned copy (the source MSIX remains Authenticode-verified), so the
// following known sidecars can be made GUI-subsystem at install/launch
// time.  The edit changes only the two-byte PE OptionalHeader.Subsystem field,
// preserves file length and is idempotent.
const CODEX_CONSOLE_SUBSYSTEM_EXECUTABLES: &[&str] = &[
    "codex.exe",
    "codex-code-mode-host.exe",
    "codex-windows-sandbox-service.exe",
    "codex-windows-sandbox-setup.exe",
    "rg.exe",
    "app.asar.unpacked/node_modules/node-pty/build/Release/winpty-agent.exe",
    "cua_node/bin/node.exe",
    "cua_node/bin/node_repl.exe",
    "cua_node/bin/node_modules/%40oai/cua/bin/windows/codex-computer-use.exe",
    "cua_node/bin/node_modules/%40oai/sky/bin/windows/codex-computer-use.exe",
    "cua_node/bin/node_modules/%40oai/sky/bin/windows/swift/x64/codex-computer-use-swift.exe",
    "plugins/openai-bundled/plugins/chrome/extension-host/windows/x64/extension-host.exe",
    "plugins/openai-bundled/plugins/latex/bin/tectonic.exe",
];

// The bundled codex_app MCP server ships as a .cmd launcher. Starting that
// launcher through cmd.exe creates a visible conhost before Codex's native
// process hiding logic can run, including when the server restarts on a new
// conversation. Invoke the already-bundled, GUI-subsystem CUA Node directly.
const CODEX_APP_TOOLS_MCP_CONFIG: &str = "plugins/openai-bundled/plugins/codex-app-tools/.mcp.json";
const CODEX_APP_TOOLS_MCP_NODE_COMMAND: &str = r"..\..\..\..\cua_node\bin\node.exe";
const CODEX_APP_TOOLS_MCP_OLD_COMMAND: &str = "cmd.exe";
const CODEX_APP_TOOLS_MCP_OLD_ARGS: &[&str] = &[
    "/d",
    "/s",
    "/c",
    "call",
    "./scripts/launch_codex_app_tools_mcp.cmd",
    "./server.mjs",
];
const CODEX_APP_TOOLS_MCP_DIRECT_ARGS: &[&str] = &["./server.mjs"];

// Codex ships the translated webview bundles, but several desktop releases
// gate loading them behind a remote `enable_i18n` flag whose local fallback is
// false.  The USB build must not depend on that flag being available (the
// machine may be offline), so enable the bundled translations in-place.  Every
// replacement is exactly the same width as the original minified expression;
// this keeps the ASAR offsets valid and avoids rebuilding/re-signing it.
const CODEX_ASAR_I18N_CALLS: &[(&[u8], &[u8])] = &[
    (b"get(`enable_i18n`, false)", b"get(`enable_i18n`, true )"),
    (b"get(`enable_i18n`,false)", b"get(`enable_i18n`,true )"),
    (
        b"get(\"enable_i18n\", false)",
        b"get(\"enable_i18n\", true )",
    ),
    (b"get(\"enable_i18n\",false)", b"get(\"enable_i18n\",true )"),
    (b"get('enable_i18n', false)", b"get('enable_i18n', true )"),
    (b"get('enable_i18n',false)", b"get('enable_i18n',true )"),
    (b"get(`enable_i18n`,!1)", b"get(`enable_i18n`,!0)"),
    (b"get(\"enable_i18n\",!1)", b"get(\"enable_i18n\",!0)"),
    (b"get('enable_i18n',!1)", b"get('enable_i18n',!0)"),
];
fn requests_chinese_codex_locale(locale: Option<&str>) -> bool {
    locale
        .map(str::trim)
        .filter(|locale| !locale.is_empty())
        .is_some_and(|locale| locale.to_ascii_lowercase().starts_with("zh"))
}

fn find_bytes_in_file(path: &Path, needle: &[u8]) -> io::Result<Option<u64>> {
    if needle.is_empty() {
        return Ok(Some(0));
    }
    let file = fs::File::open(path)?;
    let mut reader = BufReader::with_capacity(EXTRACTION_COPY_BUFFER_BYTES, file);
    let mut carry = Vec::with_capacity(needle.len().saturating_sub(1));
    let mut chunk = vec![0_u8; EXTRACTION_COPY_BUFFER_BYTES];
    let mut file_offset = 0_u64;
    loop {
        let read = reader.read(&mut chunk)?;
        if read == 0 {
            return Ok(None);
        }
        let carry_len = carry.len();
        let mut haystack = Vec::with_capacity(carry_len + read);
        haystack.extend_from_slice(&carry);
        haystack.extend_from_slice(&chunk[..read]);
        if let Some(index) = haystack
            .windows(needle.len())
            .position(|window| window == needle)
        {
            return Ok(Some(
                file_offset.saturating_sub(carry_len as u64) + index as u64,
            ));
        }
        let keep = needle.len().saturating_sub(1).min(haystack.len());
        carry.clear();
        carry.extend_from_slice(&haystack[haystack.len() - keep..]);
        file_offset = file_offset.saturating_add(read as u64);
    }
}

fn patch_asar_bytes_in_place(
    app_asar: &Path,
    offset: u64,
    replacement: &[u8],
) -> Result<(), EngineError> {
    let mut last_error = None;
    for attempt in 0..=EXTRACT_FILE_CREATE_RETRIES {
        let result = (|| {
            let mut file = fs::OpenOptions::new().write(true).open(app_asar)?;
            file.seek(SeekFrom::Start(offset))?;
            file.write_all(replacement)?;
            file.flush()?;
            Ok::<(), io::Error>(())
        })();
        match result {
            Ok(()) => return Ok(()),
            Err(error)
                if attempt < EXTRACT_FILE_CREATE_RETRIES
                    && matches!(
                        error.kind(),
                        io::ErrorKind::Interrupted
                            | io::ErrorKind::PermissionDenied
                            | io::ErrorKind::WouldBlock
                    ) =>
            {
                last_error = Some(error);
                thread::sleep(extraction_file_retry_delay(attempt));
            }
            Err(error) => {
                return Err(io_err("write Codex app.asar portable startup patch", error));
            }
        }
    }
    Err(io_err(
        "write Codex app.asar portable startup patch",
        last_error.expect("retry loop must retain the last error"),
    ))
}

fn patch_first_fixed_width_call(
    app_asar: &Path,
    call: &[u8],
    replacement_prefix: &[u8],
    label: &str,
) -> Result<bool, EngineError> {
    let Some(offset) = find_bytes_in_file(app_asar, call)
        .map_err(|error| EngineError::Io(format!("scan Codex app.asar for {label}: {error}")))?
    else {
        return Ok(false);
    };
    if replacement_prefix.len() > call.len() {
        return Err(EngineError::Msix(format!(
            "portable Codex startup patch for {label} is wider than the original call"
        )));
    }
    let mut replacement = vec![b' '; call.len()];
    replacement[..replacement_prefix.len()].copy_from_slice(replacement_prefix);
    patch_asar_bytes_in_place(app_asar, offset, &replacement)?;
    log::info!(
        "portable Codex startup compatibility patch applied path={} offset={} reason={label}",
        app_asar.display(),
        offset
    );
    Ok(true)
}

fn patch_codex_console_spawn(app_asar: &Path) -> Result<bool, EngineError> {
    // Scan the archive once.  Reopening and rescanning a 300+ MB ASAR for
    // every known shape made a slow removable-drive launch needlessly costly;
    // collect fixed-width edits from the original bytes, then apply only the
    // small offsets that matched.
    let bytes = fs::read(app_asar)
        .map_err(|error| io_err("read Codex app.asar for console-silence patch", error))?;
    let mut edits = Vec::<(usize, Vec<u8>, &'static str)>::new();
    for (label, calls) in [
        ("app-server-console-window", CODEX_ASAR_CONSOLE_SPAWN_CALLS),
        (
            "generic-command-console-window",
            CODEX_ASAR_GENERIC_SPAWN_CALLS,
        ),
    ] {
        for (call, replacement) in calls {
            if replacement.len() > call.len() {
                return Err(EngineError::Msix(
                    "portable Codex console patch is wider than the original call".to_string(),
                ));
            }
            let mut search_from = 0_usize;
            while search_from.saturating_add(call.len()) <= bytes.len() {
                let Some(relative) = bytes[search_from..]
                    .windows(call.len())
                    .position(|window| window == *call)
                else {
                    break;
                };
                let offset = search_from + relative;
                // Some of the shorter `env` forms are byte prefixes of the
                // newer `env,cwd` forms.  Keep the longest shape when their
                // ranges overlap; otherwise the second edit would overwrite
                // the beginning of the more specific replacement and leave
                // the cwd call unpatched.
                let overlaps_existing = edits.iter().any(|(existing_offset, existing, _)| {
                    let existing_end = existing_offset.saturating_add(existing.len());
                    let new_end = offset.saturating_add(call.len());
                    offset < existing_end && *existing_offset < new_end
                });
                if !overlaps_existing {
                    let mut fixed = vec![b' '; call.len()];
                    fixed[..replacement.len()].copy_from_slice(replacement);
                    edits.push((offset, fixed, label));
                }
                search_from = offset.saturating_add(call.len());
            }
        }
    }

    if !edits.is_empty() {
        for (offset, replacement, label) in edits {
            patch_asar_bytes_in_place(app_asar, offset as u64, &replacement)?;
            log::info!(
                "portable Codex startup compatibility patch applied path={} offset={} reason={label}",
                app_asar.display(),
                offset
            );
        }
        return Ok(true);
    }

    let has_known_patch = [
        CODEX_ASAR_CONSOLE_SPAWN_SENTINEL,
        CODEX_ASAR_CONSOLE_SPAWN_CWD_SENTINEL,
        CODEX_ASAR_GENERIC_SPAWN_SENTINEL,
    ]
    .iter()
    .any(|needle| bytes.windows(needle.len()).any(|window| window == *needle));
    if !has_known_patch {
        log::warn!(
            "portable Codex console spawn did not match a known console-silence patch shape path={}",
            app_asar.display()
        );
    }
    Ok(false)
}

fn read_u16_le(bytes: &[u8], offset: usize) -> Option<u16> {
    bytes
        .get(offset..offset.saturating_add(2))
        .map(|value| u16::from_le_bytes([value[0], value[1]]))
}

fn read_u32_le(bytes: &[u8], offset: usize) -> Option<u32> {
    bytes
        .get(offset..offset.saturating_add(4))
        .map(|value| u32::from_le_bytes([value[0], value[1], value[2], value[3]]))
}

fn is_regular_non_reparse_file(path: &Path) -> Result<bool, EngineError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(io_err("inspect Codex console sidecar", error)),
    };
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Ok(false);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;

        // FILE_ATTRIBUTE_REPARSE_POINT.  symlink_metadata() does not follow a
        // junction, but Windows still reports a reparse-point file as a
        // regular file in some filesystem configurations.
        if metadata.file_attributes() & 0x400 != 0 {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Change one known extracted Codex sidecar from Windows CUI (3) to GUI (2).
///
/// Invalid or non-PE files are left untouched because an upstream build may
/// replace a sidecar with a script/shim.  A real I/O or write failure remains
/// fatal: continuing would reintroduce the startup black-console regression
/// while reporting a successful portable preparation.
fn patch_console_subsystem_executable(path: &Path) -> Result<(bool, Option<u16>), EngineError> {
    if !is_regular_non_reparse_file(path)? {
        return Ok((false, None));
    }
    let bytes = fs::read(path).map_err(|error| io_err("read Codex console sidecar", error))?;
    if bytes.len() < 0x40 || &bytes[..2] != b"MZ" {
        log::warn!(
            "portable Codex sidecar is not a PE image; leaving unchanged path={}",
            path.display()
        );
        return Ok((false, None));
    }
    let nt_offset = read_u32_le(&bytes, 0x3c).map(|value| value as usize);
    let Some(nt_offset) = nt_offset else {
        return Ok((false, None));
    };
    let Some(signature) = bytes.get(nt_offset..nt_offset.saturating_add(4)) else {
        return Ok((false, None));
    };
    if signature != b"PE\0\0" {
        return Ok((false, None));
    }
    let optional_size = read_u16_le(&bytes, nt_offset.saturating_add(20))
        .map(usize::from)
        .unwrap_or(0);
    let optional_offset = nt_offset.saturating_add(24);
    let subsystem_offset = optional_offset.saturating_add(68);
    if optional_size < 70
        || read_u16_le(&bytes, optional_offset).is_none()
        || subsystem_offset.saturating_add(2) > bytes.len()
        || subsystem_offset.saturating_add(2) > optional_offset.saturating_add(optional_size)
    {
        return Ok((false, None));
    }

    let subsystem = read_u16_le(&bytes, subsystem_offset).unwrap_or_default();
    match subsystem {
        2 => Ok((false, Some(2))),
        3 => {
            let mut file = fs::OpenOptions::new()
                .write(true)
                .open(path)
                .map_err(|error| io_err("open Codex console sidecar for patch", error))?;
            file.seek(SeekFrom::Start(subsystem_offset as u64))
                .map_err(|error| io_err("seek Codex console sidecar subsystem", error))?;
            file.write_all(&2_u16.to_le_bytes())
                .map_err(|error| io_err("write Codex console sidecar subsystem", error))?;
            file.flush()
                .map_err(|error| io_err("flush Codex console sidecar subsystem", error))?;
            log::info!(
                "portable Codex console sidecar subsystem patched path={} old=3 new=2 reason=console-subsystem-suppression",
                path.display()
            );
            Ok((true, Some(2)))
        }
        other => {
            log::debug!(
                "portable Codex sidecar has unsupported PE subsystem; leaving unchanged path={} subsystem={other}",
                path.display()
            );
            Ok((false, Some(other)))
        }
    }
}

fn patch_console_subsystem_executables(
    install_root: &Path,
) -> Result<(bool, Option<u16>), EngineError> {
    let resources = install_root.join("resources");
    let mut patched = false;
    let mut cua_node_subsystem = None;
    for name in CODEX_CONSOLE_SUBSYSTEM_EXECUTABLES {
        let path = resources.join(name);
        if is_regular_non_reparse_file(&path)? {
            let (sidecar_patched, subsystem) = patch_console_subsystem_executable(&path)?;
            patched |= sidecar_patched;
            if *name == "cua_node/bin/node.exe" {
                cua_node_subsystem = subsystem;
            }
        }
    }
    Ok((patched, cua_node_subsystem))
}

fn mcp_server_mut<'a>(
    value: &'a mut serde_json::Value,
) -> Option<&'a mut serde_json::Map<String, serde_json::Value>> {
    value.get_mut("mcpServers")?.as_object_mut()
}

fn ensure_plain_directory_tree(root: &Path, target_parent: &Path) -> Result<(), EngineError> {
    let relative = target_parent
        .strip_prefix(root)
        .map_err(|_| EngineError::Install("Codex MCP 配置路径超出便携安装目录".to_string()))?;
    let mut current = root.to_path_buf();
    for component in relative.components() {
        current.push(component.as_os_str());
        let metadata = fs::symlink_metadata(&current)
            .map_err(|error| io_err("inspect Codex MCP configuration directory", error))?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(EngineError::Install(
                "Codex MCP 配置目录包含非普通目录，拒绝修改".to_string(),
            ));
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            if metadata.file_attributes() & 0x400 != 0 {
                return Err(EngineError::Install(
                    "Codex MCP 配置目录包含 Windows 重解析点，拒绝修改".to_string(),
                ));
            }
        }
    }
    Ok(())
}

fn replace_file_atomically(path: &Path, contents: &[u8]) -> Result<(), EngineError> {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("mcp.json");
    let temporary = path.with_file_name(format!(
        ".{file_name}.codex-usb-{}.tmp",
        uuid::Uuid::new_v4()
    ));
    let write_result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|error| io_err("create temporary Codex MCP configuration", error))?;
        file.write_all(contents)
            .and_then(|()| file.sync_all())
            .map_err(|error| io_err("write temporary Codex MCP configuration", error))?;
        drop(file);

        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStrExt;
            use windows_sys::Win32::Storage::FileSystem::{
                MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
            };
            let source: Vec<u16> = temporary.as_os_str().encode_wide().chain(Some(0)).collect();
            let destination: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
            let result = unsafe {
                MoveFileExW(
                    source.as_ptr(),
                    destination.as_ptr(),
                    MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
                )
            };
            if result == 0 {
                return Err(io_err(
                    "atomically replace Codex MCP configuration",
                    io::Error::last_os_error(),
                ));
            }
        }
        #[cfg(not(windows))]
        fs::rename(&temporary, path)
            .map_err(|error| io_err("atomically replace Codex MCP configuration", error))?;
        Ok(())
    })();
    if write_result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    write_result
}

fn patch_codex_app_tools_mcp_config(
    install_root: &Path,
    cua_node_subsystem: Option<u16>,
) -> Result<bool, EngineError> {
    let path = install_root
        .join("resources")
        .join(CODEX_APP_TOOLS_MCP_CONFIG);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(io_err("inspect Codex App Tools MCP configuration", error)),
    };
    if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
        return Err(EngineError::Install(
            "Codex App Tools MCP 配置不是普通文件，拒绝修改".to_string(),
        ));
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err(EngineError::Install(
                "Codex App Tools MCP 配置是 Windows 重解析点，拒绝修改".to_string(),
            ));
        }
    }
    if metadata.len() > 1024 * 1024 {
        return Err(EngineError::Install(
            "Codex App Tools MCP 配置超出安全大小上限".to_string(),
        ));
    }
    ensure_plain_directory_tree(install_root, path.parent().expect("MCP config has parent"))?;
    let original =
        fs::read(&path).map_err(|error| io_err("read Codex App Tools MCP configuration", error))?;
    let has_utf8_bom = original.starts_with(&[0xEF, 0xBB, 0xBF]);
    let json_bytes = if has_utf8_bom {
        &original[3..]
    } else {
        &original[..]
    };
    let mut config: serde_json::Value = serde_json::from_slice(json_bytes).map_err(|error| {
        EngineError::Install(format!("解析 Codex App Tools MCP 配置失败：{error}"))
    })?;
    let Some(server) = mcp_server_mut(&mut config).and_then(|servers| servers.get_mut("codex_app"))
    else {
        log::debug!("Codex App Tools MCP configuration has no codex_app server; leaving unchanged");
        return Ok(false);
    };
    let Some(server_object) = server.as_object_mut() else {
        return Err(EngineError::Install(
            "Codex App Tools MCP 服务配置格式错误".to_string(),
        ));
    };
    let command = server_object
        .get("command")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let args = server_object
        .get("args")
        .and_then(serde_json::Value::as_array);
    let direct_args = CODEX_APP_TOOLS_MCP_DIRECT_ARGS
        .iter()
        .map(|value| serde_json::Value::String((*value).to_string()))
        .collect::<Vec<_>>();
    let is_direct = command.eq_ignore_ascii_case(CODEX_APP_TOOLS_MCP_NODE_COMMAND)
        && args.is_some_and(|args| args == &direct_args);
    let is_legacy = command.eq_ignore_ascii_case(CODEX_APP_TOOLS_MCP_OLD_COMMAND)
        && args.is_some_and(|args| {
            args.len() == CODEX_APP_TOOLS_MCP_OLD_ARGS.len()
                && args
                    .iter()
                    .zip(CODEX_APP_TOOLS_MCP_OLD_ARGS)
                    .all(|(actual, expected)| actual.as_str() == Some(expected))
        });
    if !is_direct && !is_legacy {
        log::warn!("Codex App Tools MCP launcher has an unknown shape; leaving it unchanged");
        return Ok(false);
    }

    let node_path = install_root
        .join("resources")
        .join("cua_node")
        .join("bin")
        .join("node.exe");
    if !is_regular_non_reparse_file(&node_path)? || cua_node_subsystem != Some(2) {
        return Err(EngineError::Install(
            "Codex App Tools MCP 隐藏控制台启动器未通过 PE 校验，拒绝保留会弹出控制台的启动方式"
                .to_string(),
        ));
    }
    if is_direct {
        return Ok(false);
    }

    server_object.insert(
        "command".to_string(),
        serde_json::Value::String(CODEX_APP_TOOLS_MCP_NODE_COMMAND.to_string()),
    );
    server_object.insert("args".to_string(), serde_json::Value::Array(direct_args));
    let mut serialized = serde_json::to_vec_pretty(&config).map_err(|error| {
        EngineError::Install(format!("序列化 Codex App Tools MCP 配置失败：{error}"))
    })?;
    if has_utf8_bom {
        serialized.splice(0..0, [0xEF, 0xBB, 0xBF]);
    }
    serialized.push(b'\n');
    replace_file_atomically(&path, &serialized)?;
    log::info!("portable Codex App Tools MCP launcher switched from cmd.exe to bundled GUI Node");
    Ok(true)
}

/// Make an extracted Codex tree safe to launch without an MSIX package
/// identity.  This is intentionally idempotent and best-effort for payloads
/// that have already moved to a newer upstream bootstrap shape: the native
/// window check below still rejects a fatal startup dialog instead of reporting
/// a false success.
pub fn ensure_portable_codex_startup_compatibility(
    install_root: &Path,
) -> Result<bool, EngineError> {
    // The executable edit complements the ASAR `windowsHide` patch.  Both
    // run before the first portable launch and remain safe to repeat after an
    // in-place update or a manager restart.
    let (mut patched, cua_node_subsystem) = patch_console_subsystem_executables(install_root)?;
    patched |= patch_codex_app_tools_mcp_config(install_root, cua_node_subsystem)?;
    let app_asar = install_root.join("resources").join("app.asar");
    if !app_asar.is_file() {
        return Ok(patched);
    }

    // This patch is independent from the unpackaged-MSIX updater patch below:
    // newer Codex builds can change or remove the updater bootstrap while the
    // app-server spawn remains the source of a visible console window.
    patched |= patch_codex_console_spawn(&app_asar)?;

    // Only touch a Codex archive that actually contains the package-identity
    // updater.  This keeps the patch scoped to the known failure mechanism.
    if find_bytes_in_file(&app_asar, CODEX_ASAR_UPDATER_MARKER)
        .map_err(|error| io_err("scan Codex app.asar for portable startup marker", error))?
        .is_none()
    {
        return Ok(patched);
    }

    // Check the fixed-width package-query sentinel before scanning for the
    // original call.  This keeps subsequent launches bounded to the first few
    // megabytes of app.asar instead of repeatedly reading the whole archive
    // from a slow removable drive.
    if find_bytes_in_file(&app_asar, CODEX_ASAR_PACKAGE_FAMILY_PATCH_PREFIX)
        .map_err(|error| {
            io_err(
                "scan Codex app.asar for existing package-family patch",
                error,
            )
        })?
        .is_some()
    {
        return Ok(patched);
    }

    let updater_patched = patch_first_fixed_width_call(
        &app_asar,
        CODEX_ASAR_PACKAGE_FAMILY_CALL,
        CODEX_ASAR_PACKAGE_FAMILY_PATCH_PREFIX,
        "unpackaged-msix-package-family-query",
    )?;
    patched |= updater_patched;

    // The unguarded bootstrap query is the failure that prevents an unpackaged
    // Codex process from reaching its main window. Once it is repaired there
    // is no reason to scan the rest of a 300+ MB archive for an optional,
    // older updater initializer.
    if patched {
        return Ok(true);
    }

    // A patched updater call is detected without rebuilding the archive on
    // every launch; the minified bootstrap call is near the beginning of the
    // archive and the exact sentinel is unique to this compatibility patch.
    if find_bytes_in_file(&app_asar, b"await 0/*USB*/;")
        .map_err(|error| io_err("scan Codex app.asar for existing startup patch", error))?
        .is_some()
    {
        return Ok(patched);
    }

    let mut updater_call = None;
    for call in CODEX_ASAR_UPDATER_CALLS {
        if let Some(offset) = find_bytes_in_file(&app_asar, call)
            .map_err(|error| io_err("scan Codex app.asar for updater initialization", error))?
        {
            updater_call = Some((offset, call.len()));
            break;
        }
    }
    let Some((offset, call_len)) = updater_call else {
        log::warn!(
            "portable Codex package-identity updater marker is present but the expected bootstrap call was not found path={}",
            app_asar.display()
        );
        return Ok(patched);
    };

    if call_len < CODEX_ASAR_UPDATER_PATCH_PREFIX.len() {
        return Err(EngineError::Msix(
            "portable Codex startup patch has an invalid fixed-width replacement".to_string(),
        ));
    }
    let mut replacement = vec![b' '; call_len];
    replacement[..CODEX_ASAR_UPDATER_PATCH_PREFIX.len()]
        .copy_from_slice(CODEX_ASAR_UPDATER_PATCH_PREFIX);
    patch_asar_bytes_in_place(&app_asar, offset, &replacement)?;
    patched = true;
    log::info!(
        "portable Codex startup compatibility patch applied path={} offset={} reason=unpackaged-msix-updater",
        app_asar.display(),
        offset
    );
    Ok(patched)
}

/// Enable Codex's bundled webview translations for a Chinese launch.
///
/// Upstream desktop builds have shipped the `zh-CN` resources while using a
/// false local fallback for the remote `enable_i18n` flag.  That leaves the
/// complete UI in English when the machine cannot fetch the feature config.
/// The USB launcher owns the extracted payload, so it can safely flip that
/// boolean with the same fixed-width, idempotent ASAR edit used by the
/// unpackaged MSIX compatibility fix.  A build with a different bundle shape
/// is left untouched and continues to launch in its native fallback language.
pub fn ensure_portable_codex_locale_compatibility(
    install_root: &Path,
    locale: Option<&str>,
) -> Result<bool, EngineError> {
    if !requests_chinese_codex_locale(locale) {
        return Ok(false);
    }
    let app_asar = install_root.join("resources").join("app.asar");
    if !app_asar.is_file() {
        return Ok(false);
    }

    for (call, replacement) in CODEX_ASAR_I18N_CALLS {
        if call.len() != replacement.len() {
            return Err(EngineError::Msix(
                "portable Codex i18n patch changed ASAR width".to_string(),
            ));
        }
        if find_bytes_in_file(&app_asar, call)
            .map_err(|error| io_err("scan Codex app.asar for i18n fallback", error))?
            .is_some()
        {
            let patched = patch_first_fixed_width_call(
                &app_asar,
                call,
                replacement,
                "bundled-i18n-default-fallback",
            )?;
            if patched {
                return Ok(true);
            }
        }
    }

    log::warn!(
        "portable Codex Chinese locale requested but no compatible enable_i18n fallback was found path={}",
        app_asar.display()
    );
    Ok(false)
}

fn extraction_file_retry_delay(attempt: usize) -> Duration {
    // Security tools and indexers can briefly hold a newly-created file. Keep
    // this finite and short: a real ACL denial must still surface promptly.
    Duration::from_millis(match attempt {
        0 => 100,
        1 => 250,
        _ => 500,
    })
}

fn create_extracted_file(path: &Path) -> Result<fs::File, EngineError> {
    let mut last_error = None;
    for attempt in 0..=EXTRACT_FILE_CREATE_RETRIES {
        match fs::File::create(path) {
            Ok(file) => return Ok(file),
            Err(error)
                if attempt < EXTRACT_FILE_CREATE_RETRIES
                    && matches!(
                        error.kind(),
                        io::ErrorKind::Interrupted
                            | io::ErrorKind::PermissionDenied
                            | io::ErrorKind::WouldBlock
                    ) =>
            {
                log::warn!(
                    "portable extraction file create retry path={} attempt={} error={error}",
                    path.display(),
                    attempt + 1
                );
                last_error = Some(error);
                std::thread::sleep(extraction_file_retry_delay(attempt));
            }
            Err(error) => {
                return Err(io_err(
                    &format!("create extracted file {}", path.display()),
                    error,
                ));
            }
        }
    }
    Err(io_err(
        &format!("create extracted file {}", path.display()),
        last_error.expect("retry loop must retain the last error"),
    ))
}

fn cleanup_portable_work_dir(work_dir: &Path) -> Result<(), EngineError> {
    match fs::remove_dir_all(work_dir) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(io_err("remove portable staging operation", error)),
    }
    let Some(staging_root) = work_dir.parent().filter(|path| {
        path.file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name == ".codex-app-manager-staging")
    }) else {
        return Ok(());
    };
    // Never recurse into the shared staging parent. Removing it only when it
    // is empty preserves another authenticated operation/recovery directory.
    match fs::remove_dir(staging_root) {
        Ok(()) => Ok(()),
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::DirectoryNotEmpty
            ) =>
        {
            Ok(())
        }
        Err(error) => Err(io_err("remove empty portable staging root", error)),
    }
}

// Directory replacement can briefly race process teardown, Windows Defender,
// or another file scanner even after every managed process has exited. Retry
// only the Windows errors that describe transient handle/lock contention; all
// other failures still return immediately.
const WINDOWS_FS_RETRY_DELAYS_MS: [u64; 8] = [50, 100, 200, 400, 800, 1_600, 2_500, 5_000];

fn is_transient_windows_fs_error(err: &io::Error) -> bool {
    // ERROR_ACCESS_DENIED, ERROR_SHARING_VIOLATION, ERROR_LOCK_VIOLATION.
    #[cfg(any(windows, test))]
    {
        matches!(err.raw_os_error(), Some(5 | 32 | 33))
    }
    #[cfg(not(any(windows, test)))]
    {
        let _ = err;
        false
    }
}

fn filesystem_operation_with_retry<F, S>(
    operation: &str,
    source: &Path,
    destination: Option<&Path>,
    mut action: F,
    mut sleep: S,
) -> io::Result<()>
where
    F: FnMut() -> io::Result<()>,
    S: FnMut(Duration),
{
    let destination_text = destination
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "none".to_string());
    let mut attempt = 1usize;
    loop {
        match action() {
            Ok(()) => {
                if attempt > 1 {
                    log::info!(
                        "portable filesystem operation succeeded after retry operation={operation} attempts={attempt} source={} destination={destination_text}",
                        source.display()
                    );
                }
                return Ok(());
            }
            Err(err) => {
                let retryable = is_transient_windows_fs_error(&err);
                let next_delay = WINDOWS_FS_RETRY_DELAYS_MS.get(attempt - 1).copied();
                let retry_delay = if retryable { next_delay } else { None };
                if let Some(delay_ms) = retry_delay {
                    log::warn!(
                        "portable filesystem operation temporarily blocked operation={operation} attempt={attempt} raw_os_error={:?} source_exists={} destination_exists={} source={} destination={destination_text} error={err}; retrying_in_ms={delay_ms}",
                        err.raw_os_error(),
                        source.exists(),
                        destination.is_some_and(Path::exists),
                        source.display()
                    );
                    sleep(Duration::from_millis(delay_ms));
                    attempt += 1;
                    continue;
                }

                log::error!(
                    "portable filesystem operation failed operation={operation} attempts={attempt} retryable={retryable} raw_os_error={:?} source_exists={} destination_exists={} source={} destination={destination_text} error={err}",
                    err.raw_os_error(),
                    source.exists(),
                    destination.is_some_and(Path::exists),
                    source.display()
                );
                return Err(err);
            }
        }
    }
}

fn rename_with_retry<F, S>(
    operation: &str,
    from: &Path,
    to: &Path,
    rename: F,
    sleep: S,
) -> io::Result<()>
where
    F: FnMut() -> io::Result<()>,
    S: FnMut(Duration),
{
    filesystem_operation_with_retry(operation, from, Some(to), rename, sleep)
}

/// Rename a portable-install directory, retrying only transient Windows lock
/// errors with the same bounded backoff used by the real install swap.
pub fn rename_directory_with_retry(operation: &str, from: &Path, to: &Path) -> io::Result<()> {
    rename_with_retry(operation, from, to, || fs::rename(from, to), thread::sleep)
}

/// Remove a portable-install directory with bounded retries for transient
/// Windows scanner/handle contention.
pub fn remove_directory_all_with_retry(operation: &str, path: &Path) -> io::Result<()> {
    filesystem_operation_with_retry(
        operation,
        path,
        None,
        || fs::remove_dir_all(path),
        thread::sleep,
    )
}

/// Remove a known manager-owned path without following a symbolic link. The
/// install tree and Start Menu shortcut can be held briefly by the shell or a
/// scanner after the Codex process has exited, so use the same bounded retry
/// policy as the install swap and verify that the path is really gone.
fn remove_path_with_retry(operation: &str, path: &Path) -> io::Result<bool> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    let remove = || {
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            fs::remove_file(path)
        } else {
            fs::remove_dir_all(path)
        }
    };
    filesystem_operation_with_retry(operation, path, None, remove, thread::sleep)?;
    match fs::symlink_metadata(path) {
        Ok(_) => {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                format!("path still exists after removal: {}", path.display()),
            ))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    Ok(true)
}

fn rename_portable_dir(operation: &str, from: &Path, to: &Path) -> Result<(), EngineError> {
    rename_directory_with_retry(operation, from, to).map_err(|err| io_err(operation, err))
}

fn copy_dir_all(from: &Path, to: &Path) -> Result<(), EngineError> {
    fs::create_dir_all(to).map_err(|e| io_err("create dir", e))?;
    for entry in fs::read_dir(from).map_err(|e| io_err("read dir", e))? {
        let entry = entry.map_err(|e| io_err("read dir entry", e))?;
        let ty = entry.file_type().map_err(|e| io_err("read file type", e))?;
        let dest = to.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_all(&entry.path(), &dest)?;
        } else if ty.is_file() {
            let source_len = entry
                .metadata()
                .map_err(|e| io_err("read source file metadata", e))?
                .len();
            let copied = fs::copy(entry.path(), &dest).map_err(|e| io_err("copy file", e))?;
            let destination_len = fs::metadata(&dest)
                .map_err(|e| io_err("read copied file metadata", e))?
                .len();
            if copied != source_len || destination_len != source_len {
                return Err(EngineError::Io(format!(
                    "portable payload copy was incomplete: {} -> {} (source={source_len}, copied={copied}, destination={destination_len})",
                    entry.path().display(),
                    dest.display()
                )));
            }
        } else {
            return Err(EngineError::Io(format!(
                "portable payload contains an unsupported filesystem entry: {}",
                entry.path().display()
            )));
        }
    }
    Ok(())
}

fn extract_msix(msix_path: &Path, dest: &Path) -> Result<String, EngineError> {
    let file = fs::File::open(msix_path).map_err(|e| io_err("open MSIX", e))?;
    let mut zip =
        zip::ZipArchive::new(file).map_err(|e| EngineError::Msix(format!("open zip: {e}")))?;
    let mut manifest_xml = None;

    for idx in 0..zip.len() {
        let mut file = zip
            .by_index(idx)
            .map_err(|e| EngineError::Msix(format!("read zip entry {idx}: {e}")))?;
        let Some(enclosed) = file.enclosed_name() else {
            continue;
        };
        let out_path = dest.join(&enclosed);
        if file.is_dir() {
            fs::create_dir_all(&out_path).map_err(|e| io_err("create extracted dir", e))?;
            continue;
        }
        if let Some(parent) = out_path.parent() {
            fs::create_dir_all(parent).map_err(|e| io_err("create extracted parent", e))?;
        }
        let mut out = create_extracted_file(&out_path)?;
        let expected_len = file.size();
        let extracted_len =
            std::io::copy(&mut file, &mut out).map_err(|e| io_err("extract file", e))?;
        if extracted_len != expected_len {
            return Err(EngineError::Msix(format!(
                "MSIX entry extraction was incomplete: {} (expected={expected_len}, actual={extracted_len})",
                enclosed.display()
            )));
        }

        if enclosed
            .file_name()
            .and_then(|s| s.to_str())
            .is_some_and(|s| s.eq_ignore_ascii_case("AppxManifest.xml"))
            && enclosed.components().count() == 1
        {
            let mut xml = String::new();
            fs::File::open(&out_path)
                .and_then(|mut f| f.read_to_string(&mut xml))
                .map_err(|e| io_err("read extracted AppxManifest.xml", e))?;
            manifest_xml = Some(xml);
        }
    }

    manifest_xml.ok_or_else(|| EngineError::Msix("MSIX missing AppxManifest.xml".to_string()))
}

/// Entry-executable basenames the Codex lineage has shipped, newest first.
/// Post-merge packages keep a legacy `Codex.exe` next to the real entrypoint,
/// so `ChatGPT.exe` must win when the manifest can't tell us (it normally can).
const APP_EXE_CANDIDATES: [&str; 2] = ["ChatGPT.exe", "Codex.exe"];
const PORTABLE_SHELL_OWNERSHIP_SCHEMA: u32 = 1;
const PORTABLE_SHELL_OWNERSHIP_PRODUCT: &str = "codex-app-manager";
const PORTABLE_SHELL_OWNERSHIP_FILE: &str = "portable-shell-ownership.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PortableShellOwnership {
    schema_version: u32,
    product: String,
    #[serde(default)]
    install_roots: Vec<String>,
    #[serde(default)]
    shortcut_paths: Vec<String>,
}

fn portable_shell_ownership_path() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(|local| {
        PathBuf::from(local)
            .join("io.github.wangnov.codexappmanager")
            .join(PORTABLE_SHELL_OWNERSHIP_FILE)
    })
}

fn record_portable_shell_ownership(
    install_root: &Path,
    shortcut: &Path,
) -> Result<(), EngineError> {
    validate_cleanup_path(install_root)?;
    validate_cleanup_path(shortcut)?;
    let path = portable_shell_ownership_path()
        .ok_or_else(|| EngineError::Install("LOCALAPPDATA is unavailable".to_string()))?;
    let parent = path.parent().ok_or_else(|| {
        EngineError::Install("portable shell ownership has no parent".to_string())
    })?;
    validate_cleanup_path(parent)?;
    fs::create_dir_all(parent)
        .map_err(|error| io_err("create shell ownership directory", error))?;
    validate_cleanup_path(&path)?;

    let mut record = if path.is_file() {
        let bytes = fs::read(&path).map_err(|error| io_err("read shell ownership", error))?;
        if bytes.len() > 64 * 1024 {
            return Err(EngineError::Install(
                "portable shell ownership record is too large".to_string(),
            ));
        }
        serde_json::from_slice::<PortableShellOwnership>(&bytes).map_err(|error| {
            EngineError::Install(format!("invalid shell ownership record: {error}"))
        })?
    } else if path.exists() {
        return Err(EngineError::Install(
            "portable shell ownership record is not a regular file".to_string(),
        ));
    } else {
        PortableShellOwnership {
            schema_version: PORTABLE_SHELL_OWNERSHIP_SCHEMA,
            product: PORTABLE_SHELL_OWNERSHIP_PRODUCT.to_string(),
            install_roots: Vec::new(),
            shortcut_paths: Vec::new(),
        }
    };
    if record.schema_version != PORTABLE_SHELL_OWNERSHIP_SCHEMA
        || record.product != PORTABLE_SHELL_OWNERSHIP_PRODUCT
    {
        return Err(EngineError::Install(
            "portable shell ownership record identity does not match".to_string(),
        ));
    }
    let root = install_root.to_string_lossy().into_owned();
    if !record
        .install_roots
        .iter()
        .any(|existing| existing.eq_ignore_ascii_case(&root))
    {
        record.install_roots.push(root);
    }
    let shortcut = shortcut.to_string_lossy().into_owned();
    if !record
        .shortcut_paths
        .iter()
        .any(|existing| existing.eq_ignore_ascii_case(&shortcut))
    {
        record.shortcut_paths.push(shortcut);
    }
    record
        .install_roots
        .sort_by_key(|value| value.to_ascii_lowercase());
    record
        .shortcut_paths
        .sort_by_key(|value| value.to_ascii_lowercase());
    let bytes = serde_json::to_vec_pretty(&record)
        .map_err(|error| EngineError::Install(format!("serialize shell ownership: {error}")))?;
    fs::write(path, bytes).map_err(|error| io_err("write shell ownership", error))
}

fn find_exe_named(root: &Path, name: &str) -> Result<Option<PathBuf>, EngineError> {
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).map_err(|e| io_err("walk extracted MSIX", e))? {
            let entry = entry.map_err(|e| io_err("walk extracted MSIX entry", e))?;
            let path = entry.path();
            let ty = entry.file_type().map_err(|e| io_err("read file type", e))?;
            if ty.is_dir() {
                stack.push(path);
            } else if path
                .file_name()
                .and_then(|s| s.to_str())
                .is_some_and(|n| n.eq_ignore_ascii_case(name))
            {
                return Ok(Some(path));
            }
        }
    }
    Ok(None)
}

/// Locate the app's entry executable in an extracted MSIX.
///
/// The manifest's `Application@Executable` is authoritative: when declared it
/// is resolved as a package-root relative path, then by basename — and a
/// declared entry that cannot be found is a hard error, NOT a fallback case.
/// Falling back would silently select a non-entry binary (e.g. the legacy
/// `Codex.exe` shipped next to the real `ChatGPT.exe`) when the true entry is
/// missing or quarantined, and the install would then health-check the wrong
/// binary. The known-name candidates only serve manifests with no
/// `<Application>` declaration at all.
fn find_app_exe(root: &Path, manifest_xml: &str) -> Result<PathBuf, EngineError> {
    if let Some(declared) = crate::msix::parse_appx_application_executable(manifest_xml) {
        // Manifest paths use either separator; resolve component-wise. Only the
        // exact declared path counts — matching a same-named file elsewhere in
        // the package would select (and copy the parent directory of) a binary
        // that is not the entry.
        let relative: PathBuf = declared.replace('\\', "/").split('/').collect();
        let direct = root.join(&relative);
        if direct.is_file() {
            return Ok(direct);
        }
        return Err(EngineError::Msix(format!(
            "MSIX manifest declares entry executable '{declared}' but it is missing from the payload"
        )));
    }
    for name in APP_EXE_CANDIDATES {
        if let Some(found) = find_exe_named(root, name)? {
            return Ok(found);
        }
    }
    Err(EngineError::Msix(
        "MSIX did not contain an app entry executable (ChatGPT.exe / Codex.exe)".to_string(),
    ))
}

/// The entry executable of an installed portable root. Reads the payload's
/// `AppxManifest.xml` (written at install time) for the declared executable's
/// basename — the payload root is the exe's directory, so only the basename
/// applies. A declared-but-missing entry returns `None` (the install is
/// broken; picking a leftover non-entry binary would mask that). The known
/// entry names are probed only for roots without a declaring manifest.
pub fn installed_app_exe(install_root: &Path) -> Option<PathBuf> {
    let manifest = install_root.join("AppxManifest.xml");
    if let Ok(xml) = fs::read_to_string(&manifest) {
        if let Some(declared) = crate::msix::parse_appx_application_executable(&xml) {
            let basename = declared.replace('\\', "/");
            let name = basename.rsplit('/').next()?;
            let exe = install_root.join(name);
            return exe.is_file().then_some(exe);
        }
    }
    APP_EXE_CANDIDATES
        .into_iter()
        .map(|name| install_root.join(name))
        .find(|exe| exe.is_file())
}

fn prepare_portable_payload(
    msix_path: &Path,
    work_dir: &Path,
    progress: Option<&PortableExtractionProgressObserver<'_>>,
) -> Result<PreparedPortable, EngineError> {
    let extracted = work_dir.join("extracted");
    let payload = work_dir.join("payload");
    if extracted.exists() {
        fs::remove_dir_all(&extracted).map_err(|e| io_err("clear extracted dir", e))?;
    }
    if payload.exists() {
        fs::remove_dir_all(&payload).map_err(|e| io_err("clear payload dir", e))?;
    }
    fs::create_dir_all(&payload).map_err(|e| io_err("create payload dir", e))?;

    // The previous implementation unpacked the complete MSIX to
    // `work_dir/extracted` and then copied the executable directory to
    // `work_dir/payload`. For the real Codex package that meant roughly 1.96
    // GB of app files were written to a removable drive twice, with no
    // observable progress between the two writes. Prefer the manifest's
    // authoritative executable path and stream only its containing tree
    // directly into the transactional payload. This keeps the old full
    // extraction path as a compatibility fallback for malformed/minimal
    // manifests that do not declare an Application executable.
    let package_file = fs::File::open(msix_path).map_err(|e| io_err("open MSIX", e))?;
    let mut package = zip::ZipArchive::new(package_file)
        .map_err(|e| EngineError::Msix(format!("open zip: {e}")))?;
    let mut manifest_entry = package
        .by_name("AppxManifest.xml")
        .map_err(|e| EngineError::Msix(format!("read AppxManifest.xml: {e}")))?;
    let mut manifest_xml = String::new();
    manifest_entry
        .read_to_string(&mut manifest_xml)
        .map_err(|e| io_err("decode AppxManifest.xml", e))?;
    drop(manifest_entry);

    let identity = parse_appx_manifest_xml(&manifest_xml)?;
    if let Some(declared) = crate::msix::parse_appx_application_executable(&manifest_xml) {
        let declared_relative: PathBuf = declared.replace('\\', "/").split('/').collect();
        let app_prefix = declared_relative.parent().unwrap_or_else(|| Path::new(""));

        let mut selected_entries = 0usize;
        let mut selected_bytes = 0u64;
        for idx in 0..package.len() {
            let entry = package
                .by_index(idx)
                .map_err(|e| EngineError::Msix(format!("read zip entry {idx}: {e}")))?;
            let Some(enclosed) = entry.enclosed_name() else {
                continue;
            };
            if enclosed.starts_with(app_prefix) {
                selected_entries += 1;
                if !entry.is_dir() {
                    selected_bytes = selected_bytes.saturating_add(entry.size());
                }
            }
        }
        log::info!(
            "portable extraction started route=declared-app package={} destination={} entries={} bytes={} declared_executable={}",
            msix_path.display(),
            payload.display(),
            selected_entries,
            selected_bytes,
            declared
        );

        if let Some(observer) = progress {
            observer(PortableExtractionProgress {
                completed_bytes: 0,
                total_bytes: selected_bytes,
                files_completed: 0,
            })?;
        }

        let mut extracted_files = 0usize;
        let mut extracted_bytes = 0u64;
        let mut last_logged_bytes = 0u64;
        let mut last_logged_files = 0usize;
        let mut last_reported_bytes = 0u64;
        let mut last_reported_at = Instant::now();
        let mut copy_buffer = vec![0u8; EXTRACTION_COPY_BUFFER_BYTES];
        let mut declared_entry_found = false;
        for idx in 0..package.len() {
            let mut entry = package
                .by_index(idx)
                .map_err(|e| EngineError::Msix(format!("read zip entry {idx}: {e}")))?;
            let Some(enclosed) = entry.enclosed_name() else {
                continue;
            };
            if !enclosed.starts_with(app_prefix) {
                continue;
            }
            if enclosed == declared_relative {
                declared_entry_found = true;
            }
            let relative = enclosed.strip_prefix(app_prefix).map_err(|_| {
                EngineError::Msix("MSIX app entry path was not under its app directory".to_string())
            })?;
            let output = payload.join(relative);
            if entry.is_dir() {
                fs::create_dir_all(&output).map_err(|e| io_err("create extracted dir", e))?;
                continue;
            }
            if let Some(parent) = output.parent() {
                fs::create_dir_all(parent).map_err(|e| io_err("create extracted parent", e))?;
            }
            let expected_len = entry.size();
            let mut out = create_extracted_file(&output)?;
            // `std::io::copy` uses a small internal buffer.  A reusable 1 MiB
            // buffer reduces syscall overhead on a USB target while allowing
            // us to publish progress during large individual files instead of
            // waiting for the next file boundary.
            let mut extracted_len = 0u64;
            loop {
                let read = entry
                    .read(&mut copy_buffer)
                    .map_err(|e| io_err("extract app file", e))?;
                if read == 0 {
                    break;
                }
                out.write_all(&copy_buffer[..read])
                    .map_err(|e| io_err("write extracted app file", e))?;
                extracted_len = extracted_len.saturating_add(read as u64);
                extracted_bytes = extracted_bytes.saturating_add(read as u64);
                if let Some(observer) = progress {
                    if extracted_bytes.saturating_sub(last_reported_bytes)
                        >= EXTRACTION_PROGRESS_BYTES
                        || last_reported_at.elapsed() >= EXTRACTION_PROGRESS_INTERVAL
                    {
                        observer(PortableExtractionProgress {
                            completed_bytes: extracted_bytes,
                            total_bytes: selected_bytes,
                            files_completed: extracted_files as u64,
                        })?;
                        last_reported_bytes = extracted_bytes;
                        last_reported_at = Instant::now();
                    }
                }
            }
            out.flush()
                .map_err(|e| io_err("flush extracted app file", e))?;
            if extracted_len != expected_len {
                return Err(EngineError::Msix(format!(
                    "MSIX app entry extraction was incomplete: {} (expected={expected_len}, actual={extracted_len})",
                    enclosed.display()
                )));
            }
            drop(out);
            let destination_len = fs::metadata(&output)
                .map_err(|e| io_err("read extracted app file metadata", e))?
                .len();
            if destination_len != expected_len {
                return Err(EngineError::Msix(format!(
                    "MSIX app entry was written incompletely: {} (expected={expected_len}, actual={destination_len})",
                    enclosed.display()
                )));
            }
            extracted_files += 1;
            if let Some(observer) = progress {
                observer(PortableExtractionProgress {
                    completed_bytes: extracted_bytes,
                    total_bytes: selected_bytes,
                    files_completed: extracted_files as u64,
                })?;
                last_reported_bytes = extracted_bytes;
                last_reported_at = Instant::now();
            }
            if extracted_bytes.saturating_sub(last_logged_bytes) >= 128 * 1024 * 1024
                || extracted_files.saturating_sub(last_logged_files) >= 512
            {
                log::info!(
                    "portable extraction progress route=declared-app destination={} files={} bytes={} total_bytes={}",
                    payload.display(),
                    extracted_files,
                    extracted_bytes,
                    selected_bytes
                );
                last_logged_bytes = extracted_bytes;
                last_logged_files = extracted_files;
            }
        }
        if !declared_entry_found {
            return Err(EngineError::Msix(format!(
                "MSIX manifest declares entry executable '{declared}' but it is missing from the payload"
            )));
        }
        if let Some(observer) = progress {
            observer(PortableExtractionProgress {
                completed_bytes: extracted_bytes,
                total_bytes: selected_bytes,
                files_completed: extracted_files as u64,
            })?;
        }
        log::info!(
            "portable extraction completed route=declared-app destination={} files={} bytes={}",
            payload.display(),
            extracted_files,
            extracted_bytes
        );
    } else {
        log::info!(
            "portable extraction started route=full-package-fallback package={} destination={}",
            msix_path.display(),
            payload.display()
        );
        fs::create_dir_all(&extracted).map_err(|e| io_err("create extracted dir", e))?;
        let extracted_manifest = extract_msix(msix_path, &extracted)?;
        let exe = find_app_exe(&extracted, &extracted_manifest)?;
        let exe_dir = exe.parent().ok_or_else(|| {
            EngineError::Msix("app entry executable had no parent directory".to_string())
        })?;
        copy_dir_all(exe_dir, &payload)?;
        log::info!(
            "portable extraction completed route=full-package-fallback destination={}",
            payload.display()
        );
    }

    fs::write(payload.join("AppxManifest.xml"), manifest_xml)
        .map_err(|e| io_err("write portable AppxManifest.xml", e))?;

    // Apply the unpackaged-launch compatibility fix before the transactional
    // payload is moved into place.  The operation is idempotent and is also
    // repeated lazily at launch so an already-installed tree upgraded by an
    // older manager is repaired without requiring a second reinstall.
    if let Err(error) = ensure_portable_codex_startup_compatibility(&payload) {
        // Windows Defender/indexers can briefly hold a freshly extracted
        // 200–300 MB ASAR.  Do not roll back an otherwise verified install for
        // that transient lock; launch performs the same bounded repair before
        // it starts the app and will surface a precise error if it remains
        // impossible to patch.
        log::warn!(
            "portable Codex startup compatibility patch deferred path={} error={error}",
            payload.display()
        );
    }

    Ok(PreparedPortable {
        payload_dir: payload,
        identity,
    })
}

#[cfg(windows)]
fn ps_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

#[cfg(windows)]
fn powershell_exe() -> PathBuf {
    std::env::var_os("WINDIR")
        .map(PathBuf::from)
        .map(|windir| {
            windir
                .join("System32")
                .join("WindowsPowerShell")
                .join("v1.0")
                .join("powershell.exe")
        })
        .filter(|path| path.exists())
        .unwrap_or_else(|| PathBuf::from("powershell.exe"))
}

#[cfg(windows)]
fn run_powershell(script: &str) -> Result<String, EngineError> {
    // Shortcut/uninstall metadata scripts can wait on COM or registry work; use
    // the install budget so a stuck policy machine cannot hang forever.
    run_powershell_with_limits(script, RunLimits::install())
}

#[cfg(windows)]
fn run_powershell_with_limits(script: &str, limits: RunLimits) -> Result<String, EngineError> {
    let mut command = hidden_command(powershell_exe());
    command.args(["-NoProfile", "-NonInteractive", "-Command", script]);
    let output = run_capturing(command, limits, None)
        .map_err(|e| EngineError::Install(format!("powershell: {}", e.message())))?;
    if !output.status.success() {
        return Err(EngineError::Install(format!(
            "powershell failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

pub fn close_codex_gracefully_for_root(timeout_secs: u64, root: &Path) -> Result<(), EngineError> {
    crate::windows_process::close_codex_processes_for_root(timeout_secs, root)
}

/// Whether Codex currently has any process running from this exact install
/// root. The same native, path-pinned discovery is used by the close gate.
pub fn codex_running_for_root(root: &Path) -> Result<bool, EngineError> {
    crate::windows_process::codex_processes_running_for_root(root)
}

#[cfg(windows)]
fn create_start_menu_shortcut_script(install_root: &Path, shortcut: &Path, exe: &Path) -> String {
    format!(
        r#"
$shell = New-Object -ComObject WScript.Shell
$shortcut = $shell.CreateShortcut({shortcut})
$shortcut.TargetPath = {target}
$shortcut.WorkingDirectory = {workdir}
$shortcut.IconLocation = {icon}
$shortcut.Description = 'codex-app-manager:managed-portable'
$shortcut.Save()
$acl = Get-Acl -LiteralPath {shortcut}
$accounts = New-Object System.Collections.Generic.List[string]
try {{
  $accounts.Add(([Security.Principal.WindowsIdentity]::GetCurrent().User.Translate([Security.Principal.NTAccount]).Value))
}} catch {{ }}
if ($env:USERDOMAIN -and $env:USERNAME) {{
  try {{
    $accounts.Add((New-Object -TypeName System.Security.Principal.NTAccount -ArgumentList @($env:USERDOMAIN, $env:USERNAME)).Value)
  }} catch {{ }}
}}
if ($accounts.Count -eq 0) {{ throw '无法确定快捷方式所属用户' }}
foreach ($account in @($accounts | Select-Object -Unique)) {{
  $rule = New-Object System.Security.AccessControl.FileSystemAccessRule($account, 'FullControl', 'Allow')
  $acl.SetAccessRule($rule)
}}
Set-Acl -LiteralPath {shortcut} -AclObject $acl
"#,
        shortcut = ps_quote(&shortcut.to_string_lossy()),
        target = ps_quote(&exe.to_string_lossy()),
        workdir = ps_quote(&install_root.to_string_lossy()),
        icon = ps_quote(&format!("{},0", exe.to_string_lossy()))
    )
}

#[cfg(all(windows, not(test)))]
fn create_start_menu_shortcut(install_root: &Path) -> Result<bool, EngineError> {
    let Some(exe) = installed_app_exe(install_root) else {
        return Ok(false);
    };
    let Some(appdata) = std::env::var_os("APPDATA") else {
        return Ok(false);
    };
    let shortcut = PathBuf::from(appdata)
        .join("Microsoft")
        .join("Windows")
        .join("Start Menu")
        .join("Programs")
        .join("Codex.lnk");
    let script = create_start_menu_shortcut_script(install_root, &shortcut, &exe);
    record_portable_shell_ownership(install_root, &shortcut)?;
    run_powershell(&script)?;
    Ok(true)
}

#[cfg(any(not(windows), test))]
fn create_start_menu_shortcut(_install_root: &Path) -> Result<bool, EngineError> {
    Ok(false)
}

#[cfg(windows)]
fn portable_uninstall_script(install_root: &Path) -> String {
    format!(
        r#"$root = '{root}'; $rootFull = ([IO.Path]::GetFullPath($root)).TrimEnd('\') + '\'; $locations = @(); if ($env:APPDATA) {{ $locations += (Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs'); $locations += (Join-Path $env:APPDATA 'Microsoft\Windows\Desktop') }}; if ($env:USERPROFILE) {{ $locations += (Join-Path $env:USERPROFILE 'Desktop') }}; if ($env:ProgramData) {{ $locations += (Join-Path $env:ProgramData 'Microsoft\Windows\Start Menu\Programs'); $locations += (Join-Path $env:ProgramData 'Microsoft\Windows\Desktop') }}; if ($env:PUBLIC) {{ $locations += (Join-Path $env:PUBLIC 'Desktop') }}; $names = @('Codex.lnk','ChatGPT.lnk','CCSwitch.lnk','Codex Switch.lnk','OpenAI Codex.lnk'); foreach ($location in $locations) {{ foreach ($name in $names) {{ $shortcutPath = Join-Path $location $name; if (Test-Path -LiteralPath $shortcutPath -PathType Leaf) {{ try {{ $link = (New-Object -ComObject WScript.Shell).CreateShortcut($shortcutPath); $target = [IO.Path]::GetFullPath([string]$link.TargetPath); if ($target.StartsWith($rootFull, [StringComparison]::OrdinalIgnoreCase)) {{ Remove-Item -LiteralPath $shortcutPath -Force -ErrorAction SilentlyContinue }} }} catch {{ }} }} }} }}; if ($env:LOCALAPPDATA) {{ Remove-Item -LiteralPath (Join-Path $env:LOCALAPPDATA 'io.github.wangnov.codexappmanager\portable-shell-ownership.json') -Force -ErrorAction SilentlyContinue }}; Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue; Remove-Item -LiteralPath 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Codex' -Recurse -Force -ErrorAction SilentlyContinue"#,
        root = install_root.to_string_lossy().replace('\'', "''")
    )
}

#[cfg(all(windows, not(test)))]
fn register_uninstall_entry(
    install_root: &Path,
    version: &str,
    estimated_size_kb: u64,
) -> Result<bool, EngineError> {
    // Icon only — the entry works without one, so fall back to the legacy name.
    let exe = installed_app_exe(install_root).unwrap_or_else(|| install_root.join("Codex.exe"));
    let uninstall_script = portable_uninstall_script(install_root);
    // Wrap the script in DOUBLE quotes, not single, so Windows' uninstall entry
    // actually RUNS it: `-Command '<script>'` makes PowerShell evaluate the text
    // as one string literal and echo it back; `-Command "<script>"` executes it.
    // The install path sits in single quotes inside, and Windows paths can't
    // contain '"', so the outer double quotes stay unambiguous. -ExecutionPolicy
    // Bypass keeps a restrictive machine policy from blocking the removal.
    let uninstall_string = format!(
        "powershell.exe -NoProfile -ExecutionPolicy Bypass -Command \"{uninstall_script}\""
    );
    let script = format!(
        r#"
$key = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Codex'
New-Item -Path $key -Force | Out-Null
New-ItemProperty -Path $key -Name DisplayName -Value 'Codex' -PropertyType String -Force | Out-Null
New-ItemProperty -Path $key -Name DisplayVersion -Value {version} -PropertyType String -Force | Out-Null
New-ItemProperty -Path $key -Name Publisher -Value 'OpenAI' -PropertyType String -Force | Out-Null
New-ItemProperty -Path $key -Name CodexAppManagerProduct -Value 'codex-app-manager' -PropertyType String -Force | Out-Null
New-ItemProperty -Path $key -Name InstallLocation -Value {install_root} -PropertyType String -Force | Out-Null
New-ItemProperty -Path $key -Name DisplayIcon -Value {icon} -PropertyType String -Force | Out-Null
New-ItemProperty -Path $key -Name UninstallString -Value {uninstall_string} -PropertyType String -Force | Out-Null
New-ItemProperty -Path $key -Name QuietUninstallString -Value {uninstall_string} -PropertyType String -Force | Out-Null
New-ItemProperty -Path $key -Name NoModify -Value 1 -PropertyType DWord -Force | Out-Null
New-ItemProperty -Path $key -Name NoRepair -Value 1 -PropertyType DWord -Force | Out-Null
New-ItemProperty -Path $key -Name EstimatedSize -Value {estimated_size_kb} -PropertyType DWord -Force | Out-Null
"#,
        version = ps_quote(version),
        install_root = ps_quote(&install_root.to_string_lossy()),
        icon = ps_quote(&format!("{},0", exe.to_string_lossy())),
        uninstall_string = ps_quote(&uninstall_string),
        estimated_size_kb = estimated_size_kb.min(u32::MAX as u64)
    );
    run_powershell(&script)?;
    Ok(true)
}

#[cfg(any(not(windows), test))]
fn register_uninstall_entry(
    _install_root: &Path,
    _version: &str,
    _estimated_size_kb: u64,
) -> Result<bool, EngineError> {
    Ok(false)
}

#[cfg(windows)]
fn portable_metadata_ownership_script() -> &'static str {
    r#"
$ErrorActionPreference = 'Stop'
function Get-OwnedPortableEntry {
  $key = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Codex'
  try { $entry = Get-ItemProperty -LiteralPath $key -ErrorAction Stop }
  catch { if ($_.CategoryInfo.Category -eq 'ObjectNotFound') { return $null }; throw }
  $root = [string]$entry.InstallLocation
  if ([string]::IsNullOrWhiteSpace($root) -or -not [IO.Path]::IsPathRooted($root)) { return $null }
  if ($entry.CodexAppManagerProduct -eq 'codex-app-manager') { return $entry }
  # Legacy releases did not write an ownership marker. Only the exact old
  # command template, name and publisher prove that this manager created it.
  $template = @'
if ($env:APPDATA) { $Shortcut = Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs\Codex.lnk'; Remove-Item -LiteralPath $Shortcut -Force -ErrorAction SilentlyContinue }; Remove-Item -LiteralPath '__ROOT__' -Recurse -Force -ErrorAction SilentlyContinue; Remove-Item -LiteralPath 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Codex' -Recurse -Force -ErrorAction SilentlyContinue
'@
  $expected = 'powershell.exe -NoProfile -ExecutionPolicy Bypass -Command "' + $template.Trim().Replace('__ROOT__', $root.Replace("'", "''")) + '"'
  if ($entry.DisplayName -eq 'Codex' -and $entry.Publisher -eq 'OpenAI' -and [string]$entry.UninstallString -ceq $expected) { return $entry }
  return $null
}
function Get-PortableShellOwnership {
  if (-not $env:LOCALAPPDATA) { return $null }
  $path = Join-Path $env:LOCALAPPDATA 'io.github.wangnov.codexappmanager\portable-shell-ownership.json'
  if (-not (Test-Path -LiteralPath $path -PathType Leaf -ErrorAction Stop)) { return $null }
  try {
    $record = Get-Content -LiteralPath $path -Raw -Encoding UTF8 -ErrorAction Stop | ConvertFrom-Json
    if ([int]$record.schemaVersion -eq 1 -and [string]$record.product -eq 'codex-app-manager') { return $record }
  } catch { }
  return $null
}
function Get-PortableShellLocations {
  $locations = New-Object System.Collections.Generic.List[string]
  if ($env:APPDATA) {
    $locations.Add((Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs'))
    $locations.Add((Join-Path $env:APPDATA 'Microsoft\Windows\Desktop'))
  }
  if ($env:USERPROFILE) { $locations.Add((Join-Path $env:USERPROFILE 'Desktop')) }
  if ($env:ProgramData) {
    $locations.Add((Join-Path $env:ProgramData 'Microsoft\Windows\Start Menu\Programs'))
    $locations.Add((Join-Path $env:ProgramData 'Microsoft\Windows\Desktop'))
  }
  if ($env:PUBLIC) { $locations.Add((Join-Path $env:PUBLIC 'Desktop')) }
  return @($locations | Select-Object -Unique)
}
function Normalize-PortablePath([string]$value) {
  if ([string]::IsNullOrWhiteSpace($value)) { return $null }
  try { return [IO.Path]::GetFullPath($value).TrimEnd('\') } catch { return $null }
}
function Test-PortablePathUnderRoot([string]$child, [string]$root) {
  $childFull = Normalize-PortablePath $child
  $rootFull = Normalize-PortablePath $root
  if (-not $childFull -or -not $rootFull) { return $false }
  return $childFull.StartsWith(($rootFull + '\'), [StringComparison]::OrdinalIgnoreCase) -or
    $childFull.Equals($rootFull, [StringComparison]::OrdinalIgnoreCase)
}
function Get-PortableShortcutCandidates {
  $locations = @(Get-PortableShellLocations)
  $names = @('Codex.lnk','ChatGPT.lnk','CCSwitch.lnk','Codex Switch.lnk','OpenAI Codex.lnk')
  $candidates = New-Object System.Collections.Generic.List[string]
  foreach ($location in $locations) {
    foreach ($name in $names) {
      $candidates.Add((Join-Path $location $name))
    }
  }
  $record = Get-PortableShellOwnership
  if ($record) {
    foreach ($path in @($record.shortcutPaths)) {
      try {
        if ([IO.Path]::GetExtension([string]$path) -ieq '.lnk' -and
            (@($locations | Where-Object { Test-PortablePathUnderRoot ([string]$path) $_ }).Count -gt 0)) {
          $candidates.Add([string]$path)
        }
      } catch { }
    }
  }
  return @($candidates | Select-Object -Unique)
}
function Test-PortableShortcutOwned($shortcutPath, $entry, $record) {
  $link = (New-Object -ComObject WScript.Shell).CreateShortcut($shortcutPath)
  if ([string]$link.Description -eq 'codex-app-manager:managed-portable') { return $true }
  $roots = New-Object System.Collections.Generic.List[string]
  if ($entry) { $roots.Add([string]$entry.InstallLocation) }
  if ($record) {
    foreach ($root in @($record.installRoots)) { $roots.Add([string]$root) }
  }
  foreach ($root in $roots) {
    if (Test-PortablePathUnderRoot ([string]$link.TargetPath) $root) { return $true }
  }
  return $false
}
"#
}

#[cfg(windows)]
fn portable_stale_shortcut_script() -> String {
    format!(
        r#"{}
$entry = Get-OwnedPortableEntry
$record = Get-PortableShellOwnership
$removed = $false
$preserved = $false
foreach ($shortcutPath in @(Get-PortableShortcutCandidates)) {{
  if (-not (Test-Path -LiteralPath $shortcutPath -PathType Leaf -ErrorAction Stop)) {{ continue }}
  try {{
    $link = (New-Object -ComObject WScript.Shell).CreateShortcut($shortcutPath)
    $target = [string]$link.TargetPath
    if (-not [string]::IsNullOrWhiteSpace($target) -and
        (Test-Path -LiteralPath $target -PathType Leaf -ErrorAction SilentlyContinue)) {{ continue }}
    if (-not (Test-PortableShortcutOwned $shortcutPath $entry $record)) {{ continue }}
    $deleted = $false
    for ($attempt = 0; $attempt -lt 8; $attempt++) {{
      try {{
        Remove-Item -LiteralPath $shortcutPath -Force -ErrorAction Stop
        if (-not (Test-Path -LiteralPath $shortcutPath -ErrorAction Stop)) {{
          $deleted = $true
          break
        }}
      }} catch {{
        if ($attempt -eq 7) {{ throw }}
      }}
      Start-Sleep -Milliseconds 125
    }}
    if (-not $deleted) {{ throw ('stale shortcut still present: ' + $shortcutPath) }}
    $removed = $true
  }} catch {{
    # A startup sweep is best-effort; uninstall remains the authoritative
    # cleanup path and reports permission/ownership failures to the user.
    $preserved = $true
  }}
}}
if ($removed) {{ 'removed' }} elseif ($preserved) {{ 'preserved' }} else {{ 'absent' }}
"#,
        portable_metadata_ownership_script(),
    )
}

fn metadata_cleanup_result(
    output: &str,
    label: &str,
) -> Result<(bool, Option<String>), EngineError> {
    match output.trim() {
        "removed" => Ok((true, None)),
        "absent" => Ok((false, None)),
        "preserved" => Ok((
            false,
            Some(format!(
                "Preserved unrelated/unverified {label}; no manager ownership proof."
            )),
        )),
        other => Err(EngineError::Install(format!(
            "{label} cleanup postcondition unknown: {other}"
        ))),
    }
}

#[cfg(all(windows, not(test)))]
fn portable_shell_search_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(appdata) = std::env::var_os("APPDATA") {
        let appdata = PathBuf::from(appdata);
        roots.push(
            appdata
                .join("Microsoft")
                .join("Windows")
                .join("Start Menu")
                .join("Programs"),
        );
        roots.push(appdata.join("Microsoft").join("Windows").join("Desktop"));
    }
    if let Some(user_profile) = std::env::var_os("USERPROFILE") {
        roots.push(PathBuf::from(user_profile).join("Desktop"));
    }
    if let Some(program_data) = std::env::var_os("ProgramData") {
        let program_data = PathBuf::from(program_data);
        roots.push(
            program_data
                .join("Microsoft")
                .join("Windows")
                .join("Start Menu")
                .join("Programs"),
        );
        roots.push(
            program_data
                .join("Microsoft")
                .join("Windows")
                .join("Desktop"),
        );
    }
    if let Some(public) = std::env::var_os("PUBLIC") {
        roots.push(PathBuf::from(public).join("Desktop"));
    }
    roots.sort();
    roots.dedup();
    roots
}

#[cfg(all(windows, not(test)))]
fn refresh_portable_shell_directories() {
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::UI::Shell::{SHChangeNotify, SHCNE_UPDATEDIR, SHCNF_PATHW};

    for root in portable_shell_search_roots() {
        let wide = root
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>();
        unsafe {
            SHChangeNotify(
                SHCNE_UPDATEDIR,
                SHCNF_PATHW,
                Some(wide.as_ptr() as *const std::ffi::c_void),
                None,
            );
        }
    }
}

#[cfg(all(windows, not(test)))]
pub fn cleanup_stale_portable_shortcuts() -> Result<bool, EngineError> {
    for root in portable_shell_search_roots() {
        validate_cleanup_path(&root)?;
    }
    let (removed, note) = metadata_cleanup_result(
        &run_powershell(&portable_stale_shortcut_script())?,
        "stale shell",
    )?;
    if let Some(note) = note {
        log::debug!("{note}");
    }
    if removed {
        refresh_portable_shell_directories();
    }
    Ok(removed)
}

#[cfg(any(not(windows), test))]
pub fn cleanup_stale_portable_shortcuts() -> Result<bool, EngineError> {
    Ok(false)
}

#[cfg(all(windows, not(test)))]
fn remove_start_menu_shortcut() -> Result<(bool, Option<String>), EngineError> {
    for root in portable_shell_search_roots() {
        validate_cleanup_path(&root)?;
    }
    let script = format!(
        r#"{}
$entry = Get-OwnedPortableEntry
$record = Get-PortableShellOwnership
$removed = $false
$preserved = $false
foreach ($shortcutPath in @(Get-PortableShortcutCandidates)) {{
  if (-not (Test-Path -LiteralPath $shortcutPath -PathType Leaf -ErrorAction Stop)) {{ continue }}
  $owned = $false
  try {{ $owned = Test-PortableShortcutOwned $shortcutPath $entry $record }} catch {{ $preserved = $true; continue }}
  if (-not $owned) {{ $preserved = $true; continue }}
  $deleted = $false
  for ($attempt = 0; $attempt -lt 8; $attempt++) {{
    try {{
      Remove-Item -LiteralPath $shortcutPath -Force -ErrorAction Stop
      if (-not (Test-Path -LiteralPath $shortcutPath -ErrorAction Stop)) {{
        $deleted = $true
        break
      }}
    }} catch {{
      if ($attempt -eq 7) {{ throw }}
    }}
    Start-Sleep -Milliseconds 125
  }}
  if (-not $deleted) {{ throw ('shortcut still present: ' + $shortcutPath) }}
  $removed = $true
}}
$ownershipPath = if ($env:LOCALAPPDATA) {{ Join-Path $env:LOCALAPPDATA 'io.github.wangnov.codexappmanager\portable-shell-ownership.json' }} else {{ $null }}
if ($ownershipPath -and (Test-Path -LiteralPath $ownershipPath -PathType Leaf -ErrorAction Stop) -and -not $preserved) {{
  Remove-Item -LiteralPath $ownershipPath -Force -ErrorAction Stop
}}
if ($removed) {{ 'removed' }} elseif ($preserved) {{ 'preserved' }} else {{ 'absent' }}
"#,
        portable_metadata_ownership_script(),
    );
    metadata_cleanup_result(&run_powershell(&script)?, "Start Menu shortcut")
}

#[cfg(any(not(windows), test))]
fn remove_start_menu_shortcut() -> Result<(bool, Option<String>), EngineError> {
    Ok((false, None))
}

#[cfg(all(windows, not(test)))]
fn remove_uninstall_entry() -> Result<(bool, Option<String>), EngineError> {
    let script = format!(
        "{}\n{}",
        portable_metadata_ownership_script(),
        r#"
$key = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Codex'
for ($attempt = 0; $attempt -lt 8; $attempt++) {
  if (-not (Test-Path -LiteralPath $key -ErrorAction Stop)) {
    'absent'
    exit 0
  }
  if (-not (Get-OwnedPortableEntry)) { 'preserved'; exit 0 }
  try {
    Remove-Item -LiteralPath $key -Recurse -Force -ErrorAction Stop
  } catch {
    if ($attempt -eq 7) { throw }
  }
  if (-not (Test-Path -LiteralPath $key -ErrorAction Stop)) {
    'removed'
    exit 0
  }
  Start-Sleep -Milliseconds 125
}
'remaining'
"#
    );
    metadata_cleanup_result(&run_powershell(&script)?, "Apps & Features entry")
}

#[cfg(any(not(windows), test))]
fn remove_uninstall_entry() -> Result<(bool, Option<String>), EngineError> {
    Ok((false, None))
}

fn dir_size_kb(root: &Path) -> u64 {
    let mut total = 0u64;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            if meta.is_dir() {
                stack.push(entry.path());
            } else {
                total = total.saturating_add(meta.len());
            }
        }
    }
    total / 1024
}

fn restore_previous_install(
    install_root: &Path,
    backup: &Path,
    had_previous: bool,
) -> Result<(), EngineError> {
    // Never delete the failed/new tree and then discover that the old tree we
    // promised to restore is gone. Missing rollback material is ambiguous and
    // must not emit positive rollback evidence.
    if had_previous && !backup.exists() {
        return Err(EngineError::Install(format!(
            "portable rollback backup is missing: {}",
            backup.display()
        )));
    }
    if install_root.exists() {
        remove_directory_all_with_retry("remove failed portable install", install_root)
            .map_err(|e| io_err("remove failed portable install", e))?;
    }
    if had_previous {
        rename_portable_dir("restore portable rollback backup", backup, install_root)?;
    }
    Ok(())
}

fn rollback_install_error(
    install_root: &Path,
    backup: &Path,
    had_previous: bool,
    observer: &mut PortableObserver<'_>,
    err: EngineError,
) -> EngineError {
    // Give the app layer a durable, pre-rename marker before the only backup is
    // consumed. The rollback itself still proceeds if bookkeeping fails: disk
    // safety wins, and RollbackCompleted gets a second chance to persist truth.
    let intent_error = observer(PortableBoundary::BeforeRollback {
        install_root: install_root.to_path_buf(),
        backup: backup.to_path_buf(),
        had_previous,
    })
    .err();
    match restore_previous_install(install_root, backup, had_previous) {
        Ok(()) => {
            let restored = if had_previous {
                "previous install was restored"
            } else {
                "new install was removed and the absent state was restored"
            };
            let completion = observer(PortableBoundary::RollbackCompleted {
                install_root: install_root.to_path_buf(),
                backup: backup.to_path_buf(),
                had_previous,
            });
            match (intent_error, completion) {
                (None, Ok(())) => EngineError::Install(format!("{err}; {restored}")),
                (Some(intent_err), Ok(())) => EngineError::Install(format!(
                    "{err}; {restored}, but recording rollback intent failed: {intent_err}"
                )),
                (_, Err(evidence_err)) => EngineError::Install(format!(
                    "{err}; {restored}, but recording rollback evidence failed: {evidence_err}"
                )),
            }
        }
        Err(rollback_err) => {
            EngineError::Install(format!("{err}; rollback failed: {rollback_err}"))
        }
    }
}

fn health_check_portable_install(
    install_root: &Path,
    launch: bool,
    keep_running: bool,
) -> Result<bool, EngineError> {
    let exe = installed_app_exe(install_root).ok_or_else(|| {
        EngineError::Install(format!(
            "portable health check failed: no app entry executable (ChatGPT.exe / Codex.exe) in {}",
            install_root.display()
        ))
    })?;
    if !launch {
        return Ok(false);
    }
    ensure_portable_codex_startup_compatibility(install_root)?;
    // Spawn alone is not enough: a broken payload can exit immediately after
    // CreateProcess succeeds. Require a short liveness window, then leave the
    // process running (this path is the post-install relaunch).
    match spawn_and_require_liveness(hidden_command(&exe), PORTABLE_LIVENESS_WINDOW) {
        Ok(LivenessResult::Survived { child }) => {
            if keep_running {
                // Intentionally leak the Child handle so the relaunched app
                // keeps running after the manager drops the wait loop.
                std::mem::forget(child);
                Ok(true)
            } else {
                // The launch was only a health check. Close the whole Electron
                // process tree so an update cannot open an app that was closed
                // beforehand.
                let deadline = Instant::now() + Duration::from_secs(30);
                loop {
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    if remaining.is_zero() {
                        return Err(EngineError::Install(format!(
                            "portable health check cleanup failed: Codex is still running from {}",
                            install_root.display()
                        )));
                    }
                    // A closing Electron parent may replace itself with another
                    // process. Use bounded close slices and rescan the exact root
                    // between them instead of trusting one PID snapshot.
                    close_codex_gracefully_for_root(remaining.as_secs().clamp(1, 5), install_root)?;
                    if !codex_running_for_root(install_root)? {
                        break;
                    }
                }
                drop(child);
                Ok(false)
            }
        }
        Ok(LivenessResult::ExitedEarly { code }) => Err(EngineError::Install(format!(
            "portable health check failed: entry executable exited immediately after launch (exit={})",
            code.map(|c| c.to_string())
                .unwrap_or_else(|| "signal".to_string())
        ))),
        Err(err) => Err(EngineError::Install(format!(
            "portable health check launch failed: {}",
            err.message()
        ))),
    }
}

pub fn install_portable_from_msix(
    msix_path: &Path,
    install_root: &Path,
    relaunch: bool,
) -> Result<PortableInstallReport, EngineError> {
    let root = install_root.display();
    log::info!("portable install start install_root={root}");
    match install_portable_from_msix_inner(msix_path, install_root, true, relaunch) {
        Ok(report) => {
            let root = &report.install_root;
            log::info!("portable install completed install_root={root}");
            Ok(report)
        }
        Err(err) => {
            log::error!(
                "portable install failed install_root={} error={err}",
                install_root.display()
            );
            Err(err)
        }
    }
}

/// Rename boundary markers for crash-recovery callbacks and fault injection.
/// Path-carrying variants let the app layer persist a durable transaction log
/// with the real staging/backup locations chosen by this install.
#[derive(Debug, Clone)]
pub enum PortableBoundary {
    /// About to move the current install aside. `payload` is the staged new tree;
    /// `backup` is where the old install will go (if any).
    BeforeMoveOld {
        install_root: PathBuf,
        payload: PathBuf,
        backup: PathBuf,
        had_previous: bool,
    },
    /// Old install is at `backup`; install path is empty.
    AfterMoveOld {
        install_root: PathBuf,
        payload: PathBuf,
        backup: PathBuf,
        had_previous: bool,
    },
    BeforeMoveNew {
        install_root: PathBuf,
        payload: PathBuf,
        backup: PathBuf,
        had_previous: bool,
    },
    /// New payload is at install root.
    AfterMoveNew {
        install_root: PathBuf,
        backup: PathBuf,
        had_previous: bool,
    },
    /// A failure chose rollback; persist that intent before consuming backup.
    BeforeRollback {
        install_root: PathBuf,
        backup: PathBuf,
        had_previous: bool,
    },
    /// A later failure was fully rolled back. The install root now matches its
    /// pre-operation state (the old tree was restored, or a fresh tree removed).
    RollbackCompleted {
        install_root: PathBuf,
        backup: PathBuf,
        had_previous: bool,
    },
}

pub type PortableObserver<'a> = dyn FnMut(PortableBoundary) -> Result<(), EngineError> + 'a;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortableFault {
    BeforeMoveOld,
    AfterMoveOld,
    OnMoveNew,
    AfterMoveNew,
}

thread_local! {
    static PORTABLE_FAULT: std::cell::Cell<Option<PortableFault>> =
        const { std::cell::Cell::new(None) };
}

/// Install a one-shot fault for the next portable install rename sequence.
pub fn inject_portable_fault(fault: Option<PortableFault>) {
    PORTABLE_FAULT.with(|cell| cell.set(fault));
}

fn take_portable_fault() -> Option<PortableFault> {
    PORTABLE_FAULT.with(|cell| cell.take())
}

fn portable_fault_err(boundary: &str) -> EngineError {
    EngineError::Io(format!("injected portable fault at {boundary}"))
}

fn install_portable_from_msix_inner(
    msix_path: &Path,
    install_root: &Path,
    manage_process: bool,
    relaunch: bool,
) -> Result<PortableInstallReport, EngineError> {
    install_portable_from_msix_with_observer(
        msix_path,
        install_root,
        manage_process,
        relaunch,
        &mut |_| Ok(()),
    )
}

/// Like the normal portable install path, but notifies `observer` at each
/// rename boundary so callers can persist a crash-recovery transaction log.
pub fn install_portable_from_msix_with_observer(
    msix_path: &Path,
    install_root: &Path,
    manage_process: bool,
    relaunch: bool,
    observer: &mut PortableObserver<'_>,
) -> Result<PortableInstallReport, EngineError> {
    install_portable_from_msix_with_observer_options(
        msix_path,
        install_root,
        PortableInstallOptions::legacy(manage_process, relaunch),
        observer,
    )
}

/// Portable installer with independently controlled launch and shell side
/// effects.  USB callers use this entry point so installation never starts an
/// unconfigured Codex process and never creates a direct-launch shortcut.
pub fn install_portable_from_msix_with_observer_options(
    msix_path: &Path,
    install_root: &Path,
    options: PortableInstallOptions,
    observer: &mut PortableObserver<'_>,
) -> Result<PortableInstallReport, EngineError> {
    install_portable_from_msix_with_observer_options_and_progress(
        msix_path,
        install_root,
        options,
        observer,
        None,
    )
}

/// Portable installer variant that reports bytes while the verified archive is
/// unpacked.  The ordinary API above intentionally remains unchanged for
/// callers that do not need UI progress.
pub fn install_portable_from_msix_with_observer_options_and_progress(
    msix_path: &Path,
    install_root: &Path,
    options: PortableInstallOptions,
    observer: &mut PortableObserver<'_>,
    progress: Option<&PortableExtractionProgressObserver<'_>>,
) -> Result<PortableInstallReport, EngineError> {
    let install_parent = install_root.parent().unwrap_or(install_root);
    fs::create_dir_all(install_parent).map_err(|e| io_err("create install parent", e))?;
    let operation_id = uuid::Uuid::new_v4();
    let work_dir = install_parent
        .join(".codex-app-manager-staging")
        .join(format!("portable-{operation_id}"));
    if work_dir.exists() {
        fs::remove_dir_all(&work_dir).map_err(|e| io_err("clear portable staging", e))?;
    }
    fs::create_dir_all(&work_dir).map_err(|e| io_err("create portable staging", e))?;

    let prepared = match prepare_portable_payload(msix_path, &work_dir, progress) {
        Ok(prepared) => prepared,
        Err(error) => {
            return Err(match cleanup_portable_work_dir(&work_dir) {
                Ok(()) => error,
                Err(cleanup) => EngineError::Install(format!(
                    "{error}; portable staging cleanup also failed: {cleanup}"
                )),
            });
        }
    };
    let payload = prepared.payload_dir;
    let backup = install_parent.join(format!("Codex.rollback-{operation_id}"));
    let mut notes = Vec::new();

    if options.close_running {
        close_codex_gracefully_for_root(30, install_root)?;
    }

    let fault = take_portable_fault();
    let had_previous = install_root.exists();

    if let Err(error) = observer(PortableBoundary::BeforeMoveOld {
        install_root: install_root.to_path_buf(),
        payload: payload.clone(),
        backup: backup.clone(),
        had_previous,
    }) {
        let _ = cleanup_portable_work_dir(&work_dir);
        return Err(error);
    }
    if fault == Some(PortableFault::BeforeMoveOld) {
        let _ = cleanup_portable_work_dir(&work_dir);
        return Err(portable_fault_err("before-move-old"));
    }

    if had_previous {
        rename_portable_dir("move current install to rollback", install_root, &backup)?;
    }

    // Observer must persist OldMoved. On failure: restore previous install when
    // possible so we never leave an empty root without a recovery path.
    if let Err(obs_err) = observer(PortableBoundary::AfterMoveOld {
        install_root: install_root.to_path_buf(),
        payload: payload.clone(),
        backup: backup.clone(),
        had_previous,
    }) {
        let _ = cleanup_portable_work_dir(&work_dir);
        return Err(rollback_install_error(
            install_root,
            &backup,
            had_previous,
            observer,
            obs_err,
        ));
    }
    if fault == Some(PortableFault::AfterMoveOld) {
        // Leave the crash window intact for recovery tests (no auto-rollback).
        return Err(portable_fault_err("after-move-old"));
    }

    if let Err(error) = observer(PortableBoundary::BeforeMoveNew {
        install_root: install_root.to_path_buf(),
        payload: payload.clone(),
        backup: backup.clone(),
        had_previous,
    }) {
        let _ = cleanup_portable_work_dir(&work_dir);
        return Err(rollback_install_error(
            install_root,
            &backup,
            had_previous,
            observer,
            error,
        ));
    }
    if fault == Some(PortableFault::OnMoveNew) {
        let _ = cleanup_portable_work_dir(&work_dir);
        return Err(rollback_install_error(
            install_root,
            &backup,
            had_previous,
            observer,
            portable_fault_err("on-move-new"),
        ));
    }

    match rename_portable_dir("install portable payload", &payload, install_root) {
        Ok(()) => {
            if let Err(obs_err) = observer(PortableBoundary::AfterMoveNew {
                install_root: install_root.to_path_buf(),
                backup: backup.clone(),
                had_previous,
            }) {
                // Payload is already at install_root; leave for recovery.
                log::error!("portable observer failed after move-new: {obs_err}");
                return Err(obs_err);
            }
        }
        Err(err) => {
            return Err(rollback_install_error(
                install_root,
                &backup,
                had_previous,
                observer,
                err,
            ));
        }
    }

    if fault == Some(PortableFault::AfterMoveNew) {
        let _ = cleanup_portable_work_dir(&work_dir);
        return Err(rollback_install_error(
            install_root,
            &backup,
            had_previous,
            observer,
            portable_fault_err("after-move-new"),
        ));
    }

    let relaunched = match health_check_portable_install(
        install_root,
        options.launch_health_check,
        options.launch_health_check && options.relaunch,
    ) {
        Ok(relaunched) => relaunched,
        Err(err) => {
            let _ = cleanup_portable_work_dir(&work_dir);
            return Err(rollback_install_error(
                install_root,
                &backup,
                had_previous,
                observer,
                err,
            ));
        }
    };

    let (shortcut_created, uninstall_entry_created) = if options.create_shell_metadata {
        let shortcut_created = match create_start_menu_shortcut(install_root) {
            Ok(created) => created,
            Err(err) => {
                notes.push(format!("Start menu shortcut was not created: {err}"));
                false
            }
        };
        let uninstall_entry_created = match register_uninstall_entry(
            install_root,
            &prepared.identity.version,
            dir_size_kb(install_root),
        ) {
            Ok(created) => created,
            Err(err) => {
                notes.push(format!(
                    "Apps & Features uninstall entry was not created: {err}"
                ));
                false
            }
        };
        (shortcut_created, uninstall_entry_created)
    } else {
        notes.push(
            "USB edition keeps launch and uninstall entry points inside the guarded bundle; direct shell metadata was intentionally skipped."
                .to_string(),
        );
        (false, false)
    };

    let installed_exe = installed_app_exe(install_root);
    let mut backup_path = None;
    if had_previous && backup.exists() {
        match fs::remove_dir_all(&backup) {
            Ok(()) => {}
            Err(err) => {
                notes.push(format!(
                    "Portable rollback backup could not be removed after successful install: {err}"
                ));
                backup_path = Some(backup.to_string_lossy().into_owned());
            }
        }
    }

    if let Err(error) = cleanup_portable_work_dir(&work_dir) {
        notes.push(format!(
            "Portable staging cleanup could not finish after installation: {error}"
        ));
    }

    let version = read_codex_app_version_from_install_root(install_root)
        .unwrap_or_else(|| prepared.identity.version.clone());

    Ok(PortableInstallReport {
        success: true,
        install_root: install_root.to_string_lossy().into_owned(),
        executable_path: installed_exe.map(|exe| exe.to_string_lossy().into_owned()),
        version,
        backup_path,
        shortcut_created,
        uninstall_entry_created,
        relaunched,
        message: "Portable Codex install completed.".to_string(),
        notes,
    })
}

/// Return the user-profile paths that belong to the Codex desktop/CLI
/// installation. The first entries (`~/.codex` and `~/.agents/skills`) are
/// shared with Codex CLI/agent tooling; the other entries are desktop/AppX
/// caches. They are only used when the user explicitly turns off "keep my
/// data". In particular, this list never walks a broad `%LOCALAPPDATA%` tree
/// or follows an arbitrary glob.
fn codex_user_data_paths() -> Result<Vec<PathBuf>, EngineError> {
    let mut paths = Vec::new();
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"));

    if let Some(home) = home {
        let home = PathBuf::from(home);
        paths.push(home.join(".codex"));
        // The bundled capability pack installs user-scoped Skills here. Keep
        // it in the same explicit purge contract as ~/.codex so a local
        // Codex/Skills uninstall does not leave the pack behind. The USB
        // source tree is outside the user profile and is never selected here.
        paths.push(home.join(".agents").join("skills"));
    }

    if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") {
        let local = PathBuf::from(local_app_data);
        // Windows can retain the per-user AppX data directory after
        // Remove-AppxPackage, especially when a package was moved between
        // volumes.  Only direct children with the exact OpenAI.Codex family
        // prefix are eligible.
        let package_root = local.join("Packages");
        validate_cleanup_path(&package_root)?;
        match fs::read_dir(&package_root) {
            Ok(entries) => {
                for entry in entries {
                    let entry =
                        entry.map_err(|error| io_err("enumerate Codex AppX data", error))?;
                    let path = entry.path();
                    let metadata = match fs::symlink_metadata(&path) {
                        Ok(metadata) => metadata,
                        Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
                        Err(error) => return Err(io_err("inspect Codex AppX data", error)),
                    };
                    if !metadata.is_dir() || metadata.file_type().is_symlink() {
                        continue;
                    }
                    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                        continue;
                    };
                    if name.eq_ignore_ascii_case("OpenAI.Codex_2p2nqsd0c76g0") {
                        paths.push(path);
                    }
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(io_err("enumerate Codex AppX data", error)),
        }
        // These are desktop runtime caches written by the OpenAI package.  A
        // separate `Codex\\Logs` path is used instead of deleting the whole
        // `%LOCALAPPDATA%\\Codex` parent because other tools may store data
        // alongside it.
        paths.push(local.join("OpenAI").join("Codex"));
        paths.push(local.join("Codex").join("Logs"));
        // CodexAppxDataBackups contains the write-ahead recovery journal and
        // verified pre-removal copies.  It is product recovery evidence, not
        // ordinary profile data.  The USB manager deletes that exact PFN
        // subtree only from its deferred finalizer, after AppX volume/policy
        // cleanup has reached a verified terminal state.  Removing it here
        // would make a later partial-cleanup message falsely claim that the
        // evidence was retained.
    }

    if let Some(app_data) = std::env::var_os("APPDATA") {
        let roaming = PathBuf::from(app_data);
        paths.push(roaming.join("OpenAI").join("Codex"));
        paths.push(roaming.join("Codex"));
    }

    paths.sort();
    paths.dedup();
    Ok(paths)
}

/// Reject junctions/reparse points on every existing ancestor before a scoped
/// user-data deletion. `symlink_metadata` on the leaf alone does not protect
/// against e.g. a redirected Programs or Packages parent. Unknown access is not
/// absence. The caller still owns selection of the exact permitted target.
pub fn validate_cleanup_path(path: &Path) -> Result<(), EngineError> {
    if !path.is_absolute()
        || path.parent().is_none()
        || path
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
    {
        return Err(EngineError::Install(format!(
            "unsafe cleanup path: {}",
            path.display()
        )));
    }
    for ancestor in path.ancestors() {
        match fs::symlink_metadata(ancestor) {
            Ok(metadata) => {
                #[cfg(windows)]
                let reparse = {
                    use std::os::windows::fs::MetadataExt;
                    metadata.file_attributes() & 0x400 != 0
                };
                #[cfg(not(windows))]
                let reparse = false;
                if metadata.file_type().is_symlink() || reparse {
                    return Err(EngineError::Install(format!(
                        "cleanup refuses reparse point: {}",
                        ancestor.display()
                    )));
                }
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(io_err("inspect cleanup ancestor", error)),
        }
    }
    Ok(())
}

/// Remove one user-data path without following a symbolic link.  Windows can
/// keep a handle to an AppX cache for a short time after package removal, so
/// use the same bounded retry primitive as portable installation cleanup and
/// verify the postcondition before returning success.
fn remove_user_data_path(path: &Path) -> Result<bool, EngineError> {
    validate_cleanup_path(path)?;
    remove_path_with_retry("purge user data", path)
        .map_err(|error| io_err("purge user data", error))
}

/// Remove the user's Codex data directories (sign-in, sessions, config and
/// desktop/AppX caches). Returns whether anything was actually deleted.
/// Shared by the portable and MSIX uninstall paths so both honor the "don't
/// keep my data" choice identically. A missing path is idempotent; an existing
/// path that cannot be removed is surfaced so the UI cannot claim a clean
/// uninstall and can offer a retry.
pub fn purge_codex_user_data(notes: &mut Vec<String>) -> Result<bool, EngineError> {
    let paths = codex_user_data_paths()?;
    if paths.is_empty() {
        notes.push("User data purge requested but home directory was not available.".to_string());
        return Ok(false);
    }

    let mut removed_any = false;
    for path in paths {
        match fs::symlink_metadata(&path) {
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => {
                // A permission/IO error is not the same as an absent path.
                // Returning it lets the caller surface a partial cleanup and
                // offer a retry instead of claiming that the data was purged.
                return Err(io_err("inspect Codex user data", error));
            }
        }
        log::warn!("purging Codex user data path={}", path.display());
        if remove_user_data_path(&path)? {
            removed_any = true;
            notes.push(format!("Removed Codex user data: {}", path.display()));
        }
    }
    // A scanner or another process can recreate a profile directory after its
    // removal. Verify the complete set once more before telling the caller
    // that the destructive purge finished.
    for path in codex_user_data_paths()? {
        match fs::symlink_metadata(&path) {
            Ok(_) => {
                return Err(io_err(
                    "verify Codex user data purge",
                    io::Error::new(
                        io::ErrorKind::Other,
                        format!("path still exists after removal: {}", path.display()),
                    ),
                ));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(io_err("verify Codex user data purge", error)),
        }
    }
    Ok(removed_any)
}

/// Ancillary-only cleanup after the portable app tree is already gone (or
/// never existed). Retries Start Menu shortcut + Apps & Features entry removal
/// without touching an install directory. Optional user-data purge.
pub fn cleanup_portable_metadata(
    purge_user_data: bool,
) -> Result<PortableUninstallReport, EngineError> {
    let mut notes = Vec::new();
    let removed_shortcut = match remove_start_menu_shortcut() {
        Ok((removed, note)) => {
            notes.extend(note);
            removed
        }
        Err(err) => {
            notes.push(format!("Start Menu shortcut cleanup failed: {err}"));
            false
        }
    };
    let removed_uninstall_entry = match remove_uninstall_entry() {
        Ok((removed, note)) => {
            notes.extend(note);
            removed
        }
        Err(err) => {
            notes.push(format!(
                "Apps & Features uninstall entry cleanup failed: {err}"
            ));
            false
        }
    };
    // User-data purge is ancillary: a failure must not abort the whole cleanup
    // report (matches the MSIX uninstall path — partial success + recovery CTA).
    let purged_user_data = if purge_user_data {
        match purge_codex_user_data(&mut notes) {
            Ok(purged) => purged,
            Err(err) => {
                notes.push(format!("User data cleanup failed: {err}"));
                false
            }
        }
    } else {
        false
    };
    let partial = notes.iter().any(|note| note.contains("cleanup failed"));
    Ok(PortableUninstallReport {
        success: true,
        partial,
        install_root: String::new(),
        removed_files: false,
        removed_shortcut,
        removed_uninstall_entry,
        purged_user_data,
        message: if partial {
            "Portable metadata cleanup completed with warnings.".to_string()
        } else {
            "Portable metadata cleanup completed.".to_string()
        },
        notes,
    })
}

pub fn uninstall_portable(
    install_root: &Path,
    purge_user_data: bool,
) -> Result<PortableUninstallReport, EngineError> {
    let path = install_root.display();
    log::info!("portable uninstall start path={path}");
    close_codex_gracefully_for_root(30, install_root)?;

    let removed_files = remove_path_with_retry("remove portable install", install_root)
        .map_err(|e| io_err("remove portable install", e))?;

    let mut meta = cleanup_portable_metadata(purge_user_data)?;
    // Preserve install-root context on the combined report.
    meta.install_root = install_root.to_string_lossy().into_owned();
    meta.removed_files = removed_files;
    meta.message = if meta.partial {
        "Portable Codex uninstall completed with cleanup warnings.".to_string()
    } else {
        "Portable Codex uninstall completed.".to_string()
    };
    let path = &meta.install_root;
    log::info!("portable uninstall completed path={path}");
    Ok(meta)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Mutex, OnceLock};
    use zip::write::SimpleFileOptions;

    static TEST_COUNTER: AtomicU64 = AtomicU64::new(1);
    static TEST_ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    fn canonical_temp_dir() -> PathBuf {
        let temp = std::env::temp_dir();
        #[cfg(windows)]
        {
            temp
        }
        #[cfg(not(windows))]
        {
            fs::canonicalize(&temp).unwrap_or(temp)
        }
    }

    fn temp_test_dir(name: &str) -> PathBuf {
        let id = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir =
            canonical_temp_dir().join(format!("codex-portable-{name}-{}-{id}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn fake_pe_image(subsystem: u16) -> Vec<u8> {
        let mut bytes = vec![0_u8; 0x300];
        bytes[..2].copy_from_slice(b"MZ");
        bytes[0x3c..0x40].copy_from_slice(&0x80_u32.to_le_bytes());
        bytes[0x80..0x84].copy_from_slice(b"PE\0\0");
        bytes[0x80 + 20..0x80 + 22].copy_from_slice(&0xE0_u16.to_le_bytes());
        let optional = 0x80 + 24;
        bytes[optional..optional + 2].copy_from_slice(&0x20B_u16.to_le_bytes());
        bytes[optional + 68..optional + 70].copy_from_slice(&subsystem.to_le_bytes());
        bytes
    }

    fn test_env_lock() -> std::sync::MutexGuard<'static, ()> {
        TEST_ENV_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .expect("test environment lock")
    }

    #[cfg(windows)]
    fn assert_powershell_parses(script: &str) {
        use std::process::{Command, Stdio};

        let parser = r#"[Console]::InputEncoding=[Text.UTF8Encoding]::new($false);[Console]::OutputEncoding=[Text.UTF8Encoding]::new($false);$tokens=$null;$errors=$null;[void][System.Management.Automation.Language.Parser]::ParseInput([Console]::In.ReadToEnd(),[ref]$tokens,[ref]$errors);if($errors.Count -gt 0){$errors | Out-String | Write-Error;exit 1}"#;
        let mut child = Command::new(powershell_exe())
            .args(["-NoProfile", "-NonInteractive", "-Command", parser])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .expect("start PowerShell parser");
        child
            .stdin
            .take()
            .expect("PowerShell stdin")
            .write_all(script.as_bytes())
            .expect("write PowerShell script");
        let output = child
            .wait_with_output()
            .expect("wait for PowerShell parser");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    fn usb_profile_never_launches_or_publishes_a_direct_shell_entry() {
        assert_eq!(
            PortableInstallOptions::usb_offline(),
            PortableInstallOptions {
                close_running: false,
                launch_health_check: false,
                relaunch: false,
                create_shell_metadata: false,
            }
        );
    }

    #[test]
    fn portable_startup_patch_is_fixed_width_and_idempotent() {
        let root = temp_test_dir("startup-patch");
        let resources = root.join("resources");
        fs::create_dir_all(&resources).unwrap();
        let app_asar = resources.join("app.asar");
        let original = b"header getCurrentPackageFamily bootstrap OT()?.getCurrentPackageFamily() await o.initialize(); trailer";
        fs::write(&app_asar, original).unwrap();

        assert!(ensure_portable_codex_startup_compatibility(&root).unwrap());
        let patched = fs::read(&app_asar).unwrap();
        assert_eq!(patched.len(), original.len());
        assert!(!patched
            .windows(CODEX_ASAR_PACKAGE_FAMILY_CALL.len())
            .any(|window| window == CODEX_ASAR_PACKAGE_FAMILY_CALL));
        assert!(patched
            .windows(CODEX_ASAR_PACKAGE_FAMILY_PATCH_PREFIX.len())
            .any(|window| window == CODEX_ASAR_PACKAGE_FAMILY_PATCH_PREFIX));
        assert!(!ensure_portable_codex_startup_compatibility(&root).unwrap());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn codex_app_tools_mcp_uses_bundled_gui_node_and_preserves_other_settings() {
        let root = temp_test_dir("codex-app-tools-mcp");
        let resources = root.join("resources");
        let plugin_dir = resources.join("plugins/openai-bundled/plugins/codex-app-tools");
        let node_path = resources.join("cua_node/bin/node.exe");
        fs::create_dir_all(&plugin_dir).unwrap();
        fs::create_dir_all(node_path.parent().unwrap()).unwrap();
        fs::write(&node_path, fake_pe_image(3)).unwrap();
        let original = serde_json::json!({
            "mcpServers": {
                "codex_app": {
                    "command": "cmd.exe",
                    "args": ["/d", "/s", "/c", "call", "./scripts/launch_codex_app_tools_mcp.cmd", "./server.mjs"],
                    "cwd": ".",
                    "enabled": false,
                    "env_vars": { "EXAMPLE": "preserved" },
                    "approval_policy": "user"
                }
            },
            "pluginMetadata": { "keep": true }
        });
        fs::write(
            plugin_dir.join(".mcp.json"),
            serde_json::to_vec_pretty(&original).unwrap(),
        )
        .unwrap();

        let (sidecars_patched, node_subsystem) =
            patch_console_subsystem_executables(&root).unwrap();
        assert!(sidecars_patched);
        assert_eq!(node_subsystem, Some(2));
        assert!(patch_codex_app_tools_mcp_config(&root, node_subsystem).unwrap());

        let patched: serde_json::Value =
            serde_json::from_slice(&fs::read(plugin_dir.join(".mcp.json")).unwrap()).unwrap();
        let server = &patched["mcpServers"]["codex_app"];
        assert_eq!(
            server["command"],
            serde_json::Value::String(CODEX_APP_TOOLS_MCP_NODE_COMMAND.to_string())
        );
        assert_eq!(server["args"], serde_json::json!(["./server.mjs"]));
        assert_eq!(server["cwd"], ".");
        assert_eq!(server["enabled"], false);
        assert_eq!(server["env_vars"]["EXAMPLE"], "preserved");
        assert_eq!(server["approval_policy"], "user");
        assert_eq!(patched["pluginMetadata"]["keep"], true);

        let resolved_node = plugin_dir.join(CODEX_APP_TOOLS_MCP_NODE_COMMAND);
        assert_eq!(
            fs::canonicalize(resolved_node).unwrap(),
            fs::canonicalize(node_path).unwrap()
        );
        assert!(!patch_codex_app_tools_mcp_config(&root, Some(2)).unwrap());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn codex_app_tools_mcp_leaves_unknown_launcher_shape_untouched() {
        let root = temp_test_dir("codex-app-tools-mcp-unknown");
        let resources = root.join("resources");
        let plugin_dir = resources.join("plugins/openai-bundled/plugins/codex-app-tools");
        fs::create_dir_all(&plugin_dir).unwrap();
        let config = serde_json::json!({
            "mcpServers": {
                "codex_app": {
                    "command": "custom-host.exe",
                    "args": ["./server.mjs"],
                    "cwd": "."
                }
            }
        });
        let bytes = serde_json::to_vec(&config).unwrap();
        let path = plugin_dir.join(".mcp.json");
        fs::write(&path, &bytes).unwrap();

        assert!(!patch_codex_app_tools_mcp_config(&root, None).unwrap());
        assert_eq!(fs::read(&path).unwrap(), bytes);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn portable_console_spawn_patch_is_fixed_width_and_idempotent() {
        let root = temp_test_dir("console-spawn-patch");
        let resources = root.join("resources");
        fs::create_dir_all(&resources).unwrap();
        let app_asar = resources.join("app.asar");
        let original = b"header stdio:[`pipe`,`pipe`,`pipe`],env:this.options.env trailer";
        fs::write(&app_asar, original).unwrap();

        assert!(ensure_portable_codex_startup_compatibility(&root).unwrap());
        let patched = fs::read(&app_asar).unwrap();
        assert_eq!(patched.len(), original.len());
        assert!(!patched
            .windows(CODEX_ASAR_CONSOLE_SPAWN_CALLS[0].0.len())
            .any(|window| window == CODEX_ASAR_CONSOLE_SPAWN_CALLS[0].0));
        assert!(patched
            .windows(CODEX_ASAR_CONSOLE_SPAWN_SENTINEL.len())
            .any(|window| window == CODEX_ASAR_CONSOLE_SPAWN_SENTINEL));
        assert!(!ensure_portable_codex_startup_compatibility(&root).unwrap());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn portable_console_spawn_patch_covers_codex_cwd_shape() {
        let root = temp_test_dir("console-spawn-patch-cwd");
        let resources = root.join("resources");
        fs::create_dir_all(&resources).unwrap();
        let app_asar = resources.join("app.asar");
        let call = CODEX_ASAR_CONSOLE_SPAWN_CALLS
            .iter()
            .find(|(needle, _)| needle.ends_with(b",cwd:this.options.cwd"))
            .expect("cwd patch shape");
        fs::write(
            &app_asar,
            [b"header ".as_slice(), call.0, b" trailer"].concat(),
        )
        .unwrap();

        assert!(ensure_portable_codex_startup_compatibility(&root).unwrap());
        let patched = fs::read(&app_asar).unwrap();
        assert!(!patched.windows(call.0.len()).any(|window| window == call.0));
        assert!(patched
            .windows(CODEX_ASAR_CONSOLE_SPAWN_CWD_SENTINEL.len())
            .any(|window| window == CODEX_ASAR_CONSOLE_SPAWN_CWD_SENTINEL));
        assert!(!ensure_portable_codex_startup_compatibility(&root).unwrap());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn portable_generic_console_spawn_patch_is_fixed_width_and_idempotent() {
        let root = temp_test_dir("generic-console-spawn-patch");
        let resources = root.join("resources");
        fs::create_dir_all(&resources).unwrap();
        let app_asar = resources.join("app.asar");
        let (call, replacement) = CODEX_ASAR_GENERIC_SPAWN_CALLS[0];
        fs::write(
            &app_asar,
            [b"header ".as_slice(), call, b" trailer"].concat(),
        )
        .unwrap();

        assert!(ensure_portable_codex_startup_compatibility(&root).unwrap());
        let patched = fs::read(&app_asar).unwrap();
        assert_eq!(
            patched.len(),
            b"header ".len() + call.len() + b" trailer".len()
        );
        assert!(!patched.windows(call.len()).any(|window| window == call));
        assert!(patched
            .windows(replacement.len())
            .any(|window| window == replacement));
        assert!(patched
            .windows(CODEX_ASAR_GENERIC_SPAWN_SENTINEL.len())
            .any(|window| window == CODEX_ASAR_GENERIC_SPAWN_SENTINEL));
        assert!(!ensure_portable_codex_startup_compatibility(&root).unwrap());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn portable_generic_console_spawn_patch_keeps_windows_command_resolution() {
        let (original, replacement) = CODEX_ASAR_GENERIC_SPAWN_CALLS[0];
        assert!(replacement.len() <= original.len());
        assert!(replacement
            .windows(b"n.qn.resolve(e,{env:o})??e".len())
            .any(|window| window == b"n.qn.resolve(e,{env:o})??e"));
        assert!(replacement
            .windows(b"windowsHide:1".len())
            .any(|window| window == b"windowsHide:1"));
        assert!(replacement
            .windows(b"o.ComSpec".len())
            .any(|window| window == b"o.ComSpec"));
        assert!(replacement
            .windows(b"r?.stdin==null?".len())
            .any(|window| window == b"r?.stdin==null?"));
    }

    #[test]
    fn portable_console_subsystem_patch_is_fixed_width_and_idempotent() {
        let root = temp_test_dir("console-subsystem-patch");
        let resources = root.join("resources");
        fs::create_dir_all(&resources).unwrap();
        let nt_offset = 0x80_usize;
        let optional_offset = nt_offset + 24;
        let subsystem_offset = optional_offset + 68;
        let mut original = vec![0_u8; optional_offset + 0xf0];
        original[..2].copy_from_slice(b"MZ");
        original[0x3c..0x40].copy_from_slice(&(nt_offset as u32).to_le_bytes());
        original[nt_offset..nt_offset + 4].copy_from_slice(b"PE\0\0");
        original[nt_offset + 20..nt_offset + 22].copy_from_slice(&0xf0_u16.to_le_bytes());
        original[optional_offset..optional_offset + 2].copy_from_slice(&0x20b_u16.to_le_bytes());
        original[subsystem_offset..subsystem_offset + 2].copy_from_slice(&3_u16.to_le_bytes());
        for name in CODEX_CONSOLE_SUBSYSTEM_EXECUTABLES {
            let path = resources.join(name);
            fs::create_dir_all(path.parent().expect("sidecar parent")).unwrap();
            fs::write(path, &original).unwrap();
        }

        assert!(ensure_portable_codex_startup_compatibility(&root).unwrap());
        for name in CODEX_CONSOLE_SUBSYSTEM_EXECUTABLES {
            let patched = fs::read(resources.join(name)).unwrap();
            assert_eq!(patched.len(), original.len());
            assert_eq!(read_u16_le(&patched, subsystem_offset), Some(2));
        }
        assert!(!ensure_portable_codex_startup_compatibility(&root).unwrap());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn portable_updater_patch_is_fixed_width_when_package_query_is_absent() {
        let root = temp_test_dir("updater-patch");
        let resources = root.join("resources");
        fs::create_dir_all(&resources).unwrap();
        let app_asar = resources.join("app.asar");
        let original = b"header getCurrentPackageFamily bootstrap await o.initialize(); trailer";
        fs::write(&app_asar, original).unwrap();

        assert!(ensure_portable_codex_startup_compatibility(&root).unwrap());
        let patched = fs::read(&app_asar).unwrap();
        assert_eq!(patched.len(), original.len());
        assert!(!patched
            .windows(CODEX_ASAR_UPDATER_CALLS[0].len())
            .any(|window| window == CODEX_ASAR_UPDATER_CALLS[0]));
        assert!(patched
            .windows(CODEX_ASAR_UPDATER_PATCH_PREFIX.len())
            .any(|window| window == CODEX_ASAR_UPDATER_PATCH_PREFIX));
        assert!(!ensure_portable_codex_startup_compatibility(&root).unwrap());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn portable_i18n_patch_enables_bundled_chinese_and_is_idempotent() {
        let root = temp_test_dir("i18n-patch");
        let resources = root.join("resources");
        fs::create_dir_all(&resources).unwrap();
        let app_asar = resources.join("app.asar");
        let original =
            b"other get(`enable_i18n`,!0) webview zh-CN get(`enable_i18n`,!1) locale bundle";
        fs::write(&app_asar, original).unwrap();

        assert!(ensure_portable_codex_locale_compatibility(&root, Some("zh-CN")).unwrap());
        let patched = fs::read(&app_asar).unwrap();
        assert_eq!(patched.len(), original.len());
        assert!(!patched
            .windows(b"get(`enable_i18n`,!1)".len())
            .any(|window| window == b"get(`enable_i18n`,!1)"));
        assert!(
            patched
                .windows(b"get(`enable_i18n`,!0)".len())
                .filter(|window| *window == b"get(`enable_i18n`,!0)")
                .count()
                >= 2
        );
        assert!(!ensure_portable_codex_locale_compatibility(&root, Some("zh-CN")).unwrap());
        assert!(!ensure_portable_codex_locale_compatibility(&root, Some("en")).unwrap());

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn portable_shell_ownership_record_merges_paths_without_credentials() {
        let _env_lock = test_env_lock();
        let root = temp_test_dir("shell-ownership");
        let local = root.join("LocalAppData");
        let previous = std::env::var_os("LOCALAPPDATA");
        std::env::set_var("LOCALAPPDATA", &local);

        let install_root_a = root.join("apps/Codex-a");
        let install_root_b = root.join("apps/Codex-b");
        let shortcut_a = root.join("shell/Codex.lnk");
        let shortcut_b = root.join("shell/ChatGPT.lnk");
        record_portable_shell_ownership(&install_root_a, &shortcut_a).unwrap();
        record_portable_shell_ownership(&install_root_b, &shortcut_b).unwrap();

        let path = portable_shell_ownership_path().unwrap();
        let record: PortableShellOwnership =
            serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        assert_eq!(record.schema_version, PORTABLE_SHELL_OWNERSHIP_SCHEMA);
        assert_eq!(record.product, PORTABLE_SHELL_OWNERSHIP_PRODUCT);
        assert_eq!(record.install_roots.len(), 2);
        assert_eq!(record.shortcut_paths.len(), 2);
        assert!(record
            .shortcut_paths
            .iter()
            .all(|value| !value.contains("KEY") && !value.contains("SECRET")));

        match previous {
            Some(value) => std::env::set_var("LOCALAPPDATA", value),
            None => std::env::remove_var("LOCALAPPDATA"),
        }
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn portable_payload_extraction_copies_only_declared_app_tree() {
        let root = temp_test_dir("optimized-extract");
        let package = root.join("fixture.msix");
        let work = root.join("work");
        fs::create_dir_all(&work).unwrap();

        let file = fs::File::create(&package).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options = SimpleFileOptions::default();
        let manifest = r#"<Package><Identity Name="OpenAI.Codex" Publisher="CN=X" Version="1.0.0.0" ProcessorArchitecture="x64"/><Applications><Application Id="App" Executable="app/ChatGPT.exe"/></Applications></Package>"#;
        zip.start_file("AppxManifest.xml", options).unwrap();
        zip.write_all(manifest.as_bytes()).unwrap();
        zip.start_file("assets/not-copied.bin", options).unwrap();
        zip.write_all(b"no").unwrap();
        zip.start_file("app/ChatGPT.exe", options).unwrap();
        zip.write_all(b"entry").unwrap();
        zip.start_file("app/resources/helper.bin", options).unwrap();
        zip.write_all(b"helper").unwrap();
        zip.finish().unwrap();

        let progress = RefCell::new(Vec::new());
        let prepared = prepare_portable_payload(
            &package,
            &work,
            Some(&|update| {
                progress.borrow_mut().push(update);
                Ok(())
            }),
        )
        .unwrap();
        assert!(prepared.payload_dir.join("ChatGPT.exe").is_file());
        assert!(prepared.payload_dir.join("resources/helper.bin").is_file());
        assert!(!prepared.payload_dir.join("assets/not-copied.bin").exists());
        assert!(prepared.payload_dir.join("AppxManifest.xml").is_file());
        assert!(!work.join("extracted").exists());
        let progress = progress.into_inner();
        assert!(!progress.is_empty());
        assert_eq!(progress.first().unwrap().completed_bytes, 0);
        assert_eq!(progress.last().unwrap().completed_bytes, 11);
        assert_eq!(progress.last().unwrap().total_bytes, 11);
        assert_eq!(progress.last().unwrap().files_completed, 2);
        assert!(progress
            .windows(2)
            .all(|window| window[1].completed_bytes >= window[0].completed_bytes));
        fs::remove_dir_all(root).unwrap();
    }

    /// Release QA hook: exercises the exact USB extraction/transaction path
    /// against a real, already verified MSIX without starting Codex or writing
    /// shell metadata. It is opt-in because the package is roughly 800 MB.
    #[test]
    #[ignore = "set CODEX_TEST_REAL_MSIX and run this test explicitly"]
    fn real_usb_payload_extracts_transactionally() {
        let package = std::env::var_os("CODEX_TEST_REAL_MSIX")
            .map(PathBuf::from)
            .expect("CODEX_TEST_REAL_MSIX must name the verified real MSIX");
        assert!(package.is_file(), "{}", package.display());
        let root = temp_test_dir("real-usb-msix");
        let install_root = root.join("apps/Codex");
        let mut boundaries = Vec::new();
        let report = install_portable_from_msix_with_observer_options(
            &package,
            &install_root,
            PortableInstallOptions::usb_offline(),
            &mut |boundary| {
                boundaries.push(boundary);
                Ok(())
            },
        )
        .unwrap();

        assert!(report.success);
        assert!(!report.relaunched);
        assert!(!report.shortcut_created);
        assert!(!report.uninstall_entry_created);
        assert!(installed_app_exe(&install_root).is_some());
        assert!(install_root.join("AppxManifest.xml").is_file());
        assert!(boundaries
            .iter()
            .any(|boundary| matches!(boundary, PortableBoundary::AfterMoveNew { .. })));
        assert!(!root.join("apps/.codex-app-manager-staging").exists());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn portable_directory_rename_retries_transient_windows_lock_errors() {
        let attempts = Cell::new(0usize);
        let sleeps = RefCell::new(Vec::new());

        rename_with_retry(
            "test rename",
            Path::new("source"),
            Path::new("destination"),
            || {
                let attempt = attempts.get() + 1;
                attempts.set(attempt);
                if attempt < 3 {
                    Err(io::Error::from_raw_os_error(32))
                } else {
                    Ok(())
                }
            },
            |duration| sleeps.borrow_mut().push(duration),
        )
        .unwrap();

        assert_eq!(attempts.get(), 3);
        assert_eq!(
            sleeps.into_inner(),
            vec![Duration::from_millis(50), Duration::from_millis(100)]
        );
    }

    #[test]
    fn portable_directory_rename_recognizes_all_transient_windows_codes() {
        for code in [5, 32, 33] {
            assert!(is_transient_windows_fs_error(
                &io::Error::from_raw_os_error(code)
            ));
        }
        assert!(!is_transient_windows_fs_error(
            &io::Error::from_raw_os_error(2)
        ));
    }

    #[test]
    fn portable_directory_rename_does_not_retry_permanent_errors() {
        let attempts = Cell::new(0usize);
        let sleeps = Cell::new(0usize);

        let err = rename_with_retry(
            "test rename",
            Path::new("source"),
            Path::new("destination"),
            || {
                attempts.set(attempts.get() + 1);
                Err(io::Error::from_raw_os_error(2))
            },
            |_| sleeps.set(sleeps.get() + 1),
        )
        .unwrap_err();

        assert_eq!(err.raw_os_error(), Some(2));
        assert_eq!(attempts.get(), 1);
        assert_eq!(sleeps.get(), 0);
    }

    #[test]
    fn portable_directory_rename_retry_is_bounded() {
        let attempts = Cell::new(0usize);
        let sleeps = Cell::new(0usize);

        let err = rename_with_retry(
            "test rename",
            Path::new("source"),
            Path::new("destination"),
            || {
                attempts.set(attempts.get() + 1);
                Err(io::Error::from_raw_os_error(5))
            },
            |_| sleeps.set(sleeps.get() + 1),
        )
        .unwrap_err();

        assert_eq!(err.raw_os_error(), Some(5));
        assert_eq!(attempts.get(), WINDOWS_FS_RETRY_DELAYS_MS.len() + 1);
        assert_eq!(sleeps.get(), WINDOWS_FS_RETRY_DELAYS_MS.len());
    }

    /// When the AppX data container cannot be enumerated, purge fails — this
    /// must stay non-fatal so the portable uninstall can still report partial
    /// success and offer a retry.
    #[test]
    fn user_data_purge_failure_is_non_fatal_in_metadata_cleanup() {
        let _env_lock = test_env_lock();
        let home = temp_test_dir("purge-home");
        // A regular file at `Packages` makes the known AppX data enumeration
        // fail. A regular file at `.codex` itself is a valid removable data
        // target and is intentionally deleted by the production cleanup.
        let local_app_data = home.join("LocalAppData");
        fs::create_dir_all(&local_app_data).unwrap();
        fs::write(local_app_data.join("Packages"), b"not-a-directory").unwrap();
        let prev_user = std::env::var_os("USERPROFILE");
        let prev_home = std::env::var_os("HOME");
        let prev_local = std::env::var_os("LOCALAPPDATA");
        let prev_appdata = std::env::var_os("APPDATA");
        // SAFETY: test-only, serialised by the unique temp dir; restored below.
        std::env::set_var("USERPROFILE", &home);
        std::env::set_var("HOME", &home);
        std::env::set_var("LOCALAPPDATA", home.join("LocalAppData"));
        std::env::set_var("APPDATA", home.join("AppData"));
        let report = cleanup_portable_metadata(true).expect("purge failure must not Err");
        match prev_user {
            Some(v) => std::env::set_var("USERPROFILE", v),
            None => std::env::remove_var("USERPROFILE"),
        }
        match prev_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
        match prev_local {
            Some(v) => std::env::set_var("LOCALAPPDATA", v),
            None => std::env::remove_var("LOCALAPPDATA"),
        }
        match prev_appdata {
            Some(v) => std::env::set_var("APPDATA", v),
            None => std::env::remove_var("APPDATA"),
        }
        assert!(report.success);
        assert!(report.partial, "purge IO failure should mark partial");
        assert!(!report.purged_user_data);
        assert!(report
            .notes
            .iter()
            .any(|n| n.contains("User data cleanup failed")));
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn user_data_purge_removes_known_profile_and_appx_paths_without_broad_wipe() {
        let _env_lock = test_env_lock();
        let home = temp_test_dir("purge-all");
        let local = home.join("LocalAppData");
        let roaming = home.join("AppData");
        let package = local.join("Packages").join("OpenAI.Codex_2p2nqsd0c76g0");
        let unrelated_package = local.join("Packages").join("OpenAI.Codex_otherpublisher");
        let local_openai = local.join("OpenAI").join("Codex");
        let local_logs = local.join("Codex").join("Logs");
        let roaming_openai = roaming.join("OpenAI").join("Codex");
        let roaming_codex = roaming.join("Codex");
        let installed_skills = home.join(".agents").join("skills");
        let recovery_evidence = local
            .join("CodexAppxDataBackups")
            .join("OpenAI.Codex_2p2nqsd0c76g0");
        for path in [
            home.join(".codex"),
            package.clone(),
            unrelated_package.clone(),
            local_openai.clone(),
            local_logs.clone(),
            roaming_openai.clone(),
            roaming_codex.clone(),
            installed_skills.clone(),
            recovery_evidence.clone(),
        ] {
            fs::create_dir_all(&path).unwrap();
            fs::write(path.join("marker.txt"), b"codex test data").unwrap();
        }
        fs::write(
            local.join("Codex").join("keep.txt"),
            b"unrelated local data",
        )
        .unwrap();

        let previous = [
            ("USERPROFILE", std::env::var_os("USERPROFILE")),
            ("HOME", std::env::var_os("HOME")),
            ("LOCALAPPDATA", std::env::var_os("LOCALAPPDATA")),
            ("APPDATA", std::env::var_os("APPDATA")),
        ];
        std::env::set_var("USERPROFILE", &home);
        std::env::set_var("HOME", &home);
        std::env::set_var("LOCALAPPDATA", &local);
        std::env::set_var("APPDATA", &roaming);

        let mut notes = Vec::new();
        let purged = purge_codex_user_data(&mut notes).expect("known data paths should purge");

        for (name, value) in previous {
            match value {
                Some(value) => std::env::set_var(name, value),
                None => std::env::remove_var(name),
            }
        }

        assert!(purged);
        for path in [
            home.join(".codex"),
            installed_skills,
            package,
            local_openai,
            local_logs,
            roaming_openai,
            roaming_codex,
        ] {
            assert!(!path.exists(), "purge left known path: {}", path.display());
        }
        assert!(
            unrelated_package.exists(),
            "purge removed an unrelated package"
        );
        assert!(local.join("Codex").join("keep.txt").exists());
        assert!(
            recovery_evidence.exists(),
            "the primary purge deleted AppX recovery evidence before system cleanup"
        );
        let _ = fs::remove_dir_all(home);
    }

    #[test]
    fn cleanup_boundary_rejects_relative_and_parent_traversal() {
        assert!(validate_cleanup_path(Path::new("relative/.codex")).is_err());
        let root = canonical_temp_dir().join("codex-cleanup-boundary-missing");
        assert!(validate_cleanup_path(&root.join("child")).is_ok());
        assert!(validate_cleanup_path(&root.join("..").join("escape")).is_err());
    }

    #[test]
    fn unknown_portable_metadata_is_explicitly_preserved() {
        let report = metadata_cleanup_result("preserved", "other-user-shortcut").unwrap();
        assert!(!report.0);
        assert!(report.1.unwrap().contains("ownership"));
        assert!(metadata_cleanup_result("query-failed", "entry").is_err());
    }

    #[cfg(windows)]
    #[test]
    fn portable_registry_collision_is_not_manager_ownership() {
        let script = format!(
            r#"{}
function Get-ItemProperty {{ [pscustomobject]@{{ InstallLocation='C:\OtherUserApp'; DisplayName='Codex'; Publisher='OpenAI'; UninstallString='uninstall.exe' }} }}
if ($null -eq (Get-OwnedPortableEntry)) {{ 'preserved' }} else {{ 'wrongly-owned' }}
"#,
            portable_metadata_ownership_script()
        );
        assert_eq!(run_powershell(&script).unwrap().trim(), "preserved");
        let script = format!(
            r#"{}
function Get-ItemProperty {{ [pscustomobject]@{{ InstallLocation='C:\OwnedCodex'; CodexAppManagerProduct='codex-app-manager' }} }}
if ($null -ne (Get-OwnedPortableEntry)) {{ 'owned' }} else {{ 'missing' }}
"#,
            portable_metadata_ownership_script()
        );
        assert_eq!(run_powershell(&script).unwrap().trim(), "owned");
    }

    #[cfg(windows)]
    #[test]
    fn portable_shell_cleanup_script_covers_legacy_locations_and_names() {
        let script = portable_metadata_ownership_script();
        for needle in [
            "ChatGPT.lnk",
            "CCSwitch.lnk",
            "Codex Switch.lnk",
            "$env:PUBLIC",
            "portable-shell-ownership.json",
            "codex-app-manager:managed-portable",
        ] {
            assert!(
                script.contains(needle),
                "script lost required token: {needle}"
            );
        }
        let parsed = format!("{script}\n'parsed'");
        assert_eq!(run_powershell(&parsed).unwrap().trim(), "parsed");
    }

    #[cfg(windows)]
    #[test]
    fn portable_shell_create_and_apps_features_scripts_parse() {
        let install_root = Path::new(r"C:\CodexUsb\apps\Codex");
        let shortcut = Path::new(
            r"C:\Users\Fixture\AppData\Roaming\Microsoft\Windows\Start Menu\Programs\Codex.lnk",
        );
        let exe = install_root.join("ChatGPT.exe");
        let create = create_start_menu_shortcut_script(install_root, shortcut, &exe);
        assert!(create.contains("SetAccessRule"));
        assert!(create.contains("FullControl"));
        assert_powershell_parses(&create);

        let uninstall = portable_uninstall_script(install_root);
        for needle in [
            "ChatGPT.lnk",
            "$env:PUBLIC",
            "portable-shell-ownership.json",
        ] {
            assert!(
                uninstall.contains(needle),
                "missing uninstall token: {needle}"
            );
        }
        assert_powershell_parses(&uninstall);

        let stale = portable_stale_shortcut_script();
        assert!(stale.contains("Test-PortableShortcutOwned"));
        assert!(stale.contains("Test-Path -LiteralPath $target"));
        assert_powershell_parses(&stale);
    }

    #[test]
    fn cleanup_boundary_does_not_follow_linked_ancestor() {
        let root = temp_test_dir("purge-link");
        let outside = root.join("outside");
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("keep.txt"), b"outside must survive").unwrap();
        let link = root.join("link");
        #[cfg(windows)]
        let created = std::os::windows::fs::symlink_dir(&outside, &link);
        #[cfg(not(windows))]
        let created = std::os::unix::fs::symlink(&outside, &link);
        if let Err(error) = created {
            #[cfg(windows)]
            if error.raw_os_error() == Some(1314) {
                fs::remove_dir_all(&root).unwrap();
                return;
            }
            panic!("create test link: {error}");
        }
        assert!(validate_cleanup_path(&link.join("missing-child")).is_err());
        assert!(remove_user_data_path(&link).is_err());
        assert_eq!(
            fs::read(outside.join("keep.txt")).unwrap(),
            b"outside must survive"
        );
        #[cfg(windows)]
        fs::remove_dir(link).unwrap();
        #[cfg(not(windows))]
        fs::remove_file(link).unwrap();
        fs::remove_dir_all(root).unwrap();
    }

    fn write_fake_msix(path: &Path) {
        let file = fs::File::create(path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let opts = SimpleFileOptions::default();
        zip.start_file("AppxManifest.xml", opts).unwrap();
        zip.write_all(
            br#"<Package xmlns="http://schemas.microsoft.com/appx/manifest/foundation/windows10">
  <Identity Name="OpenAI.Codex" Publisher="CN=OpenAI OpCo, LLC" Version="26.602.3474.0" ProcessorArchitecture="x64" />
</Package>"#,
        )
        .unwrap();
        zip.start_file("VFS/ProgramFilesX64/Codex/Codex.exe", opts)
            .unwrap();
        zip.write_all(b"fake exe").unwrap();
        zip.start_file("VFS/ProgramFilesX64/Codex/resources/app.asar", opts)
            .unwrap();
        zip.write_all(b"fake asar").unwrap();
        zip.finish().unwrap();
    }

    fn write_fake_rebranded_msix(path: &Path) {
        // Post-rebrand layout: manifest entry is app/ChatGPT.exe while a legacy
        // Codex.exe still ships next to it (as on the real 26.707.x package).
        let file = fs::File::create(path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let opts = SimpleFileOptions::default();
        zip.start_file("AppxManifest.xml", opts).unwrap();
        zip.write_all(
            br#"<Package xmlns="http://schemas.microsoft.com/appx/manifest/foundation/windows10">
  <Identity Name="OpenAI.Codex" Publisher="CN=OpenAI OpCo, LLC" Version="26.707.3748.0" ProcessorArchitecture="x64" />
  <Applications>
    <Application Id="App" Executable="app/ChatGPT.exe" EntryPoint="Windows.FullTrustApplication" />
  </Applications>
</Package>"#,
        )
        .unwrap();
        zip.start_file("app/ChatGPT.exe", opts).unwrap();
        zip.write_all(b"fake entry exe").unwrap();
        zip.start_file("app/Codex.exe", opts).unwrap();
        zip.write_all(b"legacy compat exe").unwrap();
        zip.start_file("app/resources/app.asar", opts).unwrap();
        zip.write_all(b"fake asar").unwrap();
        zip.finish().unwrap();
    }

    #[test]
    fn installs_portable_payload_from_msix_layout() {
        let root = canonical_temp_dir().join(format!("codex-portable-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let msix = root.join("codex.msix");
        let install_root = root.join("Codex");
        write_fake_msix(&msix);

        let report = install_portable_from_msix_inner(&msix, &install_root, false, false).unwrap();
        assert!(report.success);
        assert!(install_root.join("Codex.exe").exists());
        assert!(install_root.join("resources/app.asar").exists());
        assert!(install_root.join("AppxManifest.xml").exists());
        assert_eq!(report.version, "26.602.3474.0");

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn installs_rebranded_portable_payload_by_manifest_entry() {
        let root = canonical_temp_dir().join(format!(
            "codex-portable-rebrand-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let msix = root.join("codex.msix");
        let install_root = root.join("Codex");
        write_fake_rebranded_msix(&msix);

        let report = install_portable_from_msix_inner(&msix, &install_root, false, false).unwrap();
        assert!(report.success);
        // Payload root is the manifest entry's directory; both exes ride along.
        assert!(install_root.join("ChatGPT.exe").exists());
        assert!(install_root.join("Codex.exe").exists());
        assert!(install_root.join("resources/app.asar").exists());
        assert_eq!(report.version, "26.707.3748.0");
        // The entry executable resolves to ChatGPT.exe, not the legacy binary.
        assert_eq!(
            installed_app_exe(&install_root),
            Some(install_root.join("ChatGPT.exe"))
        );
        assert_eq!(
            report.executable_path.as_deref(),
            Some(install_root.join("ChatGPT.exe").to_string_lossy().as_ref())
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn installed_app_exe_prefers_manifest_then_known_names() {
        let root =
            canonical_temp_dir().join(format!("codex-portable-exe-probe-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();

        // No manifest, legacy layout → Codex.exe via known-name probe.
        fs::write(root.join("Codex.exe"), b"legacy").unwrap();
        assert_eq!(installed_app_exe(&root), Some(root.join("Codex.exe")));

        // Both names present without a manifest → the newer entry name wins.
        fs::write(root.join("ChatGPT.exe"), b"entry").unwrap();
        assert_eq!(installed_app_exe(&root), Some(root.join("ChatGPT.exe")));

        // A manifest declaring the legacy entry overrides the probe order.
        fs::write(
            root.join("AppxManifest.xml"),
            br#"<Package xmlns="http://schemas.microsoft.com/appx/manifest/foundation/windows10">
  <Identity Name="OpenAI.Codex" Publisher="CN=X" Version="1.0.0.0" ProcessorArchitecture="x64" />
  <Applications><Application Id="App" Executable="app\Codex.exe" /></Applications>
</Package>"#,
        )
        .unwrap();
        assert_eq!(installed_app_exe(&root), Some(root.join("Codex.exe")));

        // A declared-but-missing entry means the install is broken: never
        // silently fall back to a leftover binary that happens to exist.
        fs::write(
            root.join("AppxManifest.xml"),
            br#"<Package xmlns="http://schemas.microsoft.com/appx/manifest/foundation/windows10">
  <Identity Name="OpenAI.Codex" Publisher="CN=X" Version="1.0.0.0" ProcessorArchitecture="x64" />
  <Applications><Application Id="App" Executable="app\Gone.exe" /></Applications>
</Package>"#,
        )
        .unwrap();
        assert_eq!(installed_app_exe(&root), None);

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn install_fails_when_declared_entry_is_missing_from_payload() {
        // Manifest declares app/ChatGPT.exe but the payload only carries the
        // legacy app/Codex.exe (e.g. the entry was quarantined). Selecting the
        // leftover binary would health-check the wrong thing — must error out.
        let root = canonical_temp_dir().join(format!(
            "codex-portable-missing-entry-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let msix = root.join("codex.msix");
        {
            let file = fs::File::create(&msix).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            let opts = SimpleFileOptions::default();
            zip.start_file("AppxManifest.xml", opts).unwrap();
            zip.write_all(
                br#"<Package xmlns="http://schemas.microsoft.com/appx/manifest/foundation/windows10">
  <Identity Name="OpenAI.Codex" Publisher="CN=OpenAI OpCo, LLC" Version="26.707.3748.0" ProcessorArchitecture="x64" />
  <Applications>
    <Application Id="App" Executable="app/ChatGPT.exe" EntryPoint="Windows.FullTrustApplication" />
  </Applications>
</Package>"#,
            )
            .unwrap();
            zip.start_file("app/Codex.exe", opts).unwrap();
            zip.write_all(b"legacy only").unwrap();
            zip.finish().unwrap();
        }

        let install_root = root.join("Codex");
        let err = install_portable_from_msix_inner(&msix, &install_root, false, false).unwrap_err();
        assert!(
            err.to_string().contains("missing from the payload"),
            "unexpected error: {err}"
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn replaces_existing_portable_and_removes_rollback_backup() {
        let root = canonical_temp_dir().join(format!(
            "codex-portable-replace-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let msix = root.join("codex.msix");
        let install_root = root.join("Codex");
        write_fake_msix(&msix);

        fs::create_dir_all(&install_root).unwrap();
        fs::write(install_root.join("Codex.exe"), b"old exe").unwrap();
        fs::write(install_root.join("old-marker.txt"), b"old").unwrap();

        let report = install_portable_from_msix_inner(&msix, &install_root, false, false).unwrap();
        assert!(report.success);
        assert!(report.backup_path.is_none());
        assert!(!fs::read_dir(&root).unwrap().any(|entry| entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with("Codex.rollback")));
        assert!(!install_root.join("old-marker.txt").exists());
        assert!(install_root.join("resources/app.asar").exists());

        let _ = fs::remove_dir_all(&root);
    }

    #[cfg(windows)]
    #[test]
    fn health_check_detects_immediate_exit_entry() {
        // whoami.exe exits instantly — models a broken payload that CreateProcess
        // accepts then immediately dies. The health check must fail closed.
        let root =
            canonical_temp_dir().join(format!("codex-portable-liveness-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let windir = std::env::var_os("WINDIR").unwrap_or_else(|| "C:\\Windows".into());
        let whoami = PathBuf::from(windir).join("System32").join("whoami.exe");
        if !whoami.is_file() {
            let _ = fs::remove_dir_all(&root);
            return;
        }
        fs::copy(&whoami, root.join("ChatGPT.exe")).unwrap();
        fs::write(
            root.join("AppxManifest.xml"),
            br#"<Package xmlns="http://schemas.microsoft.com/appx/manifest/foundation/windows10">
  <Identity Name="OpenAI.Codex" Publisher="CN=X" Version="1.0.0.0" ProcessorArchitecture="x64" />
  <Applications><Application Id="App" Executable="app\ChatGPT.exe" /></Applications>
</Package>"#,
        )
        .unwrap();

        let err = health_check_portable_install(&root, true, true).unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("exited immediately"),
            "unexpected error: {msg}"
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn restore_previous_install_removes_failed_payload() {
        let root = canonical_temp_dir().join(format!(
            "codex-portable-rollback-test-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let install_root = root.join("Codex");
        let backup = root.join("Codex.rollback-test");

        fs::create_dir_all(&backup).unwrap();
        fs::write(backup.join("old-marker.txt"), b"old").unwrap();
        fs::create_dir_all(&install_root).unwrap();
        fs::write(install_root.join("new-marker.txt"), b"new").unwrap();

        restore_previous_install(&install_root, &backup, true).unwrap();
        assert!(install_root.join("old-marker.txt").exists());
        assert!(!install_root.join("new-marker.txt").exists());
        assert!(!backup.exists());

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn fault_after_move_old_leaves_crash_window_for_recovery() {
        let root = canonical_temp_dir().join(format!(
            "codex-portable-fault-after-old-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let msix = root.join("codex.msix");
        let install_root = root.join("Codex");
        write_fake_msix(&msix);
        fs::create_dir_all(&install_root).unwrap();
        fs::write(install_root.join("old-marker.txt"), b"old").unwrap();

        inject_portable_fault(Some(PortableFault::AfterMoveOld));
        let err = install_portable_from_msix_inner(&msix, &install_root, false, false).unwrap_err();
        assert!(err.to_string().contains("after-move-old"));
        // Crash window: install missing, rollback backup present, staging payload present.
        assert!(!install_root.exists());
        let backup = fs::read_dir(&root)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .find(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("Codex.rollback-"))
            });
        assert!(backup.is_some(), "rollback backup must remain");
        assert!(backup.unwrap().join("old-marker.txt").exists());

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn fault_before_move_old_leaves_install_intact() {
        let root = canonical_temp_dir().join(format!(
            "codex-portable-fault-before-old-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let msix = root.join("codex.msix");
        let install_root = root.join("Codex");
        write_fake_msix(&msix);
        fs::create_dir_all(&install_root).unwrap();
        fs::write(install_root.join("old-marker.txt"), b"old").unwrap();

        inject_portable_fault(Some(PortableFault::BeforeMoveOld));
        let err = install_portable_from_msix_inner(&msix, &install_root, false, false).unwrap_err();
        assert!(err.to_string().contains("before-move-old"));
        assert!(install_root.join("old-marker.txt").exists());

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn observer_failure_before_move_old_preserves_previous_install() {
        let root = canonical_temp_dir().join(format!(
            "codex-portable-precommit-rollback-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let msix = root.join("codex.msix");
        let install_root = root.join("apps/Codex");
        write_fake_msix(&msix);
        fs::create_dir_all(&install_root).unwrap();
        fs::write(install_root.join("old-marker.txt"), b"old").unwrap();

        let err = install_portable_from_msix_with_observer(
            &msix,
            &install_root,
            false,
            false,
            &mut |boundary| {
                if matches!(boundary, PortableBoundary::BeforeMoveOld { .. }) {
                    Err(EngineError::Install(
                        "USB bundle precommit rejected".to_string(),
                    ))
                } else {
                    Ok(())
                }
            },
        )
        .unwrap_err();

        assert!(err.to_string().contains("USB bundle precommit rejected"));
        assert!(install_root.join("old-marker.txt").exists());
        assert!(!root.join("apps/.codex-app-manager-staging").exists());
        assert!(!fs::read_dir(root.join("apps"))
            .unwrap()
            .filter_map(|entry| entry.ok())
            .any(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("Codex.rollback-")
            }));

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn observer_sees_rename_boundaries() {
        let root =
            canonical_temp_dir().join(format!("codex-portable-observer-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let msix = root.join("codex.msix");
        let install_root = root.join("Codex");
        write_fake_msix(&msix);
        let mut kinds = Vec::new();
        install_portable_from_msix_with_observer(&msix, &install_root, false, false, &mut |b| {
            kinds.push(match b {
                PortableBoundary::BeforeMoveOld { .. } => "before-old",
                PortableBoundary::AfterMoveOld { .. } => "after-old",
                PortableBoundary::BeforeMoveNew { .. } => "before-new",
                PortableBoundary::AfterMoveNew { .. } => "after-new",
                PortableBoundary::BeforeRollback { .. } => "before-rollback",
                PortableBoundary::RollbackCompleted { .. } => "rollback-completed",
            });
            Ok(())
        })
        .unwrap();
        assert_eq!(
            kinds,
            ["before-old", "after-old", "before-new", "after-new"]
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn successful_fresh_install_rollback_emits_absent_state_evidence() {
        let root = canonical_temp_dir().join(format!(
            "codex-portable-fresh-rollback-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let msix = root.join("codex.msix");
        let install_root = root.join("Codex");
        write_fake_msix(&msix);
        let mut kinds = Vec::new();

        inject_portable_fault(Some(PortableFault::AfterMoveNew));
        let err = install_portable_from_msix_with_observer(
            &msix,
            &install_root,
            false,
            false,
            &mut |boundary| {
                kinds.push(match boundary {
                    PortableBoundary::BeforeMoveOld { .. } => "before-old",
                    PortableBoundary::AfterMoveOld { .. } => "after-old",
                    PortableBoundary::BeforeMoveNew { .. } => "before-new",
                    PortableBoundary::AfterMoveNew { .. } => "after-new",
                    PortableBoundary::BeforeRollback { .. } => "before-rollback",
                    PortableBoundary::RollbackCompleted { .. } => "rollback-completed",
                });
                Ok(())
            },
        )
        .unwrap_err();

        assert!(err.to_string().contains("absent state was restored"));
        assert_eq!(
            kinds,
            [
                "before-old",
                "after-old",
                "before-new",
                "after-new",
                "before-rollback",
                "rollback-completed"
            ]
        );
        assert!(
            !install_root.exists(),
            "a fully rolled-back fresh install must be absent and safe to retry"
        );
        assert!(
            !fs::read_dir(&root).unwrap().any(|entry| entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with("Codex.rollback-")),
            "a fresh rollback must not leave rollback material"
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn missing_upgrade_backup_never_reports_rollback_completion() {
        let root = canonical_temp_dir().join(format!(
            "codex-portable-missing-rollback-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let install_root = root.join("Codex");
        let backup = root.join("Codex.rollback-missing");
        fs::create_dir_all(&install_root).unwrap();
        fs::write(install_root.join("new-marker.txt"), b"new").unwrap();
        let mut boundaries = Vec::new();

        let err = rollback_install_error(
            &install_root,
            &backup,
            true,
            &mut |boundary| {
                boundaries.push(boundary);
                Ok(())
            },
            portable_fault_err("after-move-new"),
        );

        assert!(err.to_string().contains("rollback backup is missing"));
        assert_eq!(boundaries.len(), 1);
        assert!(matches!(
            boundaries[0],
            PortableBoundary::BeforeRollback {
                had_previous: true,
                ..
            }
        ));
        assert!(
            !boundaries
                .iter()
                .any(|boundary| matches!(boundary, PortableBoundary::RollbackCompleted { .. })),
            "failed rollback must not emit completion evidence"
        );
        assert!(
            install_root.join("new-marker.txt").exists(),
            "failed rollback must preserve the current tree for recovery"
        );

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn successful_upgrade_rollback_restores_old_tree_before_evidence() {
        let root = canonical_temp_dir().join(format!(
            "codex-portable-upgrade-rollback-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let install_root = root.join("Codex");
        let backup = root.join("Codex.rollback-old");
        fs::create_dir_all(&install_root).unwrap();
        fs::write(install_root.join("new-marker.txt"), b"new").unwrap();
        fs::create_dir_all(&backup).unwrap();
        fs::write(backup.join("old-marker.txt"), b"old").unwrap();
        let mut boundaries = Vec::new();
        let mut saw_restored_tree = false;

        let err = rollback_install_error(
            &install_root,
            &backup,
            true,
            &mut |boundary| {
                match boundary {
                    PortableBoundary::BeforeRollback {
                        had_previous: true, ..
                    } => {
                        boundaries.push("before-rollback");
                        assert!(install_root.join("new-marker.txt").exists());
                        assert!(backup.join("old-marker.txt").exists());
                    }
                    PortableBoundary::RollbackCompleted {
                        had_previous: true, ..
                    } => {
                        boundaries.push("rollback-completed");
                        saw_restored_tree = install_root.join("old-marker.txt").exists()
                            && !install_root.join("new-marker.txt").exists();
                    }
                    other => panic!("unexpected rollback boundary: {other:?}"),
                }
                Ok(())
            },
            portable_fault_err("after-move-new"),
        );

        assert!(err.to_string().contains("previous install was restored"));
        assert_eq!(boundaries, ["before-rollback", "rollback-completed"]);
        assert!(
            saw_restored_tree,
            "evidence must follow the completed restore"
        );
        assert!(!backup.exists());

        let _ = fs::remove_dir_all(&root);
    }
}
