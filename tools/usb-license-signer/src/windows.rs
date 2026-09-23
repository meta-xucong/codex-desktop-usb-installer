use std::path::Path;

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _;
use serde::Serialize;

use crate::error::SignerError;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct VolumeInfo {
    pub drive_root: String,
    pub volume_serial: u32,
    pub file_system: String,
}

#[derive(Debug, Clone, Copy)]
pub enum AuthenticodeSubject {
    Any,
    Contains(&'static str),
}

/// Keep the native GUI single-instance even when a user double-clicks the
/// signer repeatedly or starts it from a second copy of the release folder.
/// A stale second window can otherwise keep showing an older key-binding
/// result while a fresh instance has already passed the current check.
#[cfg(target_os = "windows")]
pub struct GuiInstanceMutex {
    handle: windows_sys::Win32::Foundation::HANDLE,
}

#[cfg(target_os = "windows")]
pub fn acquire_gui_instance_mutex() -> Result<Option<GuiInstanceMutex>, SignerError> {
    use windows_sys::Win32::Foundation::{CloseHandle, GetLastError};
    use windows_sys::Win32::System::Threading::CreateMutexW;

    const ERROR_ALREADY_EXISTS: u32 = 183;
    let name = wide("Local\\CodexUsbLicenseSigner.Gui");
    let handle = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
    if handle.is_null() {
        return Err(SignerError::PathRejected(format!(
            "cannot create GUI instance mutex: {}",
            std::io::Error::last_os_error()
        )));
    }
    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        unsafe {
            CloseHandle(handle);
        }
        return Ok(None);
    }
    Ok(Some(GuiInstanceMutex { handle }))
}

#[cfg(target_os = "windows")]
impl Drop for GuiInstanceMutex {
    fn drop(&mut self) {
        use windows_sys::Win32::Foundation::CloseHandle;

        unsafe {
            CloseHandle(self.handle);
        }
    }
}

#[cfg(not(target_os = "windows"))]
pub struct GuiInstanceMutex;

#[cfg(not(target_os = "windows"))]
pub fn acquire_gui_instance_mutex() -> Result<Option<GuiInstanceMutex>, SignerError> {
    Ok(Some(GuiInstanceMutex))
}

/// Serialize signing transactions across GUI/CLI processes on this Windows
/// user session. The lock covers the complete observe/authorize/write/readback
/// transaction, not just the final file replacement.
#[cfg(target_os = "windows")]
pub struct SigningMutex {
    handle: windows_sys::Win32::Foundation::HANDLE,
}

#[cfg(target_os = "windows")]
pub fn acquire_signing_mutex() -> Result<SigningMutex, SignerError> {
    use windows_sys::Win32::Foundation::{
        CloseHandle, WAIT_ABANDONED, WAIT_OBJECT_0, WAIT_TIMEOUT,
    };
    use windows_sys::Win32::System::Threading::{CreateMutexW, WaitForSingleObject};

    let name = wide("Local\\CodexUsbLicenseSigner.Signing");
    let handle = unsafe { CreateMutexW(std::ptr::null(), 1, name.as_ptr()) };
    if handle.is_null() {
        return Err(SignerError::PathRejected(format!(
            "cannot create signing mutex: {}",
            std::io::Error::last_os_error()
        )));
    }
    let wait = unsafe { WaitForSingleObject(handle, 0) };
    if wait == WAIT_OBJECT_0 || wait == WAIT_ABANDONED {
        return Ok(SigningMutex { handle });
    }
    unsafe {
        CloseHandle(handle);
    }
    if wait == WAIT_TIMEOUT {
        return Err(SignerError::SigningBusy);
    }
    Err(SignerError::PathRejected(format!(
        "cannot acquire signing mutex: wait result {wait}"
    )))
}

#[cfg(target_os = "windows")]
impl Drop for SigningMutex {
    fn drop(&mut self) {
        use windows_sys::Win32::Foundation::CloseHandle;
        use windows_sys::Win32::System::Threading::ReleaseMutex;

        unsafe {
            ReleaseMutex(self.handle);
            CloseHandle(self.handle);
        }
    }
}

#[cfg(not(target_os = "windows"))]
pub struct SigningMutex;

#[cfg(not(target_os = "windows"))]
pub fn acquire_signing_mutex() -> Result<SigningMutex, SignerError> {
    Err(SignerError::UnsupportedPlatform)
}

#[cfg(target_os = "windows")]
fn wide(value: &str) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    std::ffi::OsStr::new(value)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

#[cfg(all(test, target_os = "windows"))]
mod tests {
    use super::*;

    #[test]
    fn gui_instance_mutex_allows_only_one_owner() {
        let first = acquire_gui_instance_mutex()
            .expect("GUI instance mutex should be creatable")
            .expect("the test should own the first GUI instance mutex");
        let second = acquire_gui_instance_mutex().expect("second mutex probe should succeed");
        assert!(second.is_none());
        drop(first);
    }
}

#[cfg(target_os = "windows")]
pub fn volume_info(drive_root: &str) -> Result<VolumeInfo, SignerError> {
    use windows_sys::Win32::Storage::FileSystem::GetVolumeInformationW;

    let root = crate::path_security::normalize_drive_root(drive_root)?;
    let root_wide = wide(&root);
    let mut serial = 0u32;
    let mut max_component = 0u32;
    let mut flags = 0u32;
    let mut volume_name = [0u16; 256];
    let mut filesystem_name = [0u16; 256];
    let ok = unsafe {
        GetVolumeInformationW(
            root_wide.as_ptr(),
            volume_name.as_mut_ptr(),
            volume_name.len() as u32,
            &mut serial,
            &mut max_component,
            &mut flags,
            filesystem_name.as_mut_ptr(),
            filesystem_name.len() as u32,
        )
    };
    if ok == 0 {
        return Err(SignerError::PathRejected(format!(
            "cannot read volume information for {root}: {}",
            std::io::Error::last_os_error()
        )));
    }
    let nul = filesystem_name
        .iter()
        .position(|value| *value == 0)
        .unwrap_or(filesystem_name.len());
    Ok(VolumeInfo {
        drive_root: root,
        volume_serial: serial,
        file_system: String::from_utf16_lossy(&filesystem_name[..nul]),
    })
}

#[cfg(not(target_os = "windows"))]
pub fn volume_info(_drive_root: &str) -> Result<VolumeInfo, SignerError> {
    Err(SignerError::UnsupportedPlatform)
}

#[cfg(target_os = "windows")]
pub fn removable_ntfs_volumes() -> Result<Vec<VolumeInfo>, SignerError> {
    use windows_sys::Win32::Storage::FileSystem::{GetDriveTypeW, GetLogicalDrives};
    use windows_sys::Win32::System::WindowsProgramming::DRIVE_REMOVABLE;

    let mask = unsafe { GetLogicalDrives() };
    if mask == 0 {
        return Err(SignerError::PathRejected(format!(
            "cannot enumerate logical drives: {}",
            std::io::Error::last_os_error()
        )));
    }
    let mut volumes = Vec::new();
    for index in 0..26u8 {
        if mask & (1u32 << index) == 0 {
            continue;
        }
        let root = format!("{}:\\", (b'A' + index) as char);
        let root_wide = wide(&root);
        if unsafe { GetDriveTypeW(root_wide.as_ptr()) } != DRIVE_REMOVABLE {
            continue;
        }
        let info = volume_info(&root)?;
        if info.file_system.eq_ignore_ascii_case("NTFS") {
            volumes.push(info);
        }
    }
    Ok(volumes)
}

#[cfg(not(target_os = "windows"))]
pub fn removable_ntfs_volumes() -> Result<Vec<VolumeInfo>, SignerError> {
    Err(SignerError::UnsupportedPlatform)
}

#[cfg(target_os = "windows")]
pub fn drive_is_removable_ntfs(drive_root: &str) -> Result<VolumeInfo, SignerError> {
    use windows_sys::Win32::Storage::FileSystem::GetDriveTypeW;
    use windows_sys::Win32::System::WindowsProgramming::DRIVE_REMOVABLE;

    let root = crate::path_security::normalize_drive_root(drive_root)?;
    let root_wide = wide(&root);
    if unsafe { GetDriveTypeW(root_wide.as_ptr()) } != DRIVE_REMOVABLE {
        return Err(SignerError::PathRejected(format!(
            "target drive is not DRIVE_REMOVABLE: {root}"
        )));
    }
    let info = volume_info(&root)?;
    if !info.file_system.eq_ignore_ascii_case("NTFS") {
        return Err(SignerError::PathRejected(format!(
            "target drive filesystem is not NTFS: {}",
            info.file_system
        )));
    }
    Ok(info)
}

#[cfg(not(target_os = "windows"))]
pub fn drive_is_removable_ntfs(_drive_root: &str) -> Result<VolumeInfo, SignerError> {
    Err(SignerError::UnsupportedPlatform)
}

#[cfg(target_os = "windows")]
pub fn ensure_local_path(path: &Path, forbidden_drive: &str) -> Result<(), SignerError> {
    use windows_sys::Win32::Storage::FileSystem::GetDriveTypeW;
    use windows_sys::Win32::System::WindowsProgramming::{DRIVE_REMOTE, DRIVE_REMOVABLE};

    let drive_root = crate::path_security::drive_root_from_absolute_path(path)?;
    let forbidden = crate::path_security::normalize_drive_root(forbidden_drive)?;
    if drive_root.eq_ignore_ascii_case(&forbidden) {
        return Err(SignerError::PathRejected(format!(
            "local output path is on the target removable drive: {}",
            path.display()
        )));
    }
    let drive_wide = wide(&drive_root);
    let drive_type = unsafe { GetDriveTypeW(drive_wide.as_ptr()) };
    if drive_type == DRIVE_REMOVABLE || drive_type == DRIVE_REMOTE {
        return Err(SignerError::PathRejected(format!(
            "local output path is on a removable or remote drive: {}",
            path.display()
        )));
    }
    crate::path_security::ensure_no_reparse_components(path)
}

/// Validate a signer-owned path without requiring a target USB drive.  The
/// automatic key store is always kept on a local fixed disk and never on a
/// removable, remote, or reparse-backed path.
#[cfg(target_os = "windows")]
pub fn ensure_fixed_local_path(path: &Path) -> Result<(), SignerError> {
    use windows_sys::Win32::Storage::FileSystem::GetDriveTypeW;
    use windows_sys::Win32::System::WindowsProgramming::DRIVE_FIXED;

    let drive_root = crate::path_security::drive_root_from_absolute_path(path)?;
    let drive_wide = wide(&drive_root);
    let drive_type = unsafe { GetDriveTypeW(drive_wide.as_ptr()) };
    if drive_type != DRIVE_FIXED {
        return Err(SignerError::PathRejected(format!(
            "signer key path must be on a fixed local disk: {}",
            path.display()
        )));
    }
    crate::path_security::ensure_no_reparse_components(path)
}

#[cfg(not(target_os = "windows"))]
pub fn ensure_fixed_local_path(_path: &Path) -> Result<(), SignerError> {
    Err(SignerError::UnsupportedPlatform)
}

#[cfg(not(target_os = "windows"))]
pub fn ensure_local_path(_path: &Path, _forbidden_drive: &str) -> Result<(), SignerError> {
    Err(SignerError::UnsupportedPlatform)
}

#[cfg(target_os = "windows")]
pub fn verify_authenticode(
    path: &Path,
    label: &str,
    expected_subject: AuthenticodeSubject,
) -> Result<(), SignerError> {
    let path_text = path.to_string_lossy();
    let path_b64 = BASE64.encode(path_text.as_bytes());
    let subject_check = match expected_subject {
        AuthenticodeSubject::Any => String::new(),
        AuthenticodeSubject::Contains(fragment) => format!(
            "if ($subject -notlike '*{}*') {{ Write-Output ('SubjectMismatch:' + $subject); exit 12 }}",
            fragment.replace('"', "")
        ),
    };
    let script = format!(
        "$pathText=[Text.Encoding]::UTF8.GetString([Convert]::FromBase64String('{path_b64}'));$s=Get-AuthenticodeSignature -LiteralPath $pathText;if($s.Status -ne 'Valid'){{Write-Output ('Status:' + [string]$s.Status);exit 10}};$subject=[string]$s.SignerCertificate.Subject;if([string]::IsNullOrWhiteSpace($subject)){{Write-Output 'SubjectMissing';exit 11}};{subject_check}Write-Output 'Valid'"
    );
    let mut script_bytes = Vec::with_capacity(script.len() * 2);
    for unit in script.encode_utf16() {
        script_bytes.extend_from_slice(&unit.to_le_bytes());
    }
    let encoded_script = BASE64.encode(script_bytes);
    let mut last_error = None;
    for executable in ["powershell.exe", "pwsh"] {
        match std::process::Command::new(executable)
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-EncodedCommand",
                &encoded_script,
            ])
            .output()
        {
            Ok(output) => {
                if output.status.success() {
                    return Ok(());
                }
                let mut detail = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if detail.is_empty() {
                    detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
                }
                if detail.len() > 512 {
                    detail.truncate(512);
                }
                return Err(SignerError::Authenticode {
                    label: label.to_string(),
                    detail: if detail.is_empty() {
                        format!("PowerShell exited with {}", output.status)
                    } else {
                        detail
                    },
                });
            }
            Err(error) => last_error = Some(error),
        }
    }
    Err(SignerError::Authenticode {
        label: label.to_string(),
        detail: format!(
            "no PowerShell executable available: {}",
            last_error
                .map(|error| error.to_string())
                .unwrap_or_else(|| "process launch failed".to_string())
        ),
    })
}

#[cfg(not(target_os = "windows"))]
pub fn verify_authenticode(
    _path: &Path,
    _label: &str,
    _expected_subject: AuthenticodeSubject,
) -> Result<(), SignerError> {
    Err(SignerError::UnsupportedPlatform)
}

#[cfg(target_os = "windows")]
pub fn atomic_replace(temp: &Path, destination: &Path) -> Result<(), SignerError> {
    use windows_sys::Win32::Storage::FileSystem::{
        MoveFileExW, ReplaceFileW, MOVEFILE_WRITE_THROUGH, REPLACEFILE_WRITE_THROUGH,
    };

    let temp_wide = wide(&temp.to_string_lossy());
    let destination_wide = wide(&destination.to_string_lossy());
    let ok = if destination.exists() {
        unsafe {
            ReplaceFileW(
                destination_wide.as_ptr(),
                temp_wide.as_ptr(),
                std::ptr::null(),
                REPLACEFILE_WRITE_THROUGH,
                std::ptr::null(),
                std::ptr::null(),
            )
        }
    } else {
        unsafe {
            MoveFileExW(
                temp_wide.as_ptr(),
                destination_wide.as_ptr(),
                MOVEFILE_WRITE_THROUGH,
            )
        }
    };
    if ok == 0 {
        return Err(SignerError::io(
            "atomically replacing license",
            destination,
            std::io::Error::last_os_error(),
        ));
    }
    Ok(())
}

#[cfg(not(target_os = "windows"))]
pub fn atomic_replace(temp: &Path, destination: &Path) -> Result<(), SignerError> {
    std::fs::rename(temp, destination)
        .map_err(|error| SignerError::io("atomically replacing license", destination, error))
}

#[cfg(target_os = "windows")]
pub fn dpapi_unprotect(blob: &[u8]) -> Result<Vec<u8>, SignerError> {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{CryptUnprotectData, CRYPT_INTEGER_BLOB};

    if blob.is_empty() || blob.len() > u32::MAX as usize {
        return Err(SignerError::KeyBlob(
            "DPAPI blob is empty or too large".to_string(),
        ));
    }
    let input = CRYPT_INTEGER_BLOB {
        cbData: blob.len() as u32,
        pbData: blob.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB::default();
    let ok = unsafe {
        CryptUnprotectData(
            &input,
            std::ptr::null_mut(),
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            0,
            &mut output,
        )
    };
    if ok == 0 || output.pbData.is_null() {
        return Err(SignerError::KeyBlob(format!(
            "Windows DPAPI could not decrypt the local key blob: {}",
            std::io::Error::last_os_error()
        )));
    }
    let result =
        unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec() };
    unsafe {
        LocalFree(output.pbData.cast());
    }
    Ok(result)
}

#[cfg(target_os = "windows")]
pub fn dpapi_protect(plaintext: &[u8]) -> Result<Vec<u8>, SignerError> {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{CryptProtectData, CRYPT_INTEGER_BLOB};

    if plaintext.is_empty() || plaintext.len() > u32::MAX as usize {
        return Err(SignerError::KeyBlob(
            "DPAPI plaintext is empty or too large".to_string(),
        ));
    }
    let input = CRYPT_INTEGER_BLOB {
        cbData: plaintext.len() as u32,
        pbData: plaintext.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB::default();
    let ok = unsafe {
        CryptProtectData(
            &input,
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            std::ptr::null(),
            0,
            &mut output,
        )
    };
    if ok == 0 || output.pbData.is_null() {
        return Err(SignerError::KeyBlob(format!(
            "Windows DPAPI could not protect the local key blob: {}",
            std::io::Error::last_os_error()
        )));
    }
    let result =
        unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec() };
    unsafe {
        LocalFree(output.pbData.cast());
    }
    Ok(result)
}

#[cfg(not(target_os = "windows"))]
pub fn dpapi_unprotect(_blob: &[u8]) -> Result<Vec<u8>, SignerError> {
    Err(SignerError::UnsupportedPlatform)
}

#[cfg(not(target_os = "windows"))]
pub fn dpapi_protect(_plaintext: &[u8]) -> Result<Vec<u8>, SignerError> {
    Err(SignerError::UnsupportedPlatform)
}
