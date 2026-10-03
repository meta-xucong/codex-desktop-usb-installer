//! Resume a newly suspended child only after the caller attached its job.
#[cfg(windows)]
pub(crate) fn resume_suspended(child: &std::process::Child) -> Result<(), ()> {
    use windows_sys::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32,
    };
    use windows_sys::Win32::System::Threading::{OpenThread, ResumeThread, THREAD_SUSPEND_RESUME};
    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return Err(());
        }
        let mut entry: THREADENTRY32 = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<THREADENTRY32>() as u32;
        let mut found = None;
        let mut next = Thread32First(snapshot, &mut entry);
        let mut ambiguous = false;
        while next != 0 {
            if entry.th32OwnerProcessID == child.id() {
                if found.is_some() {
                    ambiguous = true;
                    break;
                }
                found = Some(entry.th32ThreadID);
            }
            entry.dwSize = std::mem::size_of::<THREADENTRY32>() as u32;
            next = Thread32Next(snapshot, &mut entry);
        }
        if CloseHandle(snapshot) == 0 || ambiguous {
            return Err(());
        }
        let thread = OpenThread(THREAD_SUSPEND_RESUME, 0, found.ok_or(())?);
        if thread.is_null() {
            return Err(());
        }
        let previous = ResumeThread(thread);
        let closed = CloseHandle(thread) != 0;
        // CREATE_SUSPENDED supplies exactly one suspend count. Anything else
        // is not the process state this bounded launcher created.
        if previous != 1 || !closed {
            return Err(());
        }
    }
    Ok(())
}
