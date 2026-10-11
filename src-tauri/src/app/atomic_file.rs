#[cfg(unix)]
use std::fs::File;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use serde::de::DeserializeOwned;

static TMP_COUNTER: AtomicU64 = AtomicU64::new(1);
static WRITE_MUTEX: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadOutcome {
    Ok,
    RecoveredFromBak,
    Corrupt,
}

pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let _guard = WRITE_MUTEX
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "atomic path has no parent"))?;
    fs::create_dir_all(parent)?;

    let tmp = tmp_path(path);
    write_atomic_inner(path, &tmp, bytes)
}

/// Replace a file atomically without rotating or deleting its adjacent `.bak`.
/// Use this for user-owned configuration files whose backup belongs to the user.
pub fn write_atomic_preserving_backup(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let _guard = WRITE_MUTEX
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let parent = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "atomic path has no parent"))?;
    fs::create_dir_all(parent)?;

    let tmp = tmp_path(path);
    write_atomic_preserving_backup_inner(path, &tmp, bytes)
}

fn write_atomic_preserving_backup_inner(path: &Path, tmp: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut owned_temp = OwnedTempFile::create(tmp)?;
    owned_temp.write_content(bytes)?;
    fail_if_injected(FailurePoint::Replace)?;
    replace_file(tmp, path)?;
    owned_temp.disarm();
    if let Some(parent) = path.parent() {
        sync_parent_dir(parent)?;
    }
    Ok(())
}

#[cfg(windows)]
fn replace_file(source: &Path, destination: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
    };

    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let destination: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    if unsafe {
        MoveFileExW(
            source.as_ptr(),
            destination.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(not(windows))]
fn replace_file(source: &Path, destination: &Path) -> io::Result<()> {
    fs::rename(source, destination)
}

fn write_atomic_inner(path: &Path, tmp: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut owned_temp = OwnedTempFile::create(tmp)?;
    owned_temp.write_content(bytes)?;

    let bak = backup_path(path);
    if path.exists() {
        let _ = fs::remove_file(&bak);
        match fs::rename(path, &bak) {
            Ok(()) => {}
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => return Err(err),
        }
    }

    fail_if_injected(FailurePoint::Replace)?;
    if let Err(err) = replace_file(tmp, path) {
        if !path.exists() && bak.exists() {
            let _ = fs::rename(&bak, path);
        }
        return Err(err);
    }
    owned_temp.disarm();

    if let Some(parent) = path.parent() {
        sync_parent_dir(parent)?;
    }
    Ok(())
}

/// Owns only a file this process created with CREATE_NEW. On failure, cleanup
/// is tied to the still-open file identity; a pre-existing or swapped-in path
/// can never be truncated or removed as part of rollback.
struct OwnedTempFile {
    #[cfg(not(windows))]
    path: PathBuf,
    file: Option<fs::File>,
    identity: Option<TempIdentity>,
    armed: bool,
}

impl OwnedTempFile {
    fn create(path: &Path) -> io::Result<Self> {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            // Keep a delete-capable handle so failed writes can remove this
            // exact CREATE_NEW object even if its directory entry is renamed.
            options
                .access_mode(0x4000_0000 | 0x0001_0000 | 0x0000_0080)
                .share_mode(0x0000_0001 | 0x0000_0002 | 0x0000_0004);
        }
        let file = options.open(path)?;
        let mut owned = Self {
            #[cfg(not(windows))]
            path: path.to_path_buf(),
            file: Some(file),
            identity: None,
            armed: true,
        };
        let file = owned.file.as_ref().expect("just-created temp handle");
        owned.identity = Some(TempIdentity::from_file(file)?);
        fail_if_injected(FailurePoint::AfterCreate)?;
        Ok(owned)
    }

    fn write_content(&mut self, bytes: &[u8]) -> io::Result<()> {
        let file = self
            .file
            .as_mut()
            .ok_or_else(|| io::Error::other("atomic temporary handle is closed"))?;
        file.write_all(bytes)?;
        fail_if_injected(FailurePoint::AfterWrite)?;
        file.flush()?;
        fail_if_injected(FailurePoint::AfterFlush)?;
        file.sync_all()?;
        fail_if_injected(FailurePoint::AfterSync)
    }

    fn disarm(&mut self) {
        self.armed = false;
        self.file.take();
    }
}

impl Drop for OwnedTempFile {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let Some(file) = self.file.as_ref() else {
            return;
        };
        if !self.identity.as_ref().map_or_else(
            || TempIdentity::is_single_link_regular(file),
            |identity| identity.still_owned_single_link(file),
        ) {
            return;
        }
        #[cfg(windows)]
        {
            use std::mem::size_of;
            use std::os::windows::io::AsRawHandle;
            use windows_sys::Win32::Storage::FileSystem::{
                FileDispositionInfo, SetFileInformationByHandle, FILE_DISPOSITION_INFO,
            };
            let disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
            unsafe {
                SetFileInformationByHandle(
                    file.as_raw_handle() as _,
                    FileDispositionInfo,
                    (&disposition as *const FILE_DISPOSITION_INFO).cast(),
                    size_of::<FILE_DISPOSITION_INFO>() as u32,
                );
            }
        }
        #[cfg(not(windows))]
        {
            // On Unix this is guarded by the opened inode identity and a
            // single-link check. Windows uses handle-relative disposition.
            let _ = fs::remove_file(&self.path);
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TempIdentity {
    #[cfg(windows)]
    volume_serial: Option<u32>,
    #[cfg(windows)]
    file_index: Option<u64>,
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
    #[cfg(not(any(windows, unix)))]
    length: u64,
}

impl TempIdentity {
    fn from_file(file: &fs::File) -> io::Result<Self> {
        #[cfg(any(unix, not(windows)))]
        let metadata = file.metadata()?;
        #[cfg(windows)]
        {
            let info = file_info(file)?;
            Ok(Self {
                volume_serial: Some(info.dwVolumeSerialNumber),
                file_index: Some(
                    (u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow),
                ),
            })
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            Ok(Self {
                device: metadata.dev(),
                inode: metadata.ino(),
            })
        }
        #[cfg(not(any(windows, unix)))]
        {
            Ok(Self {
                length: metadata.len(),
            })
        }
    }

    fn still_owned_single_link(&self, file: &fs::File) -> bool {
        #[cfg(any(unix, not(windows)))]
        let Ok(metadata) = file.metadata() else {
            return false;
        };
        #[cfg(windows)]
        {
            file_info(file).is_ok_and(|info| {
                info.nNumberOfLinks == 1
                    && info.dwFileAttributes & 0x0000_0400 == 0
                    && Some(info.dwVolumeSerialNumber) == self.volume_serial
                    && Some((u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow))
                        == self.file_index
            })
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            metadata.nlink() == 1 && metadata.dev() == self.device && metadata.ino() == self.inode
        }
        #[cfg(not(any(windows, unix)))]
        {
            metadata.is_file() && metadata.len() == self.length
        }
    }

    fn is_single_link_regular(file: &fs::File) -> bool {
        #[cfg(any(unix, not(windows)))]
        let Ok(metadata) = file.metadata() else {
            return false;
        };
        #[cfg(windows)]
        {
            file_info(file).is_ok_and(|info| {
                info.nNumberOfLinks == 1
                    && info.dwFileAttributes & 0x0000_0400 == 0
                    && info.dwFileAttributes & 0x0000_0010 == 0
            })
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            metadata.is_file() && metadata.nlink() == 1
        }
        #[cfg(not(any(windows, unix)))]
        {
            metadata.is_file()
        }
    }
}

#[cfg(windows)]
fn file_info(
    file: &fs::File,
) -> io::Result<windows_sys::Win32::Storage::FileSystem::BY_HANDLE_FILE_INFORMATION> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
    };
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    if unsafe { GetFileInformationByHandle(file.as_raw_handle() as _, &mut info) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(info)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FailurePoint {
    AfterCreate,
    AfterWrite,
    AfterFlush,
    AfterSync,
    Replace,
}

#[cfg(test)]
thread_local! {
    static TEST_FAILURE_POINT: std::cell::Cell<Option<FailurePoint>> = const { std::cell::Cell::new(None) };
}

#[cfg(test)]
fn fail_if_injected(point: FailurePoint) -> io::Result<()> {
    TEST_FAILURE_POINT.with(|value| {
        if value.get() == Some(point) {
            Err(io::Error::other(format!(
                "injected atomic failure at {point:?}"
            )))
        } else {
            Ok(())
        }
    })
}

#[cfg(not(test))]
fn fail_if_injected(_point: FailurePoint) -> io::Result<()> {
    Ok(())
}

pub fn read_with_recovery<T: DeserializeOwned>(path: &Path) -> (Option<T>, LoadOutcome) {
    if let Some(value) = read_json(path) {
        return (Some(value), LoadOutcome::Ok);
    }
    if let Some(value) = read_json(&backup_path(path)) {
        return (Some(value), LoadOutcome::RecoveredFromBak);
    }
    (None, LoadOutcome::Corrupt)
}

fn read_json<T: DeserializeOwned>(path: &Path) -> Option<T> {
    let bytes = fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn tmp_path(path: &Path) -> PathBuf {
    let counter = TMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    path_with_added_extension(path, &format!("tmp-{}-{counter}", std::process::id()))
}

pub fn backup_path(path: &Path) -> PathBuf {
    path_with_added_extension(path, "bak")
}

fn path_with_added_extension(path: &Path, added: &str) -> PathBuf {
    match path.extension().and_then(|ext| ext.to_str()) {
        Some(ext) if !ext.is_empty() => path.with_extension(format!("{ext}.{added}")),
        _ => path.with_extension(added),
    }
}

#[cfg(unix)]
fn sync_parent_dir(parent: &Path) -> io::Result<()> {
    File::open(parent)?.sync_all()
}

#[cfg(not(unix))]
fn sync_parent_dir(_parent: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        read_with_recovery, write_atomic, write_atomic_preserving_backup,
        write_atomic_preserving_backup_inner, FailurePoint, LoadOutcome,
    };
    use serde::{Deserialize, Serialize};
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_COUNTER: AtomicU64 = AtomicU64::new(1);

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    struct Payload {
        value: u64,
    }

    fn test_dir(name: &str) -> std::path::PathBuf {
        let id = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("test-data")
            .join(format!("{name}-{}-{id}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn write_atomic_replaces_main_and_preserves_backup() {
        let dir = test_dir("atomic-replace");
        let path = dir.join("settings.json");
        write_atomic(&path, br#"{"value":1}"#).unwrap();
        write_atomic(&path, br#"{"value":2}"#).unwrap();

        let (value, outcome) = read_with_recovery::<Payload>(&path);
        assert_eq!(outcome, LoadOutcome::Ok);
        assert_eq!(value.unwrap(), Payload { value: 2 });
        assert_eq!(
            serde_json::from_slice::<Payload>(&fs::read(path.with_extension("json.bak")).unwrap())
                .unwrap(),
            Payload { value: 1 }
        );

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn config_replacement_preserves_user_backup_bytes() {
        let dir = test_dir("atomic-preserve-backup");
        let path = dir.join("config.toml");
        let backup = dir.join("config.toml.bak");
        fs::write(&path, b"model = \"before\"\n").unwrap();
        fs::write(&backup, b"user-owned recovery point\n").unwrap();

        write_atomic_preserving_backup(&path, b"model = \"after\"\n").unwrap();

        assert_eq!(fs::read(&path).unwrap(), b"model = \"after\"\n");
        assert_eq!(fs::read(&backup).unwrap(), b"user-owned recovery point\n");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn preexisting_atomic_temp_hard_link_is_not_truncated_or_removed() {
        let dir = test_dir("atomic-temp-hard-link");
        let path = dir.join("revocation.json");
        let temp = dir.join("revocation.json.tmp-123-7");
        let outside = dir.join("keep.txt");
        fs::write(&path, b"original destination").unwrap();
        fs::write(&outside, b"user-owned bytes").unwrap();
        fs::hard_link(&outside, &temp).unwrap();

        let error =
            write_atomic_preserving_backup_inner(&path, &temp, b"replacement bytes").unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read(&path).unwrap(), b"original destination");
        assert_eq!(fs::read(&outside).unwrap(), b"user-owned bytes");
        assert_eq!(fs::read(&temp).unwrap(), b"user-owned bytes");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn every_post_create_failure_removes_only_the_owned_temp() {
        for point in [
            FailurePoint::AfterCreate,
            FailurePoint::AfterWrite,
            FailurePoint::AfterFlush,
            FailurePoint::AfterSync,
            FailurePoint::Replace,
        ] {
            let dir = test_dir(&format!("atomic-failure-{point:?}"));
            let path = dir.join("revocation.json");
            let temp = dir.join("revocation.json.tmp-123-7");
            fs::write(&path, b"original destination").unwrap();
            super::TEST_FAILURE_POINT.with(|value| value.set(Some(point)));
            let error = write_atomic_preserving_backup_inner(&path, &temp, b"replacement bytes")
                .unwrap_err();
            super::TEST_FAILURE_POINT.with(|value| value.set(None));

            assert!(error.to_string().contains("injected atomic failure"));
            assert!(!temp.exists(), "temp survived failure at {point:?}");
            assert_eq!(fs::read(&path).unwrap(), b"original destination");
            let _ = fs::remove_dir_all(dir);
        }
    }

    #[test]
    fn read_with_recovery_uses_backup_for_corrupt_main() {
        let dir = test_dir("atomic-recover");
        let path = dir.join("provenance.json");
        fs::write(&path, b"{").unwrap();
        fs::write(path.with_extension("json.bak"), br#"{"value":7}"#).unwrap();

        let (value, outcome) = read_with_recovery::<Payload>(&path);
        assert_eq!(outcome, LoadOutcome::RecoveredFromBak);
        assert_eq!(value.unwrap(), Payload { value: 7 });

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn read_with_recovery_reports_corrupt_when_main_and_backup_fail() {
        let dir = test_dir("atomic-corrupt");
        let path = dir.join("settings.json");
        fs::write(&path, b"").unwrap();
        fs::write(path.with_extension("json.bak"), b"{").unwrap();

        let (value, outcome) = read_with_recovery::<Payload>(&path);
        assert_eq!(outcome, LoadOutcome::Corrupt);
        assert!(value.is_none());

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn concurrent_writes_leave_complete_json() {
        let dir = test_dir("atomic-concurrent");
        let path = dir.join("settings.json");
        write_atomic(&path, br#"{"value":0}"#).unwrap();

        let handles: Vec<_> = (1_u64..=8)
            .map(|value| {
                let path = path.clone();
                std::thread::spawn(move || {
                    for _ in 0..10 {
                        let bytes = serde_json::to_vec(&Payload { value }).unwrap();
                        write_atomic(&path, &bytes).unwrap();
                    }
                })
            })
            .collect();
        for handle in handles {
            handle.join().unwrap();
        }

        let (value, outcome) = read_with_recovery::<Payload>(&path);
        assert_eq!(outcome, LoadOutcome::Ok);
        assert!(matches!(value.unwrap().value, 1..=8));

        let _ = fs::remove_dir_all(dir);
    }
}
