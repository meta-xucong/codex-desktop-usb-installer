//! Startup preflight for the user-owned Codex TOML configuration.
//!
//! Windows PowerShell 5.1 writes a UTF-8 BOM for `Set-Content -Encoding UTF8`.
//! Codex's TOML reader rejects that BOM. We repair only a BOM-only defect whose
//! remaining bytes parse as TOML; all other invalid input is left untouched.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::app::{atomic_file, paths};
use crate::errors::AppError;

const UTF8_BOM: &[u8; 3] = b"\xef\xbb\xbf";
const MAX_CONFIG_BYTES: u64 = 4 * 1024 * 1024;
static BACKUP_SEQUENCE: AtomicU64 = AtomicU64::new(1);

/// Check and, only when safe, normalize the current user's Codex config before
/// any manager launch path mutates settings or starts Codex.
pub fn ensure_user_config_ready() -> Result<(), AppError> {
    let home = paths::codex_home_dir()
        .ok_or_else(|| AppError::Engine("无法定位当前用户的 Codex 配置目录".into()))?;
    let path = home.join("config.toml");
    match ensure_config_file(&path) {
        Ok(Some(backup)) => {
            log::warn!(
                "Removed UTF-8 BOM from Codex config after validating TOML; backup={}",
                backup.display()
            );
            Ok(())
        }
        Ok(None) => Ok(()),
        Err(reason) => Err(AppError::Engine(format!(
            "Codex 配置检查未通过，已阻止启动以避免窗口无响应。配置文件：{}。{}",
            path.display(),
            reason
        ))),
    }
}

/// Returns the recovery backup path when it repaired a valid BOM-only file.
fn ensure_config_file(path: &Path) -> Result<Option<PathBuf>, String> {
    let link_metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("无法读取配置文件信息：{error}")),
    };
    let is_symlink = link_metadata.file_type().is_symlink();
    let metadata = if is_symlink {
        fs::metadata(path).map_err(|error| format!("无法读取配置文件目标：{error}"))?
    } else {
        link_metadata
    };
    if !metadata.is_file() {
        return Err("配置路径不是普通文件；原文件未更改。".into());
    }
    if metadata.len() > MAX_CONFIG_BYTES {
        return Err(format!(
            "配置文件超过安全检查上限 {} MiB；原文件未更改。",
            MAX_CONFIG_BYTES / (1024 * 1024)
        ));
    }

    let original = fs::read(path).map_err(|error| format!("无法读取配置文件：{error}"))?;
    let has_bom = original.starts_with(UTF8_BOM);
    let candidate = if has_bom {
        &original[UTF8_BOM.len()..]
    } else {
        original.as_slice()
    };
    validate_toml(candidate).map_err(|reason| {
        if has_bom {
            format!("检测到 UTF-8 BOM，移除 BOM 后仍无法解析 TOML：{reason} 原文件未更改。")
        } else {
            format!("TOML 语法或 UTF-8 编码无效：{reason} 原文件未更改。")
        }
    })?;

    if !has_bom {
        return Ok(None);
    }
    if is_symlink {
        return Err(
            "配置路径是符号链接；为避免替换链接目标，未自动修复 BOM。原文件未更改。".into(),
        );
    }

    let backup = create_preimage_backup(path, &original)?;
    let current = fs::read(path).map_err(|error| {
        format!(
            "建立备份后无法再次读取配置文件：{error}；备份保留在 {}。",
            backup.display()
        )
    })?;
    if current != original {
        return Err(format!(
            "检查期间配置文件已被其他程序修改，未覆盖新内容；检查前副本保留在 {}。",
            backup.display()
        ));
    }

    if let Err(error) = atomic_file::write_atomic_preserving_backup(path, candidate) {
        return Err(format!(
            "无法原子修复配置文件：{error}；原文件未替换，备份保留在 {}。",
            backup.display()
        ));
    }

    let repaired = match fs::read(path) {
        Ok(repaired) => repaired,
        Err(error) => {
            return Err(format!(
                "修复后无法回读配置文件：{error}；为避免覆盖其他程序可能写入的内容，未自动回滚；原始副本位于 {}。",
                backup.display()
            ));
        }
    };
    if repaired.starts_with(UTF8_BOM) || repaired != candidate || validate_toml(&repaired).is_err()
    {
        let restore_result = restore_preimage_if_unchanged(path, candidate, &original, &backup);
        return Err(match restore_result {
            Ok(()) => format!(
                "修复后校验未通过，已恢复原配置；备份位于 {}。",
                backup.display()
            ),
            Err(restore_error) => format!(
                "修复后校验未通过，恢复原配置也失败：{restore_error}；请从备份 {} 恢复。",
                backup.display()
            ),
        });
    }

    Ok(Some(backup))
}

fn validate_toml(bytes: &[u8]) -> Result<(), String> {
    let text = std::str::from_utf8(bytes).map_err(|error| format!("UTF-8 解码失败：{error}"))?;
    toml::from_str::<toml::Value>(text)
        .map(|_| ())
        .map_err(|error| error.to_string())
}

fn create_preimage_backup(path: &Path, bytes: &[u8]) -> Result<PathBuf, String> {
    let parent = path
        .parent()
        .ok_or_else(|| "配置文件路径缺少父目录，无法备份。".to_string())?;
    let modified = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "配置文件名无法安全编码，无法备份。".to_string())?;

    for _ in 0..16 {
        let sequence = BACKUP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let backup = parent.join(format!(
            "{file_name}.bom-recovery-{modified}-{}-{sequence}.bak",
            std::process::id()
        ));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&backup)
        {
            Ok(mut file) => {
                if let Err(error) = file.write_all(bytes).and_then(|()| file.sync_all()) {
                    let _ = fs::remove_file(&backup);
                    return Err(format!("无法写入配置备份：{error}"));
                }
                return Ok(backup);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(format!("无法创建配置备份：{error}")),
        }
    }
    Err("无法创建唯一的配置备份文件。".into())
}

fn restore_preimage_if_unchanged(
    path: &Path,
    expected_current: &[u8],
    bytes: &[u8],
    backup: &Path,
) -> Result<(), String> {
    let current = fs::read(path).map_err(|error| {
        format!(
            "无法确认修复后的配置内容：{error}; 未覆盖当前文件；原始副本仍保留在 {}",
            backup.display()
        )
    })?;
    if current != expected_current {
        return Err(format!(
            "配置文件在修复后又被其他程序修改；未覆盖当前内容；原始副本仍保留在 {}",
            backup.display()
        ));
    }
    atomic_file::write_atomic_preserving_backup(path, bytes)
        .map_err(|error| format!("{error}; 原始副本仍保留在 {}", backup.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU64;

    static TEST_SEQUENCE: AtomicU64 = AtomicU64::new(1);

    struct TestDir(PathBuf);

    impl TestDir {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "codex-config-guard-test-{}-{}",
                std::process::id(),
                TEST_SEQUENCE.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&path).unwrap();
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn write_config(dir: &Path, bytes: &[u8]) -> PathBuf {
        let path = dir.join("config.toml");
        fs::write(&path, bytes).unwrap();
        path
    }

    #[test]
    fn missing_config_is_allowed_without_creating_files() {
        let dir = TestDir::new();
        let path = dir.path().join("config.toml");
        assert_eq!(ensure_config_file(&path).unwrap(), None);
        assert!(!path.exists());
    }

    #[test]
    fn valid_config_is_read_only() {
        let dir = TestDir::new();
        let original = b"model = 'claude-opus-4-8'\n[mcp_servers.sample]\nenabled = true\n";
        let path = write_config(dir.path(), original);
        assert_eq!(ensure_config_file(&path).unwrap(), None);
        assert_eq!(fs::read(path).unwrap(), original);
    }

    #[test]
    fn valid_bom_config_is_backed_up_and_atomically_normalized() {
        let dir = TestDir::new();
        let content = b"model = 'claude-opus-4-8'\n[mcp_servers.sample]\nenabled = true\n";
        let mut original = UTF8_BOM.to_vec();
        original.extend_from_slice(content);
        let path = write_config(dir.path(), &original);

        let backup = ensure_config_file(&path)
            .unwrap()
            .expect("BOM should be repaired");

        assert_eq!(fs::read(&backup).unwrap(), original);
        let repaired = fs::read(path).unwrap();
        assert_eq!(repaired, content);
        assert!(!repaired.starts_with(UTF8_BOM));
        validate_toml(&repaired).unwrap();
    }

    #[test]
    fn invalid_toml_with_or_without_bom_is_preserved() {
        for prefix in [&[][..], UTF8_BOM.as_slice()] {
            let dir = TestDir::new();
            let mut original = prefix.to_vec();
            original.extend_from_slice(b"[mcp_servers.sample\nenabled = true\n");
            let path = write_config(dir.path(), &original);
            let error = ensure_config_file(&path).unwrap_err();
            assert!(error.contains("原文件未更改"));
            assert_eq!(fs::read(path).unwrap(), original);
            assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
        }
    }

    #[test]
    fn invalid_utf8_is_preserved() {
        let dir = TestDir::new();
        let original = b"model = '\xff'\n";
        let path = write_config(dir.path(), original);
        let error = ensure_config_file(&path).unwrap_err();
        assert!(error.contains("UTF-8"));
        assert_eq!(fs::read(path).unwrap(), original);
    }

    #[test]
    fn conditional_restore_preserves_a_later_external_write() {
        let dir = TestDir::new();
        let path = dir.path().join("config.toml");
        let external = b"model = 'newer external value'\n";
        fs::write(&path, external).unwrap();
        let backup = dir.path().join("config.toml.bom-recovery.bak");
        let error = restore_preimage_if_unchanged(
            &path,
            b"model = 'our repaired value'\n",
            b"\xef\xbb\xbfmodel = 'old value'\n",
            &backup,
        )
        .unwrap_err();
        assert!(error.contains("未覆盖当前内容"));
        assert_eq!(fs::read(path).unwrap(), external);
    }

    #[cfg(unix)]
    #[test]
    fn valid_symlink_config_is_read_only() {
        use std::os::unix::fs::symlink;

        let dir = TestDir::new();
        let target = dir.path().join("target.toml");
        let original = b"model = 'safe'\n";
        fs::write(&target, original).unwrap();
        let link = dir.path().join("config.toml");
        symlink(&target, &link).unwrap();
        assert_eq!(ensure_config_file(&link).unwrap(), None);
        assert_eq!(fs::read(target).unwrap(), original);
    }

    #[cfg(unix)]
    #[test]
    fn bom_symlink_is_not_replaced_or_modified() {
        use std::os::unix::fs::symlink;

        let dir = TestDir::new();
        let target = dir.path().join("target.toml");
        let mut original = UTF8_BOM.to_vec();
        original.extend_from_slice(b"model = 'safe'\n");
        fs::write(&target, &original).unwrap();
        let link = dir.path().join("config.toml");
        symlink(&target, &link).unwrap();
        assert!(ensure_config_file(&link).unwrap_err().contains("符号链接"));
        assert!(fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(fs::read(target).unwrap(), original);
    }
}
