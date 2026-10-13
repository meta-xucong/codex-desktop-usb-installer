//! Handle-relative traversal for the exact, already-authorized cleanup targets.
//! Sharing flags cannot prevent FILE_WRITE_ATTRIBUTES from converting an empty
//! directory to a junction. Never re-resolve an absolute child path after opening
//! its parent: NtCreateFile rejects reparses, enumeration and deletion use handles.

use sha2::{Digest, Sha256};
use std::ffi::{c_void, OsStr, OsString};
use std::io;
use std::mem::{offset_of, size_of};
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Component, Path, Prefix};
use windows_sys::Win32::Foundation::{
    CloseHandle, DuplicateHandle, DUPLICATE_SAME_ACCESS, HANDLE, INVALID_HANDLE_VALUE,
};
use windows_sys::Win32::Storage::FileSystem::*;
use windows_sys::Win32::System::Threading::GetCurrentProcess;

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
    fn duplicate(&self) -> Result<Self, String> {
        self.info()?;
        let current = unsafe { GetCurrentProcess() };
        let mut duplicate = std::ptr::null_mut();
        if unsafe {
            DuplicateHandle(
                current,
                self.handle,
                current,
                &mut duplicate,
                0,
                0,
                DUPLICATE_SAME_ACCESS,
            )
        } == 0
        {
            return Err(last_error("无法复制固定目录句柄"));
        }
        Ok(Self {
            handle: duplicate,
            // A duplicate shares the same open file object and share mode.
            // The pinned Bundle root remains owned by the cleanup operation;
            // this extra reference keeps the staging ancestor itself alive.
            _ancestors: Vec::new(),
        })
    }

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
        let access = FILE_READ_ATTRIBUTES
            | FILE_LIST_DIRECTORY
            | 0x0010_0000
            | if delete {
                DELETE | FILE_WRITE_ATTRIBUTES
            } else {
                0
            };
        self.child_with_access(name, access, after_check)
    }

    fn child_for_read(&self, name: &OsStr) -> Result<Self, String> {
        self.child_with_access(
            name,
            FILE_READ_ATTRIBUTES | FILE_LIST_DIRECTORY | 0x0010_0000 | 0x0000_0001,
            || {},
        )
    }

    fn child_for_delete_read(&self, name: &OsStr) -> Result<Self, String> {
        self.child_with_access_and_share(
            name,
            FILE_READ_ATTRIBUTES
                | FILE_LIST_DIRECTORY
                | 0x0010_0000
                | 0x0000_0001
                | DELETE
                | FILE_WRITE_ATTRIBUTES,
            FILE_SHARE_READ,
            || {},
        )
    }

    fn child_for_revocation_delete(&self, name: &OsStr) -> Result<Self, String> {
        self.child_with_access_and_share(
            name,
            FILE_READ_ATTRIBUTES
                | FILE_LIST_DIRECTORY
                | 0x0010_0000
                | 0x0000_0001
                | DELETE
                | FILE_WRITE_ATTRIBUTES,
            FILE_SHARE_READ,
            || {},
        )
    }

    fn child_entry_for_revocation(&self, name: &OsStr) -> Result<Self, String> {
        self.child_for_revocation_delete(name)
    }

    fn child_with_access(
        &self,
        name: &OsStr,
        access: u32,
        after_check: impl FnOnce(),
    ) -> Result<Self, String> {
        self.child_with_access_and_share(
            name,
            access,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            after_check,
        )
    }

    fn child_with_access_and_share(
        &self,
        name: &OsStr,
        access: u32,
        share: u32,
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
                access,
                &mut attributes,
                &mut status,
                std::ptr::null(),
                0,
                share,
                1,                 // FILE_OPEN: never create during cleanup
                0x00200000 | 0x20, // OPEN_REPARSE_POINT | SYNCHRONOUS_IO_NONALERT
                std::ptr::null(),
                0,
            )
        };
        if result < 0 {
            if matches!(result as u32, 0xc000_0034 | 0xc000_003a) {
                return Err(format!(
                    "__REVOCATION_NOT_FOUND__无法安全打开清理子项（NTSTATUS {:#010x}）",
                    result as u32
                ));
            }
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
                if !length.is_multiple_of(2) || offset + header + length > bytes.len() {
                    return Err("目录枚举名称超界".into());
                }
                let (filename_units, remainder) =
                    bytes[offset + header..offset + header + length].as_chunks::<2>();
                debug_assert!(remainder.is_empty());
                let units = filename_units
                    .iter()
                    .map(|pair| u16::from_ne_bytes(*pair))
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

    fn collect_revocation_tree(
        self,
        retained: &mut Vec<(Entry, usize)>,
        depth: usize,
        delete: bool,
    ) -> Result<(), String> {
        if depth > 256 {
            return Err("清理目录层级异常，停止遍历".into());
        }
        let index = retained.len();
        retained.push((self, depth));
        let info = retained[index].0.info()?;
        if info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY == 0 {
            if info.nNumberOfLinks != 1 {
                return Err("拒绝清理具有多个硬链接的文件".into());
            }
            if info.dwFileAttributes & 0x0000_0040 != 0 {
                return Err("受管目录中发现未知设备对象，已停止清理".into());
            }
            return Ok(());
        }
        let names = retained[index].0.names()?;
        for name in names {
            let child = retained[index].0.child(&name, delete)?;
            child.collect_revocation_tree(retained, depth + 1, delete)?;
        }
        Ok(())
    }

    fn delete_self(&self) -> Result<(), String> {
        let before = self.info()?;
        self.clear_readonly()?;
        self.info()?;
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
        Ok(())
    }

    fn delete_revocation_self(&self) -> Result<(), String> {
        let info = self.info()?;
        if info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY == 0 && info.nNumberOfLinks != 1 {
            return Err("拒绝删除具有多个硬链接的文件".into());
        }
        self.delete_self()
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

    fn into_read_file(self) -> std::fs::File {
        use std::mem::ManuallyDrop;
        use std::os::windows::io::{FromRawHandle, RawHandle};
        let mut entry = ManuallyDrop::new(self);
        let handle = entry.handle;
        unsafe { std::ptr::drop_in_place(&mut entry._ancestors) };
        unsafe { std::fs::File::from_raw_handle(handle as RawHandle) }
    }

    fn read_bounded(&self, max_bytes: usize) -> Result<Vec<u8>, String> {
        use std::io::Read;
        let info = self.info()?;
        if info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0 || info.nNumberOfLinks != 1 {
            return Err("固定根目录下的清单不是单链接普通文件".into());
        }
        let current = unsafe { GetCurrentProcess() };
        let mut duplicate = std::ptr::null_mut();
        if unsafe {
            DuplicateHandle(
                current,
                self.handle,
                current,
                &mut duplicate,
                0,
                0,
                DUPLICATE_SAME_ACCESS,
            )
        } == 0
        {
            return Err(last_error("无法复制固定清单句柄"));
        }
        use std::os::windows::io::{FromRawHandle, RawHandle};
        let mut file = unsafe { std::fs::File::from_raw_handle(duplicate as RawHandle) };
        let mut bytes = Vec::new();
        file.by_ref()
            .take(max_bytes.saturating_add(1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|error| format!("读取固定句柄文件失败：{error}"))?;
        if bytes.len() > max_bytes {
            return Err("固定句柄文件超过读取上限".into());
        }
        Ok(bytes)
    }

    fn prepare_revocation_delete(self) -> Result<PreparedDeleteTree, String> {
        let mut retained = Vec::new();
        self.collect_revocation_tree_named(&mut retained, 0, String::new())?;
        for (entry, _, _) in &retained {
            let info = entry.info()?;
            if info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY == 0 && info.nNumberOfLinks != 1 {
                return Err("拒绝删除具有多个硬链接的文件".into());
            }
        }
        Ok(PreparedDeleteTree { retained })
    }

    fn remove_regular_file(self) -> Result<(), String> {
        let info = self.info()?;
        if info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0 || info.nNumberOfLinks != 1 {
            return Err("拒绝删除非单链接普通文件".into());
        }
        self.delete_revocation_self()
    }

    fn collect_revocation_tree_named(
        self,
        retained: &mut Vec<(Entry, usize, String)>,
        depth: usize,
        relative: String,
    ) -> Result<(), String> {
        if depth > 256 {
            return Err("清理目录层级异常，停止遍历".into());
        }
        let index = retained.len();
        retained.push((self, depth, relative));
        let info = retained[index].0.info()?;
        if info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY == 0 {
            if info.nNumberOfLinks != 1 {
                return Err("拒绝清理具有多个硬链接的文件".into());
            }
            if info.dwFileAttributes & 0x0000_0040 != 0 {
                return Err("受管目录中发现未知设备对象，已停止清理".into());
            }
            return Ok(());
        }
        let names = retained[index].0.names()?;
        for name in names {
            let text = name
                .to_str()
                .ok_or_else(|| "固定事务目录包含非 UTF-8 文件名".to_string())?;
            let child_relative = if retained[index].2.is_empty() {
                text.to_string()
            } else {
                format!("{}/{text}", retained[index].2)
            };
            let child = retained[index].0.child_entry_for_revocation(&name)?;
            child.collect_revocation_tree_named(retained, depth + 1, child_relative)?;
        }
        Ok(())
    }
}

pub(super) struct PreparedDeleteTree {
    retained: Vec<(Entry, usize, String)>,
}

impl PreparedDeleteTree {
    pub(super) fn delete(self) -> Result<(), String> {
        self.delete_with_hook(|_| Ok(()))
    }

    #[cfg(test)]
    pub(super) fn delete_with_test_hook(
        self,
        before_item: impl FnMut(usize) -> Result<(), String>,
    ) -> Result<(), String> {
        self.delete_with_hook(before_item)
    }

    pub(super) fn read_relative_bounded(
        &self,
        relative: &str,
        max_bytes: usize,
    ) -> Result<Option<Vec<u8>>, String> {
        let Some((entry, _, _)) = self
            .retained
            .iter()
            .find(|(_, _, candidate)| candidate.eq_ignore_ascii_case(relative))
        else {
            return Ok(None);
        };
        entry.read_bounded(max_bytes).map(Some)
    }

    fn delete_with_hook(
        self,
        mut before_item: impl FnMut(usize) -> Result<(), String>,
    ) -> Result<(), String> {
        // Revalidate the complete pinned set before the first disposition.
        for (entry, _, _) in &self.retained {
            let info = entry.info()?;
            if info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY == 0 && info.nNumberOfLinks != 1 {
                return Err("拒绝删除具有多个硬链接的文件".into());
            }
        }
        for (index, (entry, _depth, _relative)) in self.retained.into_iter().rev().enumerate() {
            before_item(index)?;
            entry.delete_revocation_self()?;
        }
        Ok(())
    }
}

pub(super) struct PinnedEntry {
    entry: Option<Entry>,
}

impl PinnedEntry {
    pub(super) fn duplicate(&self) -> Result<PinnedEntry, String> {
        let entry = self
            .entry
            .as_ref()
            .ok_or_else(|| "固定条目句柄已消耗".to_string())?;
        Ok(PinnedEntry {
            entry: Some(entry.duplicate()?),
        })
    }

    pub(super) fn is_directory(&self) -> Result<bool, String> {
        let entry = self
            .entry
            .as_ref()
            .ok_or_else(|| "固定条目句柄已消耗".to_string())?;
        Ok(entry.info()?.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0)
    }

    pub(super) fn read_bounded(&self, max_bytes: usize) -> Result<Vec<u8>, String> {
        self.entry
            .as_ref()
            .ok_or_else(|| "固定条目句柄已消耗".to_string())?
            .read_bounded(max_bytes)
    }

    pub(super) fn open_child(&self, name: &str) -> Result<Option<PinnedEntry>, String> {
        let parent = self
            .entry
            .as_ref()
            .ok_or_else(|| "固定条目句柄已消耗".to_string())?;
        match parent.child_entry_for_revocation(OsStr::new(name)) {
            Ok(entry) => Ok(Some(PinnedEntry { entry: Some(entry) })),
            Err(error) if error.starts_with("__REVOCATION_NOT_FOUND__") => Ok(None),
            Err(error) => Err(error),
        }
    }

    pub(super) fn list_names(&self) -> Result<Vec<OsString>, String> {
        let entry = self
            .entry
            .as_ref()
            .ok_or_else(|| "固定条目句柄已消耗".to_string())?;
        if entry.info()?.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY == 0 {
            return Err("固定条目不是目录".into());
        }
        entry.names()
    }

    pub(super) fn prepare_delete_tree(mut self) -> Result<PreparedDeleteTree, String> {
        self.entry
            .take()
            .ok_or_else(|| "固定条目句柄已消耗".to_string())?
            .prepare_revocation_delete()
    }

    pub(super) fn remove_regular_file(mut self) -> Result<(), String> {
        self.entry
            .take()
            .ok_or_else(|| "固定条目句柄已消耗".to_string())?
            .remove_regular_file()
    }
}

pub(super) struct PinnedRoot {
    root: Entry,
    volume_serial: u64,
}

impl PinnedRoot {
    pub(super) fn open(path: &Path, expected_volume_serial: u64) -> Result<Self, String> {
        let root = Entry::open(path, false)?;
        let info = root.info()?;
        if info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY == 0 {
            return Err("作废 Bundle 根句柄不是目录".into());
        }
        let volume_serial = u64::from(info.dwVolumeSerialNumber);
        if volume_serial != expected_volume_serial {
            return Err("固定的 Bundle 根句柄卷序列号与已观察 USB 指纹不一致".into());
        }
        Ok(Self {
            root,
            volume_serial,
        })
    }

    #[cfg(test)]
    pub(super) fn open_for_test(path: &Path) -> Result<Self, String> {
        let root = Entry::open(path, false)?;
        let info = root.info()?;
        if info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY == 0 {
            return Err("测试 Bundle 根句柄不是目录".into());
        }
        Ok(Self {
            volume_serial: u64::from(info.dwVolumeSerialNumber),
            root,
        })
    }

    pub(super) fn volume_serial(&self) -> u64 {
        self.volume_serial
    }

    fn open_relative(
        &self,
        relative: &str,
        delete: bool,
        read_data: bool,
    ) -> Result<Entry, String> {
        let names = relative.split('/').collect::<Vec<_>>();
        if names.is_empty()
            || names.iter().any(|name| {
                name.is_empty()
                    || *name == "."
                    || *name == ".."
                    || name.contains('\\')
                    || name.contains(':')
            })
        {
            return Err("拒绝非受控相对目标路径".into());
        }
        let mut current: Option<Entry> = None;
        let mut ancestors = Vec::new();
        for (index, name) in names.iter().enumerate() {
            let parent = current.as_ref().unwrap_or(&self.root);
            let next = if delete && index + 1 == names.len() {
                parent.child_for_revocation_delete(OsStr::new(name))?
            } else if read_data && index + 1 == names.len() {
                if delete {
                    parent.child_for_delete_read(OsStr::new(name))?
                } else {
                    parent.child_for_read(OsStr::new(name))?
                }
            } else {
                parent.child(OsStr::new(name), delete)?
            };
            if let Some(previous) = current.replace(next) {
                ancestors.push(previous);
            }
        }
        let mut target = current.ok_or_else(|| "清理目标路径为空".to_string())?;
        target._ancestors = ancestors;
        Ok(target)
    }

    pub(super) fn read_relative_bounded(
        &self,
        relative: &str,
        max_bytes: usize,
    ) -> Result<Vec<u8>, String> {
        use std::io::Read;
        let entry = self.open_relative(relative, false, true)?;
        let info = entry.info()?;
        if info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0 || info.nNumberOfLinks != 1 {
            return Err("固定根目录下的清单不是单链接普通文件".into());
        }
        let mut file = entry.into_read_file();
        let mut bytes = Vec::new();
        file.by_ref()
            .take(max_bytes.saturating_add(1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|error| format!("读取固定根目录相对文件失败：{error}"))?;
        if bytes.len() > max_bytes {
            return Err("固定根目录相对文件超过读取上限".into());
        }
        Ok(bytes)
    }

    pub(super) fn check_relative(&self, relative: &str) -> Result<(), String> {
        let root = match self.open_relative(relative, false, false) {
            Ok(root) => root,
            Err(error) if error.starts_with("__REVOCATION_NOT_FOUND__") => return Ok(()),
            Err(error) => return Err(error),
        };
        let mut retained = Vec::new();
        root.collect_revocation_tree(&mut retained, 0, false)
    }

    pub(super) fn open_pinned_entry(&self, relative: &str) -> Result<Option<PinnedEntry>, String> {
        // This entry is used to pin `.staging` as a parent during transaction
        // inventory/removal. It must deny rename/delete sharing without
        // requesting DELETE itself, so the same directory can still be
        // re-opened read-only for post-delete existence checks.
        match self.open_relative(relative, false, true) {
            Ok(entry) => Ok(Some(PinnedEntry { entry: Some(entry) })),
            Err(error) if error.starts_with("__REVOCATION_NOT_FOUND__") => Ok(None),
            Err(error) => Err(error),
        }
    }

    pub(super) fn exists_relative(&self, relative: &str) -> Result<bool, String> {
        match self.open_relative(relative, false, false) {
            Ok(_entry) => Ok(true),
            Err(error) if error.starts_with("__REVOCATION_NOT_FOUND__") => Ok(false),
            Err(error) => Err(error),
        }
    }

    pub(super) fn validate_backup(&self, relative: &str, architecture: &str) -> Result<(), String> {
        let backup = match self.open_relative(relative, false, false) {
            Ok(backup) => backup,
            Err(error) if error.starts_with("__REVOCATION_NOT_FOUND__") => return Ok(()),
            Err(error) => return Err(error),
        };
        if backup.info()?.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY == 0 {
            return Err("Codex 载荷备份路径不是目录".into());
        }
        let names = backup.names()?;
        if names.len() > 1 {
            return Err("Codex 载荷备份最多只能包含一个直接子文件".into());
        }
        let Some(name) = names.first() else {
            return Ok(());
        };
        let name_text = name.to_string_lossy();
        let prefix = format!("OpenAI.Codex_{architecture}_");
        let version = name_text
            .strip_prefix(&prefix)
            .and_then(|value| value.strip_suffix(".msix"))
            .ok_or_else(|| "Codex 载荷备份文件名无效".to_string())?;
        if version.is_empty()
            || !version
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
        {
            return Err("Codex 载荷备份版本名无效".into());
        }
        let child = backup.child(name, false)?;
        let info = child.info()?;
        if info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0 || info.nNumberOfLinks != 1 {
            return Err("Codex 载荷备份必须是单链接普通 MSIX 文件".into());
        }
        if (u64::from(info.nFileSizeHigh) << 32) | u64::from(info.nFileSizeLow) == 0 {
            return Err("Codex 载荷备份不能为空".into());
        }
        Ok(())
    }

    pub(super) fn remove_relative(&self, relative: &str) -> Result<(), String> {
        self.remove_relative_with_hook(relative, |_| {})
    }

    pub(super) fn remove_regular_file_relative_if_sha256(
        &self,
        relative: &str,
        expected_sha256: &str,
    ) -> Result<(), String> {
        self.remove_regular_file_relative_if_sha256_with_hook(relative, expected_sha256, || {})
    }

    fn remove_regular_file_relative_if_sha256_with_hook(
        &self,
        relative: &str,
        expected_sha256: &str,
        after_hash: impl FnOnce(),
    ) -> Result<(), String> {
        if expected_sha256.len() != 64
            || !expected_sha256.bytes().all(|byte| byte.is_ascii_hexdigit())
        {
            return Err("拒绝使用无效 SHA-256 删除恢复载荷".into());
        }
        let entry = match self.open_relative(relative, true, true) {
            Ok(entry) => entry,
            Err(error) if error.starts_with("__REVOCATION_NOT_FOUND__") => return Ok(()),
            Err(error) => return Err(error),
        };
        let before = entry.info()?;
        if before.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0 || before.nNumberOfLinks != 1 {
            return Err("恢复载荷必须是普通单链接文件".into());
        }
        let expected_size =
            (u64::from(before.nFileSizeHigh) << 32) | u64::from(before.nFileSizeLow);
        let actual_sha256 = Self::sha256_read_handle(entry.handle, expected_size)?;
        if !actual_sha256.eq_ignore_ascii_case(expected_sha256) {
            return Err("恢复载荷 SHA-256 与事务记录不一致；保留目标文件".into());
        }
        after_hash();
        let after = entry.info()?;
        let after_size = (u64::from(after.nFileSizeHigh) << 32) | u64::from(after.nFileSizeLow);
        if after.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY != 0
            || after.nNumberOfLinks != 1
            || after_size != expected_size
        {
            return Err("恢复载荷在句柄校验期间发生变化；保留目标文件".into());
        }
        // The handle was opened without FILE_SHARE_WRITE or FILE_SHARE_DELETE.
        // Its identity and bytes cannot be replaced between hashing and this
        // handle-based disposition.
        entry.delete_revocation_self()
    }

    fn sha256_read_handle(
        handle: *mut std::ffi::c_void,
        expected_size: u64,
    ) -> Result<String, String> {
        use std::mem::ManuallyDrop;
        use std::os::windows::io::{FromRawHandle, RawHandle};
        let file =
            ManuallyDrop::new(unsafe { std::fs::File::from_raw_handle(handle as RawHandle) });
        let mut reader = &*file;
        let mut hasher = Sha256::new();
        let mut buffer = [0_u8; 64 * 1024];
        let mut total = 0_u64;
        loop {
            let count = std::io::Read::read(&mut reader, &mut buffer)
                .map_err(|error| format!("从固定恢复载荷句柄读取失败：{error}"))?;
            if count == 0 {
                break;
            }
            total = total
                .checked_add(count as u64)
                .ok_or_else(|| "恢复载荷长度溢出".to_string())?;
            if total > expected_size {
                return Err("恢复载荷在句柄读取期间增长；拒绝删除".into());
            }
            hasher.update(&buffer[..count]);
        }
        if total != expected_size {
            return Err("恢复载荷在句柄读取期间长度发生变化；拒绝删除".into());
        }
        Ok(hasher
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect())
    }

    fn remove_relative_with_hook(
        &self,
        relative: &str,
        before_delete: impl FnOnce(&[(Entry, usize)]),
    ) -> Result<(), String> {
        let root = match self.open_relative(relative, true, false) {
            Ok(root) => root,
            Err(error) if error.starts_with("__REVOCATION_NOT_FOUND__") => return Ok(()),
            Err(error) => return Err(error),
        };
        let mut retained = Vec::new();
        root.collect_revocation_tree(&mut retained, 0, true)?;
        before_delete(&retained);
        // Revalidate the pinned handles as a complete set before the first
        // disposition. A junction/reparse or hard link introduced after the
        // inventory aborts without partially deleting this target.
        for (entry, _) in &retained {
            let info = entry.info()?;
            if info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY == 0 && info.nNumberOfLinks != 1 {
                return Err("拒绝删除具有多个硬链接的文件".into());
            }
        }
        for (entry, _depth) in retained.into_iter().rev() {
            entry.delete_revocation_self()?;
        }
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn remove_relative_with_test_hook(
        &self,
        relative: &str,
        before_delete: impl FnOnce(&[(Entry, usize)]),
    ) -> Result<(), String> {
        self.remove_relative_with_hook(relative, before_delete)
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
    fn pinned_revocation_preflight_rejects_hardlinks_before_any_delete() {
        let fixture = std::env::temp_dir().join(format!(
            "codex-pinned-revocation-hardlink-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir(&fixture).unwrap();
        let fixture = fs::canonicalize(fixture).unwrap();
        let owned = fixture.join("owned");
        let outside = fixture.join("outside.txt");
        fs::create_dir(&owned).unwrap();
        fs::write(owned.join("shared.txt"), b"must keep both links").unwrap();
        fs::hard_link(owned.join("shared.txt"), &outside).unwrap();
        let volume = Entry::open(&fixture, false)
            .unwrap()
            .info()
            .unwrap()
            .dwVolumeSerialNumber as u64;
        assert!(PinnedRoot::open(&fixture, volume.saturating_add(1)).is_err());
        let pinned = PinnedRoot::open(&fixture, volume).unwrap();

        assert!(pinned.check_relative("owned").is_err());
        assert!(pinned.remove_relative("owned").is_err());
        assert_eq!(
            fs::read(owned.join("shared.txt")).unwrap(),
            b"must keep both links"
        );
        assert_eq!(fs::read(&outside).unwrap(), b"must keep both links");
        drop(pinned);
        remove(&fixture).unwrap();
    }

    #[test]
    fn pinned_hash_delete_rejects_path_replacement_between_hash_and_disposition() {
        use sha2::{Digest, Sha256};

        let fixture = std::env::temp_dir().join(format!(
            "codex-pinned-hash-delete-race-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir(&fixture).unwrap();
        let fixture = fs::canonicalize(fixture).unwrap();
        let payload_dir = fixture.join("payload");
        fs::create_dir(&payload_dir).unwrap();
        let payload = payload_dir.join("codex-new.msix");
        let outside = fixture.join("outside-sentinel.txt");
        let bytes = b"verified staged payload";
        fs::write(&payload, bytes).unwrap();
        fs::write(&outside, b"outside data must survive").unwrap();
        let expected_sha = Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let volume = Entry::open(&fixture, false)
            .unwrap()
            .info()
            .unwrap()
            .dwVolumeSerialNumber as u64;
        let pinned = PinnedRoot::open(&fixture, volume).unwrap();

        pinned
            .remove_regular_file_relative_if_sha256_with_hook(
                "payload/codex-new.msix",
                &expected_sha,
                || {
                    let renamed = payload_dir.join("attacker-moved.msix");
                    assert!(
                        fs::rename(&payload, &renamed).is_err(),
                        "the hashed handle must deny rename before disposition"
                    );
                    assert!(
                        fs::write(&payload, b"attacker replacement").is_err(),
                        "the hashed handle must deny write before disposition"
                    );
                    assert_eq!(fs::read(&outside).unwrap(), b"outside data must survive");
                },
            )
            .unwrap();

        assert!(!payload.exists());
        assert_eq!(fs::read(&outside).unwrap(), b"outside data must survive");
        drop(pinned);
        remove(&fixture).unwrap();
    }

    #[test]
    fn pinned_revocation_rejects_junction_swap_after_inventory() {
        let fixture = std::env::temp_dir().join(format!(
            "codex-pinned-revocation-junction-{}",
            uuid::Uuid::new_v4()
        ));
        fs::create_dir(&fixture).unwrap();
        let fixture = fs::canonicalize(fixture).unwrap();
        let owned = fixture.join("owned");
        let outside = fixture.join("outside");
        fs::create_dir(&owned).unwrap();
        fs::create_dir(owned.join("empty")).unwrap();
        fs::create_dir(&outside).unwrap();
        fs::write(owned.join("managed.txt"), b"managed").unwrap();
        fs::write(outside.join("keep.txt"), b"outside sentinel").unwrap();
        let volume = Entry::open(&fixture, false)
            .unwrap()
            .info()
            .unwrap()
            .dwVolumeSerialNumber as u64;
        let pinned = PinnedRoot::open(&fixture, volume).unwrap();
        let mut writer = INVALID_HANDLE_VALUE;
        let result = pinned.remove_relative_with_test_hook("owned", |retained| {
            let renamed = fixture.join("owned-renamed");
            assert!(
                fs::rename(&owned, &renamed).is_err(),
                "retained handle must prevent rename replacement"
            );
            let empty = owned.join("empty");
            let wide = empty
                .as_os_str()
                .encode_wide()
                .chain(Some(0))
                .collect::<Vec<_>>();
            writer = unsafe {
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
            assert!(retained[0].0.info().is_ok());
            set_junction(writer, &outside);
        });
        assert!(result.is_err());
        assert_eq!(
            fs::read(outside.join("keep.txt")).unwrap(),
            b"outside sentinel"
        );
        delete_junction(writer);
        unsafe { CloseHandle(writer) };
        drop(pinned);
        assert_eq!(fs::read(owned.join("managed.txt")).unwrap(), b"managed");
        remove(&fixture).unwrap();
    }

    fn set_junction(handle: HANDLE, target: &Path) {
        let print = target
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
        let mut returned = 0;
        assert_ne!(
            unsafe {
                DeviceIoControl(
                    handle,
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
            "junction swap fixture must be installed: {}",
            io::Error::last_os_error()
        );
    }

    fn delete_junction(handle: HANDLE) {
        let mut deletion = [0_u8; 8];
        deletion[..4].copy_from_slice(&0xa0000003_u32.to_le_bytes());
        let mut returned = 0;
        assert_ne!(
            unsafe {
                DeviceIoControl(
                    handle,
                    0x000900ac,
                    deletion.as_ptr().cast(),
                    deletion.len() as u32,
                    std::ptr::null_mut(),
                    0,
                    &mut returned,
                    std::ptr::null_mut(),
                )
            },
            0,
            "junction fixture cleanup failed: {}",
            io::Error::last_os_error()
        );
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
