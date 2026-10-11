//! Public signed-proof registration with the USB license service.
//!
//! The server accepts only the Ed25519-signed registration envelope already
//! verified locally by the signer. No Sub2API/admin credential is sent here.

use std::io::Read;
use std::time::Duration;

use codex_usb_license::RevocationRegistrationEnvelope;
use reqwest::blocking::Client;
use reqwest::redirect::Policy;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const REGISTRATION_ENDPOINT: &str = "https://license.404token.xyz/v1/licenses/register";
const USER_AGENT: &str = "codex-usb-license-signer/1";
const MAX_RESPONSE_BYTES: u64 = 4096;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ActivationError {
    #[error("VPS 暂不可用或网络请求超时；U 盘已签名，授权尚未确认，可重新扫描后重试同步")]
    Network,
    #[error("VPS 已明确返回此 U 盘授权已作废；没有重新激活")]
    Revoked,
    #[error("安装编号与 VPS 中已有授权记录冲突；没有覆盖现有记录")]
    Conflict,
    #[error("VPS 响应无效；没有确认授权已激活")]
    InvalidResponse,
    #[error("VPS 未接受登记凭证；授权尚未确认")]
    Rejected,
}

#[derive(Serialize)]
struct RegistrationUpload<'a> {
    registration: &'a RevocationRegistrationEnvelope,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ActivationResponse {
    status: String,
    installation_id: String,
    state_version: i64,
}

#[derive(Debug, Deserialize)]
struct ErrorResponse {
    error: String,
}

pub fn activate_registration(
    registration: &RevocationRegistrationEnvelope,
    expected_installation_id: &str,
) -> Result<i64, ActivationError> {
    activate_registration_at(
        REGISTRATION_ENDPOINT,
        registration,
        expected_installation_id,
    )
}

fn activate_registration_at(
    endpoint: &str,
    registration: &RevocationRegistrationEnvelope,
    expected_installation_id: &str,
) -> Result<i64, ActivationError> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    if rustls::crypto::CryptoProvider::get_default().is_none() {
        return Err(ActivationError::Network);
    }
    let client = Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(15))
        .redirect(Policy::none())
        .user_agent(USER_AGENT)
        .build()
        .map_err(|_| ActivationError::Network)?;
    let response = client
        .post(endpoint)
        .json(&RegistrationUpload { registration })
        .send()
        .map_err(|_| ActivationError::Network)?;
    let status = response.status();
    let mut limited = response.take(MAX_RESPONSE_BYTES);
    let mut body = Vec::new();
    limited
        .read_to_end(&mut body)
        .map_err(|_| ActivationError::Network)?;
    validate_response(status, &body, expected_installation_id)
}

fn validate_response(
    status: StatusCode,
    body: &[u8],
    expected_installation_id: &str,
) -> Result<i64, ActivationError> {
    if status == StatusCode::CONFLICT {
        let error: ErrorResponse =
            serde_json::from_slice(body).map_err(|_| ActivationError::InvalidResponse)?;
        return match error.error.as_str() {
            "license_revoked" => Err(ActivationError::Revoked),
            "registration_conflict" => Err(ActivationError::Conflict),
            _ => Err(ActivationError::Rejected),
        };
    }
    if status != StatusCode::OK && status != StatusCode::CREATED {
        return Err(ActivationError::Rejected);
    }
    let response: ActivationResponse =
        serde_json::from_slice(body).map_err(|_| ActivationError::InvalidResponse)?;
    if response.status != "active"
        || response.installation_id != expected_installation_id
        || response.state_version < 1
    {
        return Err(ActivationError::InvalidResponse);
    }
    Ok(response.state_version)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    #[test]
    fn accepts_only_matching_active_registration_responses() {
        let body = br#"{"status":"active","installationId":"usb-install-001","stateVersion":1,"created":true}"#;
        assert_eq!(
            validate_response(StatusCode::CREATED, body, "usb-install-001"),
            Ok(1)
        );
        assert_eq!(
            validate_response(StatusCode::OK, body, "usb-install-001"),
            Ok(1)
        );
        assert_eq!(
            validate_response(StatusCode::OK, body, "usb-install-other"),
            Err(ActivationError::InvalidResponse)
        );
        let revoked = br#"{"error":"license_revoked"}"#;
        assert_eq!(
            validate_response(StatusCode::CONFLICT, revoked, "usb-install-001"),
            Err(ActivationError::Revoked)
        );
        let conflict = br#"{"error":"registration_conflict"}"#;
        assert_eq!(
            validate_response(StatusCode::CONFLICT, conflict, "usb-install-001"),
            Err(ActivationError::Conflict)
        );
    }

    #[test]
    fn posts_only_registration_proof_to_loopback_test_endpoint() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind local test server");
        let endpoint = format!(
            "http://{}/v1/licenses/register",
            listener.local_addr().unwrap()
        );
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept signer request");
            let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
            let mut request = Vec::new();
            let mut chunk = [0_u8; 2048];
            let header_end;
            loop {
                let read = stream.read(&mut chunk).expect("read request headers");
                request.extend_from_slice(&chunk[..read]);
                if let Some(index) = request.windows(4).position(|window| window == b"\r\n\r\n") {
                    header_end = index + 4;
                    break;
                }
            }
            let headers = String::from_utf8_lossy(&request[..header_end]);
            assert!(headers.starts_with("POST /v1/licenses/register HTTP/1.1"));
            assert!(headers
                .to_ascii_lowercase()
                .contains("content-type: application/json"));
            let content_length = headers
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("content-length")
                        .then(|| value.trim().parse::<usize>().ok())
                        .flatten()
                })
                .expect("content length");
            while request.len() < header_end + content_length {
                let read = stream.read(&mut chunk).expect("read request body");
                request.extend_from_slice(&chunk[..read]);
            }
            let body = &request[header_end..header_end + content_length];
            let body_text = String::from_utf8_lossy(body);
            assert!(body_text.contains("payloadB64"));
            assert!(body_text.contains("signatureB64"));
            assert!(!body_text.contains("accountBinding"));
            assert!(!body_text.contains("apiKey"));
            let response_body = br#"{"status":"active","installationId":"usb-install-001","stateVersion":1,"created":true}"#;
            let response_headers = format!(
                "HTTP/1.1 201 Created\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                response_body.len()
            );
            stream
                .write_all(response_headers.as_bytes())
                .and_then(|_| stream.write_all(response_body))
                .expect("write activation response");
        });

        let proof = RevocationRegistrationEnvelope {
            payload_b64: "signed-payload".to_string(),
            signature_b64: "signed-proof".to_string(),
        };
        assert_eq!(
            activate_registration_at(&endpoint, &proof, "usb-install-001"),
            Ok(1)
        );
        server.join().expect("local test server completed");
    }
}
