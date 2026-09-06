//! Handle-relative traversal for the exact, already-authorized cleanup targets.
//! Sharing flags cannot prevent FILE_WRITE_ATTRIBUTES from converting an empty
//! directory to a junction. Never re-resolve an absolute child path after opening
//! its parent: NtCreateFile rejects reparses, enumeration and deletion use handles.

use std::ffi::{c_void, OsStr, OsString};
use std::io;
use std::mem::{offset_of, size_of};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Component, Path, Prefix};
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::*;

#[repr(C)]
struct UnicodeString {
    length: u16,
    maximum_length: u16,
    buffer: *mut u16,
}
#[repr(C)]
struct ObjectAttributes {
    length: u32,
    root_directory: HANDLE,
    object_name: *mut UnicodeString,
    attributes: u32,
    security_descriptor: *mut c_void,
    security_quality_of_service: *mut c_void,
}
#[repr(C)]
struct IoStatusBlock {
    status_or_pointer: usize,
    information: usize,
}
#[link(name = "ntdll")]
unsafe extern "system" {
    fn NtCreateFile(
        file: *mut HANDLE,
        access: u32,
        attributes: *mut ObjectAttributes,
        status: *mut IoStatusBlock,
        allocation: *const i64,
        file_attributes: u32,
        share: u32,
        disposition: u32,
        options: u32,
        ea: *const c_void,
        ea_length: u32,
    ) -> i32;
    fn RtlNtStatusToDosError(status: i32) -> u32;
}

pub(super) struct Entry {
    handle: HANDLE,
    // Ancestors deny rename/delete. Reparse mutation is addressed separately by
    // relative opens + OBJ_DONT_REPARSE, not by this sharing restriction.
    _ancestors: Vec<Entry>,
}
impl Drop for Entry {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.handle) };
    }
}

fn last_error(action: &str) -> String {
    format!("{action}：{}", io::Error::last_os_error())
}

impl Entry {
    fn info(&self) -> Result<BY_HANDLE_FILE_INFORMATION, String> {
        let mut info = BY_HANDLE_FILE_INFORMATION::default();
        if unsafe { GetFileInformationByHandle(self.handle, &mut info) } == 0 {
            return Err(last_error("无法核验清理句柄"));
        }
        if info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err("拒绝清理重解析对象；原数据已保留".into());
        }
        Ok(info)
    }

    fn child(&self, name: &OsStr, delete: bool) -> Result<Self, String> {
        self.child_after_check(name, delete, || {})
    }

    fn child_after_check(
        &self,
        name: &OsStr,
        delete: bool,
        after_check: impl FnOnce(),
    ) -> Result<Self, String> {
        self.info()?;
        after_check();
        let mut units: Vec<u16> = name.encode_wide().collect();
        if units.is_empty()
            || units.len() > 32767
            || units.iter().any(|unit| matches!(*unit, 0 | 47 | 58 | 92))
            || units == [46]
            || units == [46, 46]
        {
            return Err("拒绝非单一子项名称".into());
        }
        let mut name = UnicodeString {
            length: (units.len() * 2) as u16,
            maximum_length: (units.len() * 2) as u16,
            buffer: units.as_mut_ptr(),
        };
        let mut attributes = ObjectAttributes {
            length: size_of::<ObjectAttributes>() as u32,
            root_directory: self.handle,
            object_name: &mut name,
            attributes: 0x40 | 0x1000, // OBJ_CASE_INSENSITIVE | OBJ_DONT_REPARSE
            security_descriptor: std::ptr::null_mut(),
            security_quality_of_service: std::ptr::null_mut(),
        };
        let mut status = IoStatusBlock {
            status_or_pointer: 0,
            information: 0,
        };
        let mut handle = std::ptr::null_mut();
        let result = unsafe {
            NtCreateFile(
                &mut handle,
                FILE_READ_ATTRIBUTES
                    | FILE_LIST_DIRECTORY
                    | 0x00100000
                    | if delete {
                        DELETE | FILE_WRITE_ATTRIBUTES
                    } else {
                        0
                    },
                &mut attributes,
                &mut status,
                std::ptr::null(),
                0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                1,                 // FILE_OPEN: never create during cleanup
                0x00200000 | 0x20, // OPEN_REPARSE_POINT | SYNCHRONOUS_IO_NONALERT
                std::ptr::null(),
                0,
            )
        };
        if result < 0 {
            return Err(format!(
                "无法安全打开清理子项（NTSTATUS {:#010x}）：{}",
                result as u32,
                io::Error::from_raw_os_error(unsafe { RtlNtStatusToDosError(result) } as i32)
            ));
        }
        let entry = Self {
            handle,
            _ancestors: Vec::new(),
        };
        entry.info()?;
        Ok(entry)
    }

    pub(super) fn open(path: &Path, delete: bool) -> Result<Self, String> {
        let mut components = path.components();
        let drive = match components.next() {
            Some(Component::Prefix(prefix)) => match prefix.kind() {
                Prefix::Disk(drive) | Prefix::VerbatimDisk(drive) => drive,
                _ => return Err("清理仅接受本地 DOS 卷路径".into()),
            },
            _ => return Err("清理路径必须为绝对路径".into()),
        };
        if components.next() != Some(Component::RootDir) {
            return Err("拒绝驱动器相对路径".into());
        }
        let names = components
            .map(|part| match part {
                Component::Normal(name) => Ok(name),
                _ => Err("拒绝清理跳转路径".to_string()),
            })
            .collect::<Result<Vec<_>, _>>()?;
        if names.is_empty() {
            return Err("拒绝清理卷根目录".into());
        }
        let root = [drive as u16, 58, 92, 0];
        let handle = unsafe {
            CreateFileW(
                root.as_ptr(),
                FILE_READ_ATTRIBUTES | FILE_LIST_DIRECTORY,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                std::ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
                std::ptr::null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(last_error("无法打开清理卷根句柄"));
        }
        let mut current = Self {
            handle,
            _ancestors: Vec::new(),
        };
        current.info()?;
        let mut ancestors = Vec::new();
        for (index, name) in names.iter().enumerate() {
            let next = current.child(name, delete && index + 1 == names.len())?;
            ancestors.push(current);
            current = next;
        }
        current._ancestors = ancestors;
        Ok(current)
    }

    fn names(&self) -> Result<Vec<OsString>, String> {
        if self.info()?.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY == 0 {
            return Err("无法枚举非目录清理句柄".into());
        }
        let mut names = Vec::new();
        let mut buffer = vec![0_u64; 8192]; // native structure alignment, 64 KiB
        let mut class = FileIdBothDirectoryRestartInfo;
        loop {
            if unsafe {
                GetFileInformationByHandleEx(
                    self.handle,
                    class,
                    buffer.as_mut_ptr().cast(),
                    (buffer.len() * 8) as u32,
                )
            } == 0
            {
                let error = io::Error::last_os_error();
                if error.raw_os_error() == Some(18) {
                    return Ok(names);
                } // NO_MORE_FILES
                return Err(format!("无法枚举清理句柄：{error}"));
            }
            class = FileIdBothDirectoryInfo;
            let bytes = unsafe {
                std::slice::from_raw_parts(buffer.as_ptr().cast::<u8>(), buffer.len() * 8)
            };
            let mut offset = 0;
            loop {
                let header = offset_of!(FILE_ID_BOTH_DIR_INFO, FileName);
                if offset + header > bytes.len() {
                    return Err("目录枚举结构超界".into());
                }
                // Decode the kernel's variable-length byte buffer without
                // assuming that filename length implies structure alignment.
                let next =
                    u32::from_ne_bytes(bytes[offset..offset + 4].try_into().unwrap()) as usize;
                let length_offset = offset + offset_of!(FILE_ID_BOTH_DIR_INFO, FileNameLength);
                let length =
                    u32::from_ne_bytes(bytes[length_offset..length_offset + 4].try_into().unwrap())
                        as usize;
                if length % 2 != 0 || offset + header + length > bytes.len() {
                    return Err("目录枚举名称超界".into());
                }
                let units = bytes[offset + header..offset + header + length]
                    .chunks_exact(2)
                    .map(|pair| u16::from_ne_bytes([pair[0], pair[1]]))
                    .collect::<Vec<_>>();
                if units != [46] && units != [46, 46] {
                    names.push(OsString::from_wide(&units));
                }
                if next == 0 {
                    break;
                }
                if next < header + length || offset + next >= bytes.len() {
                    return Err("目录枚举下一项超界".into());
                }
                offset += next;
            }
        }
    }

    fn walk(&self, delete: bool, depth: usize) -> Result<(), String> {
        if depth > 256 {
            return Err("清理目录层级异常，停止遍历".into());
        }
        let info = self.info()?;
        if info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
            for name in self.names()? {
                let child = self.child(&name, delete)?;
                child.walk(delete, depth + 1)?;
            }
        }
        if delete {
            let before = self.info()?;
            self.clear_readonly()?;
            // No path lookup here: mark the verified object, not a name that
            // another process might have redirected after the check.
            if let Err(error) = self.info() {
                if before.dwFileAttributes & FILE_ATTRIBUTE_READONLY != 0 {
                    self.set_attributes(before.dwFileAttributes)?;
                }
                return Err(error);
            }
            let disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
            if unsafe {
                SetFileInformationByHandle(
                    self.handle,
                    FileDispositionInfo,
                    (&disposition as *const FILE_DISPOSITION_INFO).cast(),
                    size_of::<FILE_DISPOSITION_INFO>() as u32,
                )
            } == 0
            {
                let error = last_error("无法移除已核验的清理对象");
                if before.dwFileAttributes & FILE_ATTRIBUTE_READONLY != 0 {
                    self.set_attributes(before.dwFileAttributes)
                        .map_err(|restore| format!("{error}；恢复原只读属性也失败：{restore}"))?;
                }
                return Err(error);
            }
        }
        Ok(())
    }

    fn clear_readonly(&self) -> Result<(), String> {
        let info = self.info()?;
        if info.dwFileAttributes & FILE_ATTRIBUTE_READONLY == 0 {
            return Ok(());
        }
        if info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY == 0 && info.nNumberOfLinks != 1 {
            return Err("拒绝修改多硬链接对象的只读属性".into());
        }
        let remaining = info.dwFileAttributes & !FILE_ATTRIBUTE_READONLY;
        self.set_attributes(remaining)
    }

    fn set_attributes(&self, remaining: u32) -> Result<(), String> {
        let attributes = FILE_BASIC_INFO {
            FileAttributes: if remaining == 0 {
                FILE_ATTRIBUTE_NORMAL
            } else {
                remaining
            },
            ..FILE_BASIC_INFO::default()
        };
        if unsafe {
            SetFileInformationByHandle(
                self.handle,
                FileBasicInfo,
                (&attributes as *const FILE_BASIC_INFO).cast(),
                size_of::<FILE_BASIC_INFO>() as u32,
            )
        } == 0
        {
            return Err(last_error("无法移除自有文件只读属性"));
        }
        Ok(())
    }
}

pub(super) fn check(path: &Path) -> Result<(), String> {
    Entry::open(path, false)?.walk(false, 0)
}
pub(super) fn remove(path: &Path) -> Result<(), String> {
    let entry = Entry::open(path, true)?;
    entry.walk(false, 0)?; // no deletion if the original tree already has links
    entry.walk(true, 0)
}
#[cfg(test)]
pub(super) fn clear_readonly(path: &Path) -> Result<(), String> {
    Entry::open(path, true)?.clear_readonly()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn DeviceIoControl(
            device: HANDLE,
            code: u32,
            input: *const c_void,
            input_size: u32,
            output: *mut c_void,
            output_size: u32,
            returned: *mut u32,
            overlapped: *mut c_void,
        ) -> i32;
    }

    #[test]
    fn in_place_write_attributes_junction_attack_cannot_redirect_child_delete() {
        let fixture =
            std::env::temp_dir().join(format!("codex-relative-cleanup-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&fixture).unwrap();
        let fixture = fs::canonicalize(fixture).unwrap();
        let owned = fixture.join("owned");
        let outside = fixture.join("outside");
        fs::create_dir(&owned).unwrap();
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("keep.txt"), b"must survive").unwrap();
        let entry = Entry::open(&owned, true).unwrap();
        let wide = owned
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>();
        // This access bypasses the protection that ordinary sharing flags appear
        // to provide. It must remain a real successful attack, not a mock.
        let writer = unsafe {
            CreateFileW(
                wide.as_ptr(),
                FILE_WRITE_ATTRIBUTES,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                std::ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT,
                std::ptr::null_mut(),
            )
        };
        assert_ne!(
            writer,
            INVALID_HANDLE_VALUE,
            "{}",
            io::Error::last_os_error()
        );
        let writer = Entry {
            handle: writer,
            _ancestors: Vec::new(),
        };
        let print = outside
            .to_string_lossy()
            .trim_start_matches(r"\\?\")
            .to_string();
        let substitute = format!(r"\??\{print}").encode_utf16().collect::<Vec<_>>();
        let print = print.encode_utf16().collect::<Vec<_>>();
        let mut buffer = Vec::new();
        buffer.extend_from_slice(&0xa0000003_u32.to_le_bytes());
        buffer.extend_from_slice(
            &((8 + (substitute.len() + print.len() + 2) * 2) as u16).to_le_bytes(),
        );
        buffer.extend_from_slice(&0_u16.to_le_bytes());
        for value in [
            0,
            (substitute.len() * 2) as u16,
            ((substitute.len() + 1) * 2) as u16,
            (print.len() * 2) as u16,
        ] {
            buffer.extend_from_slice(&value.to_le_bytes());
        }
        for unit in substitute
            .iter()
            .chain(Some(&0))
            .chain(print.iter())
            .chain(Some(&0))
        {
            buffer.extend_from_slice(&unit.to_le_bytes());
        }
        let result = entry.child_after_check(OsStr::new("keep.txt"), true, || {
            let mut returned = 0;
            assert_ne!(
                unsafe {
                    DeviceIoControl(
                        writer.handle,
                        0x000900a4,
                        buffer.as_ptr().cast(),
                        buffer.len() as u32,
                        std::ptr::null_mut(),
                        0,
                        &mut returned,
                        std::ptr::null_mut(),
                    )
                },
                0,
                "FSCTL attack must succeed: {}",
                io::Error::last_os_error()
            );
        });
        assert!(
            result.is_err(),
            "relative open must reject parent reparse after its metadata check"
        );
        assert_eq!(fs::read(outside.join("keep.txt")).unwrap(), b"must survive");
        assert!(entry.walk(true, 0).is_err());
        assert_eq!(fs::read(outside.join("keep.txt")).unwrap(), b"must survive");
        // Remove only the fixture reparse metadata; never recurse through it.
        let mut deletion = [0_u8; 8];
        deletion[..4].copy_from_slice(&0xa0000003_u32.to_le_bytes());
        let mut returned = 0;
        assert_ne!(
            unsafe {
                DeviceIoControl(
                    writer.handle,
                    0x000900ac,
                    deletion.as_ptr().cast(),
                    deletion.len() as u32,
                    std::ptr::null_mut(),
                    0,
                    &mut returned,
                    std::ptr::null_mut(),
                )
            },
            0
        );
        drop(writer);
        drop(entry);
        remove(&fixture).unwrap();
        assert!(!fixture.exists());
    }

    #[test]
    fn native_cleanup_rejects_broad_unc_and_relative_targets() {
        for path in [
            r"C:\",
            r"\\?\C:\",
            r"C:relative",
            r"relative",
            r"\\server\share\data",
            r"C:\one\..\two",
        ] {
            assert!(Entry::open(Path::new(path), true).is_err(), "{path}");
        }
    }

    #[test]
    fn wide_directory_enumeration_and_readonly_folder_are_supported() {
        let root = std::env::temp_dir().join(format!(
            "codex-relative-cleanup-wide-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir(&root).unwrap();
        for index in 0..700 {
            fs::write(
                root.join(format!("内容-{index:04}-{}", "long".repeat(18))),
                b"owned fixture",
            )
            .unwrap();
        }
        let mut permissions = fs::metadata(&root).unwrap().permissions();
        permissions.set_readonly(true);
        fs::set_permissions(&root, permissions).unwrap();
        check(&root).unwrap();
        remove(&root).unwrap();
        assert!(!root.exists());
    }
}
