use std::fs;
use std::io::{self, Read};
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

fn io_err(context: &str, err: impl ToString) -> EngineError {
    EngineError::Io(format!("{context}: {}", err.to_string()))
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
        let mut out =
            fs::File::create(&out_path).map_err(|e| io_err("create extracted file", e))?;
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
) -> Result<PreparedPortable, EngineError> {
    let extracted = work_dir.join("extracted");
    let payload = work_dir.join("payload");
    if extracted.exists() {
        fs::remove_dir_all(&extracted).map_err(|e| io_err("clear extracted dir", e))?;
    }
    if payload.exists() {
        fs::remove_dir_all(&payload).map_err(|e| io_err("clear payload dir", e))?;
    }
    fs::create_dir_all(&extracted).map_err(|e| io_err("create extracted dir", e))?;

    let manifest_xml = extract_msix(msix_path, &extracted)?;
    let identity = parse_appx_manifest_xml(&manifest_xml)?;
    let exe = find_app_exe(&extracted, &manifest_xml)?;
    let exe_dir = exe.parent().ok_or_else(|| {
        EngineError::Msix("app entry executable had no parent directory".to_string())
    })?;

    copy_dir_all(exe_dir, &payload)?;
    fs::write(payload.join("AppxManifest.xml"), manifest_xml)
        .map_err(|e| io_err("write portable AppxManifest.xml", e))?;

    Ok(PreparedPortable {
        payload_dir: payload,
        identity,
    })
}

#[cfg(all(windows, not(test)))]
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
    let script = format!(
        r#"
$shell = New-Object -ComObject WScript.Shell
$shortcut = $shell.CreateShortcut({shortcut})
$shortcut.TargetPath = {target}
$shortcut.WorkingDirectory = {workdir}
$shortcut.IconLocation = {icon}
$shortcut.Description = 'codex-app-manager:managed-portable'
$shortcut.Save()
"#,
        shortcut = ps_quote(&shortcut.to_string_lossy()),
        target = ps_quote(&exe.to_string_lossy()),
        workdir = ps_quote(&install_root.to_string_lossy()),
        icon = ps_quote(&format!("{},0", exe.to_string_lossy()))
    );
    run_powershell(&script)?;
    Ok(true)
}

#[cfg(any(not(windows), test))]
fn create_start_menu_shortcut(_install_root: &Path) -> Result<bool, EngineError> {
    Ok(false)
}

#[cfg(all(windows, not(test)))]
fn register_uninstall_entry(
    install_root: &Path,
    version: &str,
    estimated_size_kb: u64,
) -> Result<bool, EngineError> {
    // Icon only — the entry works without one, so fall back to the legacy name.
    let exe = installed_app_exe(install_root).unwrap_or_else(|| install_root.join("Codex.exe"));
    let uninstall_script = format!(
        "if ($env:APPDATA) {{ $Shortcut = Join-Path $env:APPDATA 'Microsoft\\Windows\\Start Menu\\Programs\\Codex.lnk'; Remove-Item -LiteralPath $Shortcut -Force -ErrorAction SilentlyContinue }}; Remove-Item -LiteralPath '{}' -Recurse -Force -ErrorAction SilentlyContinue; Remove-Item -LiteralPath 'HKCU:\\Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\Codex' -Recurse -Force -ErrorAction SilentlyContinue",
        install_root.to_string_lossy().replace('\'', "''")
    );
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
"#
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
fn remove_start_menu_shortcut() -> Result<(bool, Option<String>), EngineError> {
    let Some(appdata) = std::env::var_os("APPDATA") else {
        return Ok((false, None));
    };
    let shortcut = PathBuf::from(appdata)
        .join("Microsoft")
        .join("Windows")
        .join("Start Menu")
        .join("Programs")
        .join("Codex.lnk");
    validate_cleanup_path(&shortcut)?;
    let script = format!(
        r#"{}
$shortcutPath = {}
if (-not (Test-Path -LiteralPath $shortcutPath -ErrorAction Stop)) {{ 'absent'; exit 0 }}
$link = (New-Object -ComObject WScript.Shell).CreateShortcut($shortcutPath)
$entry = Get-OwnedPortableEntry
$owned = $link.Description -eq 'codex-app-manager:managed-portable'
if ($entry) {{
  $root = [IO.Path]::GetFullPath([string]$entry.InstallLocation).TrimEnd('\') + '\'
  $target = [IO.Path]::GetFullPath([string]$link.TargetPath)
  $owned = $target.StartsWith($root, [StringComparison]::OrdinalIgnoreCase)
}}
if (-not $owned) {{ 'preserved'; exit 0 }}
Remove-Item -LiteralPath $shortcutPath -Force -ErrorAction Stop
if (Test-Path -LiteralPath $shortcutPath -ErrorAction Stop) {{ throw 'shortcut still present' }}
'removed'
"#,
        portable_metadata_ownership_script(),
        ps_quote(&shortcut.to_string_lossy())
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
        fs::remove_dir_all(install_root)
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

    let prepared = match prepare_portable_payload(msix_path, &work_dir) {
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
/// installation.  The first entry (`~/.codex`) is shared with Codex CLI; the
/// other entries are desktop/AppX caches.  They are only used when the user
/// explicitly turns off "keep my data".  In particular, this list never
/// walks a broad `%LOCALAPPDATA%` tree or follows an arbitrary glob.
fn codex_user_data_paths() -> Result<Vec<PathBuf>, EngineError> {
    let mut paths = Vec::new();
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"));

    if let Some(home) = home {
        paths.push(PathBuf::from(home).join(".codex"));
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

    fn test_env_lock() -> std::sync::MutexGuard<'static, ()> {
        TEST_ENV_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .expect("test environment lock")
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
