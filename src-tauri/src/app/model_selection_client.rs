//! Finite metadata-only RPC to the existing CCSwitch configuration writer.
//! Native readiness stays in the manager; this transport never accepts proof.
use crate::app::model_diagnostics::safe_identifier;
use serde::{Deserialize, Serialize};
use std::sync::atomic::AtomicBool;
#[cfg(windows)]
use std::sync::atomic::Ordering;
#[cfg(windows)]
use std::time::{Duration, Instant};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SelectionRequest<'a> {
    pub schema_version: u32,
    pub request_id: &'a str,
    pub operation: &'a str,
    pub expected_revision: Option<&'a str>,
    pub model: Option<&'a str>,
    pub catalog_path: Option<&'a str>,
    pub catalog_sha256: Option<&'a str>,
}
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SelectionReply {
    pub schema_version: u32,
    pub request_id: String,
    pub ok: bool,
    pub code: String,
    pub revision: Option<String>,
    pub selected_model: Option<String>,
    pub config_sha256: Option<String>,
    pub applied: bool,
}
fn hash_string(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit())
}
fn decode(
    bytes: &[u8],
    request: &SelectionRequest<'_>,
    key: &str,
) -> Result<SelectionReply, &'static str> {
    if request.schema_version != 3
        || !matches!(request.operation, "snapshot" | "apply")
        || (request.operation == "apply"
            && !request
                .model
                .is_some_and(|m| !m.is_empty() && safe_identifier(m)))
    {
        return Err("selection-response-invalid");
    }
    let reply: SelectionReply =
        serde_json::from_slice(bytes).map_err(|_| "selection-response-invalid")?;
    if reply.schema_version != request.schema_version || reply.request_id != request.request_id {
        return Err("selection-response-invalid");
    }
    if !reply.ok {
        // Do not forward arbitrary helper data into UI or diagnostic output.
        return Err(match reply.code.as_str() {
            "user-model-conflict" => "user-model-conflict",
            "revision-conflict" | "revision-mismatch" => "selection-revision-conflict",
            "endpoint-conflict" | "endpoint-mismatch" => "selection-endpoint-conflict",
            "codex-home-conflict" => "selection-home-conflict",
            "binding-context-mismatch" | "binding-context-missing" => "selection-binding-changed",
            "catalog-conflict" | "catalog-revision-mismatch" | "catalog-model-conflict" => {
                "selection-catalog-conflict"
            }
            "apply-rolled-back" => "selection-rolled-back",
            "rollback-incomplete" | "rollback-conflict" => "selection-rollback-incomplete",
            "user-provider-conflict" | "user-auth-conflict" => "selection-user-config-conflict",
            "unverified-model-options" => "selection-model-options-unverified",
            "proxy-takeover" | "proxy-takeover-unsupported" => "selection-proxy-takeover",
            _ => "selection-bridge-failed",
        });
    }
    if reply.code != "ok"
        || !reply.revision.as_deref().is_some_and(hash_string)
        || reply
            .selected_model
            .as_deref()
            .is_some_and(|m| !safe_identifier(m) || m.contains(key))
        || reply
            .config_sha256
            .as_deref()
            .is_some_and(|h| !hash_string(h))
        || (request.operation == "snapshot" && reply.applied)
        || (request.operation == "apply"
            && (!reply.applied
                || reply.selected_model.as_deref() != request.model
                || reply.config_sha256.is_none()))
    {
        return Err("selection-response-invalid");
    }
    Ok(reply)
}

#[cfg(not(windows))]
pub(crate) fn request(
    _request: SelectionRequest<'_>,
    _binding: &codex_usb_license::UsbAccountBinding,
    _cancelled: &AtomicBool,
    _codex_home: &std::path::Path,
) -> Result<SelectionReply, &'static str> {
    Err("native-platform-unsupported")
}

#[cfg(windows)]
fn run_selection_process(
    mut command: std::process::Command,
    input: Vec<u8>,
    cancelled: &AtomicBool,
    deadline: Instant,
) -> Result<(Vec<u8>, bool), &'static str> {
    use std::io::{Read, Write};
    use std::os::windows::io::AsRawHandle;
    use std::os::windows::process::CommandExt;
    use std::process::Stdio;
    use std::sync::mpsc;
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };

    struct Job(HANDLE);
    impl Job {
        fn close(mut self) -> Result<(), &'static str> {
            let terminated = unsafe { TerminateJobObject(self.0, 1) } != 0;
            let closed = unsafe { CloseHandle(self.0) } != 0;
            if closed {
                self.0 = std::ptr::null_mut();
            }
            if terminated && closed {
                Ok(())
            } else {
                Err("selection-cleanup-failed")
            }
        }
    }
    impl Drop for Job {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe {
                    TerminateJobObject(self.0, 1);
                    CloseHandle(self.0);
                }
            }
        }
    }

    if cancelled.load(Ordering::Acquire) {
        return Err("selection-cancelled");
    }
    if Instant::now() >= deadline {
        return Err("selection-timeout");
    }
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .creation_flags(0x08000000 | 0x00000004);

    let job = unsafe {
        let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if handle.is_null() {
            return Err("selection-containment-unavailable");
        }
        let job = Job(handle);
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        if SetInformationJobObject(
            handle,
            JobObjectExtendedLimitInformation,
            &limits as *const _ as *const _,
            std::mem::size_of_val(&limits) as u32,
        ) == 0
        {
            return Err("selection-containment-unavailable");
        }
        job
    };
    if cancelled.load(Ordering::Acquire) {
        return Err("selection-cancelled");
    }
    if Instant::now() >= deadline {
        return Err("selection-timeout");
    }
    let mut child = command.spawn().map_err(|_| "selection-spawn-failed")?;
    if unsafe { AssignProcessToJobObject(job.0, child.as_raw_handle()) } == 0 {
        let _ = child.kill();
        let _ = child.wait();
        return Err("selection-containment-unavailable");
    }
    if crate::app::model_process::resume_suspended(&child).is_err() {
        let _ = child.kill();
        let _ = child.wait();
        return Err("selection-containment-unavailable");
    }
    // No request bytes are written until the initially suspended child is contained.
    let (mut stdin, stdout) = match (child.stdin.take(), child.stdout.take()) {
        (Some(stdin), Some(stdout)) => (stdin, stdout),
        _ => {
            let cleanup = job.close();
            let _ = child.kill();
            let waited = child.wait();
            if cleanup.is_err() || waited.is_err() {
                return Err("selection-cleanup-failed");
            }
            return Err("selection-stdio-unavailable");
        }
    };
    let writer = std::thread::spawn(move || stdin.write_all(&input));
    let (tx, rx) = mpsc::sync_channel(1);
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = stdout.take(32769).read_to_end(&mut bytes);
        let _ = tx.send(if result.is_ok() && bytes.len() <= 32768 {
            Ok(bytes)
        } else {
            Err(())
        });
    });
    let outcome = loop {
        if cancelled.load(Ordering::Acquire) {
            break Err("selection-cancelled");
        }
        if Instant::now() >= deadline {
            break Err("selection-timeout");
        }
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status.success()),
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(_) => break Err("selection-process-failed"),
        }
    };
    // Confirm cleanup before accepting a reply, even when stdout looks valid.
    let cleanup = job.close();
    let _ = child.kill();
    let waited = child.wait();
    let writer_ok = writer.join().ok().is_some_and(|result| result.is_ok());
    let reader_ok = reader.join().is_ok();
    if cleanup.is_err() || waited.is_err() || !reader_ok {
        return Err("selection-cleanup-failed");
    }
    let exited_successfully = outcome?;
    if !writer_ok {
        return Err("selection-write-failed");
    }
    let bytes = rx
        .recv()
        .map_err(|_| "selection-read-failed")?
        .map_err(|_| "selection-read-failed")?;
    Ok((bytes, exited_successfully))
}

#[cfg(windows)]
pub(crate) fn request(
    request: SelectionRequest<'_>,
    binding: &codex_usb_license::UsbAccountBinding,
    cancelled: &AtomicBool,
    codex_home: &std::path::Path,
) -> Result<SelectionReply, &'static str> {
    use std::process::{Command, Stdio};
    let deadline = Instant::now() + Duration::from_secs(30);
    if !matches!(request.operation, "snapshot" | "apply") || cancelled.load(Ordering::Acquire) {
        return Err("selection-cancelled");
    }
    let executable = crate::app::ccswitch::verified_usb_selection_executable()
        .map_err(|_| "selection-binary-unavailable")?;
    let current = crate::app::usb_guard::authorized_account_binding()
        .map_err(|_| "selection-binding-changed")?;
    if current.as_ref() != Some(binding) {
        return Err("selection-binding-changed");
    }
    codex_usb_license::validate_account_binding(binding)
        .map_err(|_| "selection-binding-changed")?;
    let context = serde_json::to_vec(&(
        &binding.ccswitch.provider_id,
        &binding.account_ref,
        &binding.ccswitch.credential_id,
        binding.ccswitch.group_id,
        binding.sub2api_user_id,
    ))
    .map_err(|_| "selection-request-invalid")?;
    let mut payload = serde_json::to_value(&request).map_err(|_| "selection-request-invalid")?;
    payload["bindingFingerprint"] =
        serde_json::json!(crate::app::model_diagnostics::digest_hex(&context));
    let input = serde_json::to_vec(&payload).map_err(|_| "selection-request-invalid")?;
    if input.len() > 4096 {
        return Err("selection-request-invalid");
    }
    let mut command = Command::new(&executable);
    command
        .arg("--usb-model-selection")
        .env("CODEX_USB_EXPECTED_CODEX_HOME", codex_home)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if let Some(parent) = executable.parent() {
        command.current_dir(parent);
    }
    for (name, value) in crate::app::ccswitch::selection_environment(binding) {
        command.env(name, value);
    }
    command
        .env(
            "CCSWITCH_404TOKEN_CREDENTIAL_ID",
            &binding.ccswitch.credential_id,
        )
        .env(
            "CCSWITCH_404TOKEN_USER_ID",
            binding.sub2api_user_id.to_string(),
        );
    let (bytes, exited_successfully) = run_selection_process(command, input, cancelled, deadline)?;
    let reply = decode(&bytes, &request, binding.ccswitch.device_api_key.trim())?;
    if !exited_successfully {
        return Err("selection-helper-failed");
    }
    Ok(reply)
}

/// Same finite, thread-affine lock as the CCSwitch Codex writer. It protects
/// the final compare-and-write of our catalog reference, never model proof.
#[cfg(windows)]
pub(crate) struct ConfigurationLock {
    handle: *mut std::ffi::c_void,
    _thread: std::marker::PhantomData<std::rc::Rc<()>>,
}
#[cfg(windows)]
#[link(name = "kernel32")]
extern "system" {
    fn CreateMutexW(
        attributes: *const std::ffi::c_void,
        initial_owner: i32,
        name: *const u16,
    ) -> *mut std::ffi::c_void;
    fn WaitForSingleObject(handle: *mut std::ffi::c_void, milliseconds: u32) -> u32;
    fn ReleaseMutex(handle: *mut std::ffi::c_void) -> i32;
    fn CloseHandle(handle: *mut std::ffi::c_void) -> i32;
}
#[cfg(windows)]
impl Drop for ConfigurationLock {
    fn drop(&mut self) {
        unsafe {
            ReleaseMutex(self.handle);
            CloseHandle(self.handle);
        }
    }
}
#[cfg(windows)]
pub(crate) fn lock_configuration() -> Result<ConfigurationLock, &'static str> {
    let name: Vec<u16> = "Local\\CodexUsbModelSelection"
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let handle = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
    if handle.is_null() {
        return Err("selection-lock-unavailable");
    }
    let status = unsafe { WaitForSingleObject(handle, 10_000) };
    if status != 0 {
        if status == 0x80 {
            unsafe {
                ReleaseMutex(handle);
            }
        }
        unsafe {
            CloseHandle(handle);
        }
        return Err("selection-lock-unavailable");
    }
    Ok(ConfigurationLock {
        handle,
        _thread: std::marker::PhantomData,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn fixture_request(operation: &str) -> SelectionRequest<'_> {
        const HASH: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let apply = operation == "apply";
        SelectionRequest {
            schema_version: 3,
            request_id: "fixture",
            operation,
            expected_revision: apply.then_some(HASH),
            model: apply.then_some("vendor-model"),
            catalog_path: apply.then_some("C:/fixture/codex-usb-native-model-catalog.json"),
            catalog_sha256: apply.then_some(HASH),
        }
    }
    fn fixture_reply() -> serde_json::Value {
        json!({"schemaVersion":3,"requestId":"fixture","ok":true,"code":"ok",
            "revision":"a".repeat(64),"selectedModel":"vendor-model",
            "configSha256":"b".repeat(64),"applied":true})
    }
    fn rejected(value: &serde_json::Value, request: &SelectionRequest<'_>) -> bool {
        decode(&serde_json::to_vec(value).unwrap(), request, "fixture-key").is_err()
    }
    #[cfg(windows)]
    fn powershell_fixture(script: &str) -> std::process::Command {
        use std::process::{Command, Stdio};

        let mut command = Command::new("powershell.exe");
        command
            .args([
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                script,
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        command
    }
    #[test]
    fn decoder_requires_matching_protocol_and_supported_request_operation() {
        let value = fixture_reply();
        let mut request = fixture_request("apply");
        request.schema_version = 2;
        assert!(rejected(&value, &request));
        request.schema_version = 3;
        request.operation = "unknown";
        assert!(rejected(&value, &request));
        request.operation = "apply";
        request.model = None;
        assert!(rejected(&value, &request));
        request.model = Some("");
        assert!(rejected(&value, &request));
        for version in [json!(2), json!("3"), json!(true)] {
            let mut bad = value.clone();
            bad["schemaVersion"] = version;
            assert!(rejected(&bad, &fixture_request("apply")));
        }
    }
    #[test]
    fn decoder_requires_hashes_exact_model_and_consistent_success_code() {
        let request = fixture_request("apply");
        let value = fixture_reply();
        for field in ["revision", "configSha256"] {
            for wrong in [
                serde_json::Value::Null,
                json!(""),
                json!("a".repeat(63)),
                json!("z".repeat(64)),
                json!(false),
            ] {
                let mut bad = value.clone();
                bad[field] = wrong;
                assert!(rejected(&bad, &request));
            }
            let mut missing = value.clone();
            missing.as_object_mut().unwrap().remove(field);
            assert!(rejected(&missing, &request));
        }
        for wrong in [
            serde_json::Value::Null,
            json!("different-model"),
            json!(true),
        ] {
            let mut bad = value.clone();
            bad["selectedModel"] = wrong;
            assert!(rejected(&bad, &request));
        }
        for wrong in ["apply-rolled-back", "unexpected", ""] {
            let mut bad = value.clone();
            bad["code"] = json!(wrong);
            assert!(rejected(&bad, &request));
        }
    }
    #[test]
    fn decoder_distinguishes_snapshot_from_acknowledged_application() {
        let value = fixture_reply();
        assert!(rejected(&value, &fixture_request("snapshot")));
        let mut snapshot = value.clone();
        snapshot["applied"] = json!(false);
        snapshot["selectedModel"] = serde_json::Value::Null;
        snapshot["configSha256"] = serde_json::Value::Null;
        assert!(!rejected(&snapshot, &fixture_request("snapshot")));
        assert!(rejected(&snapshot, &fixture_request("apply")));
        for field in ["ok", "applied"] {
            for wrong in [json!("true"), json!(1), serde_json::Value::Null] {
                let mut bad = value.clone();
                bad[field] = wrong;
                assert!(rejected(&bad, &fixture_request("apply")));
            }
        }
    }
    #[test]
    fn decoder_keeps_failure_codes_static_and_rejects_secret_reflection() {
        let request = fixture_request("apply");
        for (raw, safe) in [
            ("binding-context-mismatch", "selection-binding-changed"),
            ("apply-rolled-back", "selection-rolled-back"),
            ("rollback-incomplete", "selection-rollback-incomplete"),
            ("fixture-key private response", "selection-bridge-failed"),
        ] {
            let mut bad = fixture_reply();
            bad["ok"] = json!(false);
            bad["applied"] = json!(false);
            bad["code"] = json!(raw);
            let outcome = decode(&serde_json::to_vec(&bad).unwrap(), &request, "fixture-key");
            assert!(matches!(outcome, Err(code) if code == safe));
        }
        let mut snapshot = fixture_reply();
        snapshot["applied"] = json!(false);
        snapshot["selectedModel"] = json!("vendor-fixture-key-model");
        assert!(rejected(&snapshot, &fixture_request("snapshot")));
        for field in ["authorization", "prompt", "response", "password"] {
            let mut bad = fixture_reply();
            bad[field] = json!("fixture-only");
            assert!(rejected(&bad, &request));
        }
    }

    #[test]
    fn selection_reply_is_metadata_only_and_bound_to_the_request() {
        let request = fixture_request("apply");
        let value = json!({"schemaVersion":3,"requestId":"fixture","ok":true,"code":"ok",
            "revision":"a".repeat(64),"selectedModel":"vendor-model","configSha256":"b".repeat(64),"applied":true});
        assert!(decode(
            &serde_json::to_vec(&value).unwrap(),
            &request,
            "fixture-key"
        )
        .is_ok());
        let mut bad = value.clone();
        bad["requestId"] = json!("another");
        assert!(decode(&serde_json::to_vec(&bad).unwrap(), &request, "fixture-key").is_err());
        let mut bad = value.clone();
        bad["applied"] = json!("true");
        assert!(decode(&serde_json::to_vec(&bad).unwrap(), &request, "fixture-key").is_err());
        let mut bad = value;
        bad["auth"] = json!({"secret":"fixture"});
        assert!(decode(&serde_json::to_vec(&bad).unwrap(), &request, "fixture-key").is_err());
    }

    #[cfg(windows)]
    #[test]
    fn contained_helper_process_roundtrips_selection_metadata() {
        let request = fixture_request("apply");
        let input = serde_json::to_vec(&request).expect("fixture request should serialize");
        let script = concat!(
            "$raw = [Console]::In.ReadToEnd(); ",
            "$request = ConvertFrom-Json -InputObject $raw; ",
            "$reply = [ordered]@{ schemaVersion = 3; requestId = $request.requestId; ",
            "ok = $true; code = 'ok'; revision = ('a' * 64); ",
            "selectedModel = $request.model; configSha256 = ('b' * 64); applied = $true }; ",
            "[Console]::Out.Write((ConvertTo-Json -InputObject $reply -Compress))",
        );

        let (bytes, exited_successfully) = run_selection_process(
            powershell_fixture(script),
            input,
            &AtomicBool::new(false),
            Instant::now() + Duration::from_secs(15),
        )
        .expect("contained fixture helper should finish and clean up");

        assert!(exited_successfully);
        let reply = decode(&bytes, &request, "fixture-key")
            .expect("the helper reply should be bound to the fixture request");
        assert_eq!(reply.selected_model.as_deref(), Some("vendor-model"));
        let expected_revision = "a".repeat(64);
        assert_eq!(reply.revision.as_deref(), Some(expected_revision.as_str()));
    }

    #[cfg(windows)]
    #[test]
    fn contained_helper_process_enforces_timeout_and_output_limit() {
        let timeout = run_selection_process(
            powershell_fixture("Start-Sleep -Seconds 30"),
            b"{}".to_vec(),
            &AtomicBool::new(false),
            Instant::now() + Duration::from_millis(300),
        );
        assert!(matches!(timeout, Err("selection-timeout")));

        let oversized = run_selection_process(
            powershell_fixture("[Console]::Out.Write(('x' * 32769))"),
            b"{}".to_vec(),
            &AtomicBool::new(false),
            Instant::now() + Duration::from_secs(15),
        );
        assert!(matches!(oversized, Err("selection-read-failed")));
    }
}
#[cfg(not(windows))]
pub(crate) struct ConfigurationLock;
#[cfg(not(windows))]
pub(crate) fn lock_configuration() -> Result<ConfigurationLock, &'static str> {
    Ok(ConfigurationLock)
}
