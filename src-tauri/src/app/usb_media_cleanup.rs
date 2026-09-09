//! Explicit source-kit erasure. Pin identities before verifying hashes; delete
//! the same Windows handles and retain retry metadata if final cleanup fails.
use crate::app::usb_guard::{validate_license, UsbFingerprint, UsbLicense};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, Write};
use std::path::{Component, Path, PathBuf};

const LIMIT: u64 = 128 * 1024;
const RECOVERY_LIMIT: u64 = 64 * 1024 * 1024;
const FILES: &[&str] = &[
    "CodexDesktopSetup.exe",
    "CodexAppManager.exe",
    "manifest.json",
    "usb-manifest.json",
    "README.txt",
    "verify-payload.ps1",
    "verify-usb.ps1",
    "verify-usb-launcher.ps1",
    "ccswitch/CodexSwitch.exe",
    "ccswitch/LICENSE",
    "ccswitch/manifest.json",
    "ccswitch/providers.json",
    "ccswitch/SOURCE-NOTICE.txt",
    "ccswitch/verify-product.ps1",
    "payload/OpenAI.Codex_x64.msix",
    "resources/README.md",
    "resources/ccswitch/CodexSwitch.exe",
    "resources/ccswitch/LICENSE",
    "resources/ccswitch/manifest.json",
    "resources/ccswitch/providers.json",
    "resources/ccswitch/SOURCE-NOTICE.txt",
    "resources/ccswitch/verify-product.ps1",
    "resources/prerequisites/vc_redist.x64.exe",
    "resources/prerequisites/MicrosoftEdgeWebView2RuntimeInstallerX64.exe",
    "apps/README.txt",
    "content/README.txt",
    "content/skills/README.txt",
    "content/mcp/README.txt",
];
const RECOVERY: &[&str] = &[
    "CodexAppManager.exe",
    "CodexDesktopSetup.exe",
    "manifest.json",
    "usb-manifest.json",
    "SHA256SUMS.txt",
    "usb-license.json",
    "usb-license.json.bak",
];
const EXTRA: &[&str] = &["SHA256SUMS.txt", "usb-license.json", "usb-license.json.bak"];
const CAPABILITY_PACK_PREFIX: &str = "content/codex-starter-kit/";
const DIRS: &[&str] = &[
    "content/skills",
    "content/mcp",
    "content",
    "apps",
    "resources/ccswitch",
    "resources/prerequisites",
    "resources",
    "ccswitch",
    "payload",
];

fn is_capability_pack_path(name: &str) -> bool {
    if !name.starts_with(CAPABILITY_PACK_PREFIX) || name.contains('\\') {
        return false;
    }
    let relative = &name[CAPABILITY_PACK_PREFIX.len()..];
    !relative.split('/').any(|segment| {
        segment.is_empty() || segment == "." || segment == ".." || segment.contains(':')
    })
}

fn is_allowed_release_file(name: &str) -> bool {
    FILES.contains(&name) || EXTRA.contains(&name) || is_capability_pack_path(name)
}

fn is_allowed_release_dir(name: &str) -> bool {
    DIRS.contains(&name)
        || name.eq_ignore_ascii_case("content/codex-starter-kit")
        || name.starts_with(CAPABILITY_PACK_PREFIX)
}

fn metadata(path: &Path) -> Result<Option<fs::Metadata>, String> {
    match fs::symlink_metadata(path) {
        Ok(value) => Ok(Some(value)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(format!("无法核验 {}：{error}", path.display())),
    }
}
fn is_reparse(meta: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        meta.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        meta.file_type().is_symlink()
    }
}
fn check_path(path: &Path) -> Result<(), String> {
    if !path.is_absolute()
        || path
            .components()
            .any(|c| matches!(c, Component::ParentDir | Component::CurDir))
    {
        return Err(format!("拒绝非绝对/包含跳转的路径：{}", path.display()));
    }
    for ancestor in path.ancestors() {
        if metadata(ancestor)?.as_ref().is_some_and(is_reparse) {
            return Err(format!("拒绝链接/重解析点：{}", ancestor.display()));
        }
    }
    Ok(())
}
fn validate_scope(bundle: &Path) -> Result<(), String> {
    check_path(bundle)?;
    if bundle.parent().is_none() || bundle.file_name().is_none() {
        return Err("拒绝删除盘根目录".into());
    }
    let canonical = fs::canonicalize(bundle).map_err(|e| e.to_string())?;
    let temp = std::env::temp_dir();
    if let Some(base) = directories::BaseDirs::new() {
        for protected in [
            base.home_dir(),
            base.data_dir(),
            base.data_local_dir(),
            temp.as_path(),
        ] {
            if fs::canonicalize(protected).ok().as_ref() == Some(&canonical) {
                return Err("用户目录/系统暂存根目录不能作为安装包擦除范围".into());
            }
        }
    }
    Ok(())
}

/// Directories share READ|WRITE but not DELETE, preventing ancestor replacement.
/// Files share READ only: writers/renamers cannot change the hash-verified object.
struct Pin {
    path: PathBuf,
    file: File,
    #[cfg_attr(not(windows), allow(dead_code))]
    readonly: bool,
    directory: bool,
}
impl Pin {
    fn open(path: &Path, directory: bool, deletable: bool) -> Result<Self, String> {
        check_path(path)?;
        #[cfg(windows)]
        let file = {
            use std::os::windows::fs::OpenOptionsExt;
            use windows_sys::Win32::Foundation::GENERIC_READ;
            use windows_sys::Win32::Storage::FileSystem::*;
            let readonly = metadata(path)?
                .ok_or("目标文件/目录已消失")?
                .permissions()
                .readonly();
            let access = if directory {
                // Metadata-only handles do not enforce the intended sharing
                // exclusion. Directory read-data/list access is required.
                FILE_READ_ATTRIBUTES | FILE_LIST_DIRECTORY
            } else {
                GENERIC_READ
            };
            let access = access
                | if deletable {
                    0x0001_0000
                        | if readonly && !directory {
                            FILE_WRITE_ATTRIBUTES
                        } else {
                            0
                        }
                } else {
                    0
                };
            OpenOptions::new()
                .access_mode(access)
                .share_mode(if directory {
                    FILE_SHARE_READ | FILE_SHARE_WRITE
                } else if deletable {
                    FILE_SHARE_READ
                } else {
                    // Preflight is read-only and runs while Manager is mapped.
                    // Cleanup reopens strict pins after that process exits.
                    FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE
                })
                .custom_flags(
                    FILE_FLAG_OPEN_REPARSE_POINT
                        | if directory {
                            FILE_FLAG_BACKUP_SEMANTICS
                        } else {
                            0
                        },
                )
                .open(path)
                .map_err(|error| format!("无法固定目标/目标被占用 {}：{error}", path.display()))?
        };
        #[cfg(not(windows))]
        let file = {
            let _ = deletable;
            File::open(path).map_err(|e| e.to_string())?
        };
        let meta = file.metadata().map_err(|e| e.to_string())?;
        if is_reparse(&meta) || (directory && !meta.is_dir()) || (!directory && !meta.is_file()) {
            return Err(format!("拒绝重解析点/意外文件类型：{}", path.display()));
        }
        Ok(Self {
            path: path.into(),
            file,
            readonly: meta.permissions().readonly(),
            directory,
        })
    }
    fn bytes(&mut self, limit: u64) -> Result<Vec<u8>, String> {
        self.file.rewind().map_err(|e| e.to_string())?;
        let mut bytes = Vec::new();
        Read::by_ref(&mut self.file)
            .take(limit + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() as u64 > limit {
            return Err(format!("清单/重试文件过大：{}", self.path.display()));
        }
        Ok(bytes)
    }
    fn text(&mut self) -> Result<String, String> {
        String::from_utf8(self.bytes(LIMIT)?)
            .map(|text| text.trim_start_matches('\u{feff}').to_owned())
            .map_err(|e| e.to_string())
    }
    fn hash(&mut self) -> Result<String, String> {
        self.file.rewind().map_err(|e| e.to_string())?;
        let mut hash = Sha256::new();
        let mut buf = [0u8; 128 * 1024];
        loop {
            let n = self.file.read(&mut buf).map_err(|e| e.to_string())?;
            if n == 0 {
                break;
            }
            hash.update(&buf[..n]);
        }
        Ok(hash.finalize().iter().map(|b| format!("{b:02x}")).collect())
    }
    fn mark_delete(&self) -> Result<(), String> {
        #[cfg(windows)]
        {
            use std::os::windows::io::AsRawHandle;
            use windows_sys::Win32::Storage::FileSystem::*;
            let handle = self.file.as_raw_handle();
            let mut original = FILE_BASIC_INFO::default();
            let readonly = self.readonly && !self.directory;
            if readonly {
                if unsafe {
                    GetFileInformationByHandleEx(
                        handle,
                        FileBasicInfo,
                        (&mut original as *mut FILE_BASIC_INFO).cast(),
                        std::mem::size_of::<FILE_BASIC_INFO>() as u32,
                    )
                } == 0
                {
                    return Err(format!("读取文件属性失败：{}", io::Error::last_os_error()));
                }
                let mut writable = original;
                writable.FileAttributes &= !FILE_ATTRIBUTE_READONLY;
                if writable.FileAttributes == 0 {
                    writable.FileAttributes = FILE_ATTRIBUTE_NORMAL;
                }
                if unsafe {
                    SetFileInformationByHandle(
                        handle,
                        FileBasicInfo,
                        (&writable as *const FILE_BASIC_INFO).cast(),
                        std::mem::size_of::<FILE_BASIC_INFO>() as u32,
                    )
                } == 0
                {
                    return Err(format!("清理只读文件失败：{}", io::Error::last_os_error()));
                }
            }
            let disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
            let mut error = None;
            for attempt in 0..4 {
                if unsafe {
                    SetFileInformationByHandle(
                        handle,
                        FileDispositionInfo,
                        (&disposition as *const FILE_DISPOSITION_INFO).cast(),
                        std::mem::size_of::<FILE_DISPOSITION_INFO>() as u32,
                    )
                } != 0
                {
                    return Ok(());
                }
                error = Some(io::Error::last_os_error());
                if attempt < 3 {
                    std::thread::sleep(std::time::Duration::from_millis(300));
                }
            }
            if readonly
                && unsafe {
                    SetFileInformationByHandle(
                        handle,
                        FileBasicInfo,
                        (&original as *const FILE_BASIC_INFO).cast(),
                        std::mem::size_of::<FILE_BASIC_INFO>() as u32,
                    )
                } == 0
            {
                return Err(format!(
                    "删除失败且只读属性恢复失败 {}：{}；{}",
                    self.path.display(),
                    error.unwrap(),
                    io::Error::last_os_error()
                ));
            }
            Err(format!(
                "删除 {} 失败：{}",
                self.path.display(),
                error.unwrap()
            ))
        }
        #[cfg(not(windows))]
        {
            let result = if self.directory {
                fs::remove_dir(&self.path)
            } else {
                fs::remove_file(&self.path)
            };
            result.map_err(|e| e.to_string())
        }
    }
}
fn parse_checksums(text: &str) -> Result<BTreeMap<String, String>, String> {
    let mut entries = BTreeMap::new();
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        let (hash, name) = line.split_once("  ").ok_or("无效 SHA256 清单行")?;
        if hash.len() != 64
            || !hash.bytes().all(|c| c.is_ascii_hexdigit())
            || !is_allowed_release_file(name)
        {
            return Err(format!("清单包含不允许删除的条目：{name}"));
        }
        if entries
            .insert(name.into(), hash.to_ascii_lowercase())
            .is_some()
        {
            return Err(format!("清单重复条目：{name}"));
        }
    }
    for name in [
        "CodexAppManager.exe",
        "CodexDesktopSetup.exe",
        "usb-manifest.json",
    ] {
        if !entries.contains_key(name) {
            return Err(format!("缺少发行包身份条目：{name}"));
        }
    }
    Ok(entries)
}

struct BundlePins {
    root: PathBuf,
    _ancestors: Vec<Pin>,
    dirs: BTreeMap<String, Pin>,
    files: BTreeMap<String, Pin>,
}
impl BundlePins {
    fn inventory(
        &mut self,
        relative: &str,
        deletable: bool,
        allowed_runtime_dir: Option<&str>,
    ) -> Result<Vec<String>, String> {
        let dir = self.root.join(relative);
        self.dirs
            .insert(relative.into(), Pin::open(&dir, true, deletable)?);
        let mut blockers = Vec::new();
        for entry in fs::read_dir(&dir).map_err(|e| e.to_string())? {
            let path = entry.map_err(|e| e.to_string())?.path();
            let meta = metadata(&path)?.ok_or("枚举期间目标消失，请重试")?;
            let name = path
                .strip_prefix(&self.root)
                .map_err(|e| e.to_string())?
                .to_string_lossy()
                .replace('\\', "/");
            if is_reparse(&meta) {
                blockers.push(format!("拒绝链接/重解析点：{name}"));
                continue;
            }
            if meta.is_dir() {
                if allowed_runtime_dir.is_some_and(|allowed| name.eq_ignore_ascii_case(allowed)) {
                    if deletable {
                        blockers.push(format!(
                            "受管程序目录尚未由主卸载流程清空，保留安装介质：{name}"
                        ));
                        continue;
                    }
                    // The program tree is mutable runtime output and is not a
                    // release checksum input. Pin its exact, non-reparse root
                    // during the read-only preflight, but never descend into
                    // it or authorize source-media cleanup while it exists.
                    self.dirs.insert(name, Pin::open(&path, true, false)?);
                    continue;
                }
                if !is_allowed_release_dir(&name) {
                    blockers.push(format!("保留非发行包目录：{name}"));
                    continue;
                }
                blockers.extend(self.inventory(&name, deletable, allowed_runtime_dir)?);
            } else {
                if !is_allowed_release_file(&name) {
                    blockers.push(format!("保留非发行包文件：{name}"));
                    continue;
                }
                self.files.insert(name, Pin::open(&path, false, deletable)?);
            }
        }
        Ok(blockers)
    }
    fn open_with_runtime(
        bundle: &Path,
        deletable: bool,
        allowed_runtime_dir: Option<&str>,
        expected_license: Option<&UsbFingerprint>,
    ) -> Result<Self, String> {
        validate_scope(bundle)?;
        let mut ancestors = Vec::new();
        for ancestor in bundle
            .ancestors()
            .skip(1)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
        {
            ancestors.push(Pin::open(ancestor, true, false)?);
        }
        let mut pins = Self {
            root: bundle.into(),
            _ancestors: ancestors,
            dirs: BTreeMap::new(),
            files: BTreeMap::new(),
        };
        let blockers = pins.inventory("", deletable, allowed_runtime_dir)?;
        if !blockers.is_empty() {
            return Err(format!(
                "检测到 {} 个不能自动删除的安装介质项目：\n- {}",
                blockers.len(),
                blockers.join("\n- ")
            ));
        }
        let manifest: serde_json::Value = serde_json::from_str(
            &pins
                .files
                .get_mut("usb-manifest.json")
                .ok_or("缺少 usb-manifest.json")?
                .text()?,
        )
        .map_err(|e| e.to_string())?;
        if manifest["product"].as_str() != Some("codex-usb-oneclick")
            || manifest["schemaVersion"].as_u64() != Some(2)
        {
            return Err("不是本产品可清理的 U 盘发行包".into());
        }
        let sums = parse_checksums(
            &pins
                .files
                .get_mut("SHA256SUMS.txt")
                .ok_or("缺少 SHA256SUMS.txt")?
                .text()?,
        )?;
        for (name, file) in &mut pins.files {
            if let Some(expected) = sums.get(name) {
                if &file.hash()? != expected {
                    return Err(format!("文件已被修改，保留：{name}"));
                }
            } else if !EXTRA.contains(&name.as_str()) && !is_capability_pack_path(name) {
                return Err(format!("保留清单外文件：{name}"));
            }
        }
        let license_present = pins.files.contains_key("usb-license.json");
        let license_backup_present = pins.files.contains_key("usb-license.json.bak");
        if expected_license.is_some() && !license_present {
            return Err(if license_backup_present {
                "主许可文件缺失，仅发现备份；为避免删除不完整授权状态，已保留：usb-license.json.bak"
                    .into()
            } else {
                "当前授权 U 盘的主许可文件已消失；为避免删除被替换的介质，已保留：usb-license.json"
                    .into()
            });
        }
        if license_present || license_backup_present {
            let expected = expected_license.ok_or(
                "发现运行期许可文件，但没有当前授权 U 盘指纹；为避免误删，已保留：usb-license.json",
            )?;
            for name in ["usb-license.json", "usb-license.json.bak"] {
                let Some(file) = pins.files.get_mut(name) else {
                    continue;
                };
                let license: UsbLicense = serde_json::from_str(&file.text()?)
                    .map_err(|error| format!("许可文件 JSON 无效，已保留 {name}：{error}"))?;
                uuid::Uuid::parse_str(&license.installation_id)
                    .map_err(|_| format!("许可文件 installationId 无效，已保留：{name}"))?;
                license
                    .issued_at
                    .parse::<u64>()
                    .map_err(|_| format!("许可文件 issuedAt 无效，已保留：{name}"))?;
                validate_license(&license, expected)
                    .map_err(|error| format!("许可文件身份校验失败，已保留 {name}：{error}"))?;
            }
        }
        Ok(pins)
    }

    fn open(bundle: &Path, deletable: bool) -> Result<Self, String> {
        Self::open_with_runtime(bundle, deletable, None, None)
    }
    fn remove_file(&mut self, name: &str) -> Result<(), String> {
        let Some(file) = self.files.get(name) else {
            return Ok(());
        };
        file.mark_delete()?;
        drop(self.files.remove(name));
        verify_absent(&self.root.join(name))
    }
    fn remove_dir(&mut self, name: &str) -> Result<(), String> {
        let Some(dir) = self.dirs.get(name) else {
            return Ok(());
        };
        dir.mark_delete()?;
        drop(self.dirs.remove(name));
        verify_absent(&self.root.join(name))
    }
    fn recovery_bytes(&mut self) -> Result<BTreeMap<String, Vec<u8>>, String> {
        let mut remaining = RECOVERY_LIMIT;
        let mut bytes = BTreeMap::new();
        for name in RECOVERY {
            if let Some(file) = self.files.get_mut(*name) {
                let data = file.bytes(remaining)?;
                remaining = remaining
                    .checked_sub(data.len() as u64)
                    .ok_or("恢复资料超过 64 MB")?;
                bytes.insert(name.to_string(), data);
            }
        }
        Ok(bytes)
    }
    fn restore(&mut self, recovery: &BTreeMap<String, Vec<u8>>) -> Vec<String> {
        if !self.dirs.contains_key("") {
            return vec!["安装包根目录已消失，不能安全恢复入口；请使用原始发行包重试".into()];
        }
        let mut errors = Vec::new();
        for (name, bytes) in recovery {
            if self.files.contains_key(name) {
                continue;
            }
            let path = self.root.join(name);
            let outcome = (|| -> Result<(), String> {
                check_path(&path)?;
                let mut options = OpenOptions::new();
                options.write(true).create_new(true);
                #[cfg(windows)]
                {
                    use std::os::windows::fs::OpenOptionsExt;
                    options.share_mode(0);
                }
                let mut file = options
                    .open(&path)
                    .map_err(|e| format!("恢复重试文件失败，未覆盖既有内容 {name}：{e}"))?;
                file.write_all(bytes)
                    .and_then(|_| file.sync_all())
                    .map_err(|e| e.to_string())
            })();
            if let Err(error) = outcome {
                errors.push(error);
            }
        }
        // Preserve offline-mode recognition after payload deletion. The root
        // handle prevents ancestor replacement; create_dir never follows links.
        let payload = self.root.join("payload");
        match metadata(&payload) {
            Ok(None) => {
                if let Err(e) = fs::create_dir(&payload) {
                    errors.push(format!("恢复空 payload 失败：{e}"));
                }
            }
            Ok(Some(meta)) if !meta.is_dir() || is_reparse(&meta) => {
                errors.push("payload 被替换，未覆盖".into())
            }
            Err(e) => errors.push(e),
            _ => {}
        }
        errors
    }
}
fn verify_absent(path: &Path) -> Result<(), String> {
    if metadata(path)?.is_some() {
        return Err(format!("删除后仍存在：{}", path.display()));
    }
    Ok(())
}
/// Full read-only inventory/hash preflight before application mutation.
pub fn preflight_bundle(bundle: &Path) -> Result<(), String> {
    BundlePins::open(bundle, false).map(|_| ())
}

/// Preflight used only before the primary portable uninstall. The one exact
/// manager-owned runtime directory may still exist at this point. Final media
/// cleanup reopens the bundle in strict mode and refuses deletion unless the
/// primary uninstall has removed that directory first.
pub fn preflight_bundle_before_uninstall(
    bundle: &Path,
    runtime_root: Option<&Path>,
    expected_license: &UsbFingerprint,
) -> Result<(), String> {
    let allowed = if let Some(runtime_root) = runtime_root {
        let expected = bundle.join("apps").join("Codex");
        if runtime_root != expected {
            return Err(format!(
                "拒绝非当前发行包的程序目录：{}",
                runtime_root.display()
            ));
        }
        Some("apps/Codex")
    } else {
        None
    };
    BundlePins::open_with_runtime(bundle, false, allowed, Some(expected_license)).map(|_| ())
}
/// Only the detached helper with validated source identity calls this function.
pub fn cleanup_bundle(bundle: &Path) -> Result<Vec<String>, String> {
    cleanup_inner(bundle, None, || Ok(()))
}
/// Delete a live product bundle that contains the manager-created mutable USB
/// license.  The fingerprint is captured from the active, process-owned lease
/// before the guard is stopped and is rechecked by the detached helper.
pub fn cleanup_authorized_bundle(
    bundle: &Path,
    expected_license: &UsbFingerprint,
) -> Result<Vec<String>, String> {
    cleanup_inner(bundle, Some(expected_license), || Ok(()))
}
fn cleanup_inner(
    bundle: &Path,
    expected_license: Option<&UsbFingerprint>,
    before_final_commit: impl FnOnce() -> Result<(), String>,
) -> Result<Vec<String>, String> {
    let mut pins = BundlePins::open_with_runtime(bundle, true, None, expected_license)?;
    let recovery = pins.recovery_bytes()?;
    let mut notes = Vec::new();
    let result = (|| {
        let ordinary = pins
            .files
            .keys()
            .filter(|name| !RECOVERY.contains(&name.as_str()))
            .cloned()
            .collect::<Vec<_>>();
        for name in ordinary {
            pins.remove_file(&name)?;
            notes.push(format!("已清理：{name}"));
        }
        // Finish all potentially nonempty directories BEFORE metadata commit.
        // The capability pack has a recursive source tree, so remove pinned
        // directories deepest-first; static release roots remain covered by
        // the same inventory and checksum gates.
        let mut directories = pins
            .dirs
            .keys()
            .filter(|name| !name.is_empty())
            .cloned()
            .collect::<Vec<_>>();
        directories.sort_by(|left, right| {
            right
                .matches('/')
                .count()
                .cmp(&left.matches('/').count())
                .then_with(|| right.len().cmp(&left.len()))
        });
        for name in directories {
            pins.remove_dir(&name)?;
        }
        for entry in fs::read_dir(bundle).map_err(|e| e.to_string())? {
            let name = entry
                .map_err(|e| e.to_string())?
                .file_name()
                .to_string_lossy()
                .into_owned();
            if !RECOVERY.contains(&name.as_str()) {
                return Err(format!("出现非发行包内容，保留重试入口：{name}"));
            }
        }
        before_final_commit()?;
        for name in RECOVERY {
            pins.remove_file(name)?;
        }
        pins.remove_dir("")?;
        Ok(())
    })();
    if let Err(error) = result {
        let failures = pins.restore(&recovery);
        return Err(if failures.is_empty() {
            format!("{error}\n已保留/恢复卸载重试入口及身份清单（已删除载荷不自动恢复）")
        } else {
            format!("{error}\n重试入口恢复不完整：{}", failures.join("；"))
        });
    }
    notes.push(format!("已删除经核验的安装包目录：{}", bundle.display()));
    Ok(notes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_parent() -> PathBuf {
        let temp = std::env::temp_dir();
        fs::canonicalize(&temp).unwrap_or(temp)
    }

    fn fixture() -> PathBuf {
        let root = fixture_parent().join(format!(
            "codex-media-cleanup-中文 空格 ' 测试-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir(&root).unwrap();
        let mut list = String::new();
        for (name, content) in [
            ("CodexAppManager.exe", "manager"),
            ("CodexDesktopSetup.exe", "setup"),
            ("manifest.json", "{}"),
            (
                "usb-manifest.json",
                r#"{"schemaVersion":2,"product":"codex-usb-oneclick"}"#,
            ),
            ("payload/OpenAI.Codex_x64.msix", "package"),
        ] {
            let path = root.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(&path, content).unwrap();
            list.push_str(&format!(
                "{}  {name}\n",
                Pin::open(&path, false, false).unwrap().hash().unwrap()
            ));
        }
        fs::write(root.join("SHA256SUMS.txt"), list).unwrap();
        root
    }
    fn fingerprint() -> UsbFingerprint {
        UsbFingerprint {
            volume_serial: 42,
            anchor_sha256: "ab".repeat(32),
            bundle_relative_path: "usb-oneclick-v0.5.24-final".into(),
            anchor_schema: 1,
        }
    }
    fn write_license(root: &Path, expected: &UsbFingerprint, backup: bool) {
        let license = crate::app::usb_guard::new_unsigned_license(expected.clone());
        let bytes = serde_json::to_vec_pretty(&license).unwrap();
        fs::write(root.join("usb-license.json"), &bytes).unwrap();
        if backup {
            fs::write(root.join("usb-license.json.bak"), bytes).unwrap();
        }
    }
    fn discard(root: &Path) {
        let parent = fixture_parent();
        assert_eq!(root.parent(), Some(parent.as_path()));
        assert!(root
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("codex-media-cleanup-"));
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn reject_manifest_traversal_unknown_and_duplicate() {
        let hash = "a".repeat(64);
        for bad in [
            "../notes.txt",
            "C:/Windows/file",
            "resources/../private",
            "ccswitch\\CodexSwitch.exe",
            "content/codex-starter-kit/../private.txt",
            "notes.txt",
            "CodexAppManager.exe:secret",
        ] {
            assert!(parse_checksums(&format!("{hash}  {bad}")).is_err());
        }
        assert!(parse_checksums(&format!(
            "{hash}  CodexAppManager.exe\n{hash}  CodexAppManager.exe"
        ))
        .is_err());
    }
    #[test]
    fn unicode_space_apostrophe_root_cleans_successfully() {
        let root = fixture();
        preflight_bundle(&root).unwrap();
        cleanup_bundle(&root).unwrap();
        assert!(!root.exists());
    }

    #[test]
    fn bundled_capability_pack_tree_is_verified_and_removed() {
        let root = fixture();
        let pack_file = root.join("content/codex-starter-kit/agents/general.toml");
        fs::create_dir_all(pack_file.parent().unwrap()).unwrap();
        fs::write(&pack_file, "name = 'general'\n").unwrap();
        let hash = Pin::open(&pack_file, false, false).unwrap().hash().unwrap();
        let mut sums = fs::read_to_string(root.join("SHA256SUMS.txt")).unwrap();
        sums.push_str(&format!(
            "{hash}  content/codex-starter-kit/agents/general.toml\n"
        ));
        fs::write(root.join("SHA256SUMS.txt"), sums).unwrap();

        preflight_bundle(&root).unwrap();
        cleanup_bundle(&root).unwrap();
        assert!(!root.exists());
    }

    #[test]
    fn unknown_and_modified_files_are_found_before_mutation() {
        let root = fixture();
        fs::write(root.join("my-project.txt"), "user work").unwrap();
        fs::write(root.join("another-user-file.txt"), "user work").unwrap();
        let blockers = preflight_bundle(&root).unwrap_err();
        assert!(blockers.contains("my-project.txt"));
        assert!(blockers.contains("another-user-file.txt"));
        assert!(cleanup_bundle(&root).is_err());
        assert!(root.join("CodexDesktopSetup.exe").exists());
        fs::remove_file(root.join("my-project.txt")).unwrap();
        fs::remove_file(root.join("another-user-file.txt")).unwrap();
        fs::write(root.join("payload/OpenAI.Codex_x64.msix"), "user change").unwrap();
        assert!(preflight_bundle(&root).unwrap_err().contains("修改"));
        assert!(cleanup_bundle(&root).unwrap_err().contains("修改"));
        discard(&root);
    }
    #[test]
    fn unknown_nested_directory_retains_payload() {
        let root = fixture();
        fs::create_dir_all(root.join("payload/my-project")).unwrap();
        assert!(preflight_bundle(&root)
            .unwrap_err()
            .contains("非发行包目录"));
        assert!(cleanup_bundle(&root).is_err());
        assert!(root.join("payload/OpenAI.Codex_x64.msix").exists());
        discard(&root);
    }
    #[test]
    fn absent_optional_payload_is_idempotent() {
        let root = fixture();
        fs::remove_file(root.join("payload/OpenAI.Codex_x64.msix")).unwrap();
        cleanup_bundle(&root).unwrap();
        assert!(!root.exists());
    }

    #[test]
    fn uninstall_preflight_allows_only_exact_runtime_then_cleanup_stays_strict() {
        let root = fixture();
        let expected = fingerprint();
        write_license(&root, &expected, true);
        let runtime = root.join("apps/Codex");
        fs::create_dir_all(&runtime).unwrap();
        fs::write(runtime.join("Codex.exe"), "runtime").unwrap();

        preflight_bundle_before_uninstall(&root, Some(&runtime), &expected).unwrap();
        assert!(preflight_bundle(&root).is_err());
        assert!(cleanup_bundle(&root).unwrap_err().contains("非发行包目录"));
        assert!(runtime.join("Codex.exe").exists());

        fs::remove_dir_all(&runtime).unwrap();
        cleanup_authorized_bundle(&root, &expected).unwrap();
        assert!(!root.exists());
    }

    #[test]
    fn uninstall_preflight_rejects_runtime_outside_exact_bundle_slot() {
        let root = fixture();
        let expected = fingerprint();
        let wrong = root.join("content/Codex");
        fs::create_dir_all(&wrong).unwrap();
        assert!(
            preflight_bundle_before_uninstall(&root, Some(&wrong), &expected)
                .unwrap_err()
                .contains("非当前发行包")
        );
        discard(&root);
    }

    #[test]
    fn manager_license_and_atomic_backup_are_validated_then_removed_last() {
        let root = fixture();
        let expected = fingerprint();
        write_license(&root, &expected, true);

        let unauthenticated = preflight_bundle(&root).unwrap_err();
        assert!(unauthenticated.contains("没有当前授权 U 盘指纹"));
        preflight_bundle_before_uninstall(&root, None, &expected).unwrap();
        cleanup_authorized_bundle(&root, &expected).unwrap();
        assert!(!root.exists());
    }

    #[test]
    fn missing_or_tampered_license_fails_closed_before_payload_mutation() {
        let expected = fingerprint();

        let missing = fixture();
        let error = preflight_bundle_before_uninstall(&missing, None, &expected).unwrap_err();
        assert!(error.contains("主许可文件已消失"));
        assert!(missing.join("payload/OpenAI.Codex_x64.msix").exists());
        discard(&missing);

        let wrong_fingerprint = fixture();
        let mut other = expected.clone();
        other.volume_serial += 1;
        write_license(&wrong_fingerprint, &other, false);
        let error = cleanup_authorized_bundle(&wrong_fingerprint, &expected).unwrap_err();
        assert!(error.contains("指纹不匹配"));
        assert!(wrong_fingerprint
            .join("payload/OpenAI.Codex_x64.msix")
            .exists());
        discard(&wrong_fingerprint);

        let wrong_product = fixture();
        let mut license = crate::app::usb_guard::new_unsigned_license(expected.clone());
        license.product = "not-our-product".into();
        fs::write(
            wrong_product.join("usb-license.json"),
            serde_json::to_vec_pretty(&license).unwrap(),
        )
        .unwrap();
        let error = cleanup_authorized_bundle(&wrong_product, &expected).unwrap_err();
        assert!(error.contains("产品标识不匹配"));
        assert!(wrong_product.join("payload/OpenAI.Codex_x64.msix").exists());
        discard(&wrong_product);
    }

    #[cfg(windows)]
    #[test]
    fn manager_license_readonly_is_removed_but_an_open_handle_blocks_all_mutation() {
        use std::os::windows::fs::OpenOptionsExt;
        let expected = fingerprint();

        let locked_root = fixture();
        write_license(&locked_root, &expected, false);
        let held = OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(locked_root.join("usb-license.json"))
            .unwrap();
        let error = cleanup_authorized_bundle(&locked_root, &expected).unwrap_err();
        assert!(error.contains("占用"), "{error}");
        assert!(locked_root.join("payload/OpenAI.Codex_x64.msix").exists());
        drop(held);
        cleanup_authorized_bundle(&locked_root, &expected).unwrap();

        let readonly_root = fixture();
        write_license(&readonly_root, &expected, true);
        for name in ["usb-license.json", "usb-license.json.bak"] {
            let path = readonly_root.join(name);
            let mut permissions = fs::metadata(&path).unwrap().permissions();
            permissions.set_readonly(true);
            fs::set_permissions(path, permissions).unwrap();
        }
        cleanup_authorized_bundle(&readonly_root, &expected).unwrap();
        assert!(!readonly_root.exists());
    }
    #[cfg(windows)]
    #[test]
    fn locked_executable_fails_before_any_deletion() {
        use std::os::windows::fs::OpenOptionsExt;
        let root = fixture();
        let locked = OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(root.join("CodexDesktopSetup.exe"))
            .unwrap();
        assert!(cleanup_bundle(&root).unwrap_err().contains("占用"));
        assert!(root.join("CodexAppManager.exe").exists());
        assert!(root.join("payload/OpenAI.Codex_x64.msix").exists());
        drop(locked);
        cleanup_bundle(&root).unwrap();
    }
    #[cfg(windows)]
    #[test]
    fn readonly_distribution_file_can_be_removed() {
        let root = fixture();
        let path = root.join("payload/OpenAI.Codex_x64.msix");
        let mut p = fs::metadata(&path).unwrap().permissions();
        p.set_readonly(true);
        fs::set_permissions(&path, p).unwrap();
        cleanup_bundle(&root).unwrap();
        assert!(!root.exists());
    }
    #[cfg(windows)]
    #[test]
    fn pins_deny_directory_rename_file_swap_and_manifest_rewrite() {
        let root = fixture();
        let pins = BundlePins::open(&root, true).unwrap();
        assert!(fs::rename(&root, root.with_extension("moved")).is_err());
        assert!(fs::rename(root.join("payload"), root.join("other")).is_err());
        assert!(fs::write(root.join("SHA256SUMS.txt"), "replace").is_err());
        assert!(fs::rename(root.join("CodexDesktopSetup.exe"), root.join("swapped.exe")).is_err());
        drop(pins);
        cleanup_bundle(&root).unwrap();
    }
    #[test]
    fn late_unknown_file_restores_retry_metadata() {
        let root = fixture();
        let error = cleanup_inner(&root, None, || {
            fs::write(root.join("late-user-file.txt"), "keep").map_err(|e| e.to_string())
        })
        .unwrap_err();
        assert!(error.contains("恢复"));
        for name in [
            "CodexAppManager.exe",
            "CodexDesktopSetup.exe",
            "manifest.json",
            "usb-manifest.json",
            "SHA256SUMS.txt",
        ] {
            assert!(root.join(name).is_file(), "{name}");
        }
        assert_eq!(
            fs::read_to_string(root.join("late-user-file.txt")).unwrap(),
            "keep"
        );
        assert!(root.join("payload").is_dir());
        fs::remove_file(root.join("late-user-file.txt")).unwrap();
        preflight_bundle(&root).unwrap();
        cleanup_bundle(&root).unwrap();
    }
    #[test]
    fn known_folder_roots_are_never_deletion_scopes() {
        assert!(validate_scope(&std::env::temp_dir()).is_err());
        assert!(validate_scope(Path::new(r"C:\")).is_err());
    }

    #[test]
    #[ignore = "Read-only hash audit of local release; never deletes release files"]
    fn actual_release_read_only_preflight() {
        let bundle = std::env::var_os("CODEX_TEST_USB_BUNDLE")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .parent()
                    .unwrap()
                    .join("release/usb-oneclick")
            });
        preflight_bundle(&bundle).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn junction_is_refused_without_touching_target() {
        use std::os::windows::process::CommandExt;
        let root = fixture();
        let outside = fixture();
        let link = root.join("resources");
        let result = std::process::Command::new("cmd.exe")
            .args(["/d", "/c", "mklink", "/J"])
            .arg(&link)
            .arg(&outside)
            .creation_flags(0x08000000)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert!(preflight_bundle(&root).is_err());
        assert!(cleanup_bundle(&root).is_err());
        assert_eq!(
            fs::read_to_string(outside.join("CodexDesktopSetup.exe")).unwrap(),
            "setup"
        );
        // Remove only the fixture junction, never its destination tree.
        fs::remove_dir(&link).unwrap();
        cleanup_bundle(&root).unwrap();
        cleanup_bundle(&outside).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn locked_directory_retains_source_and_payload() {
        let root = fixture();
        let held = Pin::open(&root.join("payload"), true, false).unwrap();
        assert!(cleanup_bundle(&root).is_err());
        assert!(root.join("CodexDesktopSetup.exe").exists());
        assert!(root.join("payload/OpenAI.Codex_x64.msix").exists());
        drop(held);
        cleanup_bundle(&root).unwrap();
    }
}
