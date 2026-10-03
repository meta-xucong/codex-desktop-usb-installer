//! Metadata-only diagnostics: never accept request bodies, auth maps or errors.
use serde::Serialize;
use sha2::{Digest, Sha256};

pub fn digest_hex(bytes: &[u8]) -> String {
    // sha2 0.11 returns an array without LowerHex. Match the existing USB code.
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub fn safe_identifier(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    !value.is_empty()
        && value.len() <= 256
        && value
            .chars()
            .all(|c| c.is_alphanumeric() || "-_.:/".contains(c))
        && ![
            "sk-",
            "bearer",
            "authorization",
            "password",
            "api_key",
            "apikey",
        ]
        .iter()
        .any(|prefix| lower.starts_with(prefix))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeError {
    Network,
    Timeout,
    Cancelled,
    Unauthorized,
    Http,
    InvalidResponse,
    InvalidCall,
    InvalidStream,
    TooLarge,
    BudgetExhausted,
    ContextChanged,
}
impl ProbeError {
    pub fn code(self) -> &'static str {
        match self {
            Self::Network => "upstream-unavailable",
            Self::Timeout => "probe-timeout",
            Self::Cancelled => "probe-cancelled",
            Self::Unauthorized => "upstream-auth",
            Self::Http => "upstream-http",
            Self::InvalidResponse => "upstream-response",
            Self::InvalidCall => "tool-correlation-invalid",
            Self::InvalidStream => "stream-invalid",
            Self::TooLarge => "response-too-large",
            Self::BudgetExhausted => "probe-budget-exhausted",
            Self::ContextChanged => "binding-changed",
        }
    }
    pub fn retryable(self) -> bool {
        matches!(
            self,
            Self::Network
                | Self::Timeout
                | Self::Cancelled
                | Self::Unauthorized
                | Self::Http
                | Self::BudgetExhausted
                | Self::ContextChanged
        )
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ProtocolDiagnostic {
    pub request_id: Option<String>,
    pub response_id: Option<String>,
    pub call_id: Option<String>,
    pub event_sequence: Option<u64>,
    pub model_id: Option<String>,
    pub adapter_version: Option<String>,
    pub run_id: Option<String>,
    pub stage: &'static str,
    pub error_code: Option<&'static str>,
}
fn opaque(value: Option<&str>) -> Option<String> {
    value
        .filter(|s| !s.is_empty() && s.len() <= 512)
        .map(|value| format!("sha256:{}", digest_hex(value.as_bytes())))
}
fn public_id(value: Option<&str>) -> Option<String> {
    value.filter(|s| safe_identifier(s)).map(str::to_owned)
}
impl ProtocolDiagnostic {
    pub fn new(stage: &'static str, model: Option<&str>) -> Self {
        Self {
            request_id: None,
            response_id: None,
            call_id: None,
            event_sequence: None,
            model_id: public_id(model),
            adapter_version: None,
            run_id: None,
            stage,
            error_code: None,
        }
    }
    pub fn request(&mut self, id: Option<&str>) {
        self.request_id = opaque(id);
    }
    pub fn response(&mut self, id: Option<&str>) {
        self.response_id = opaque(id);
    }
    pub fn call(&mut self, id: Option<&str>) {
        self.call_id = opaque(id);
    }
    pub fn adapter(&mut self, version: Option<&str>) {
        self.adapter_version = public_id(version);
    }
    pub fn run(&mut self, id: Option<&str>) {
        self.run_id = id.filter(|value| safe_identifier(value)).map(str::to_owned);
    }
    pub fn fail(&mut self, error: ProbeError) {
        self.error_code = Some(error.code());
    }
    pub fn record(&self) {
        if let Ok(metadata) = serde_json::to_string(self) {
            log::info!("codex_gateway_probe {}", metadata);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn secret_shaped_metadata_is_never_logged_verbatim() {
        let mut record = ProtocolDiagnostic::new("text", Some("Authorization: Bearer fixture-key"));
        record.request(Some("private-fixture-prompt"));
        record.call(Some("sk-fixture-key"));
        record.adapter(Some("password=fixture"));
        record.run(Some("probe-fixture-1"));
        record.fail(ProbeError::InvalidCall);
        let output = serde_json::to_string(&record).unwrap();
        for secret in [
            "private-fixture-prompt",
            "sk-fixture-key",
            "Bearer",
            "password=fixture",
        ] {
            assert!(!output.contains(secret));
        }
        assert_eq!(record.model_id, None);
        assert_eq!(record.adapter_version, None);
        assert_eq!(record.run_id.as_deref(), Some("probe-fixture-1"));
    }
    #[test]
    fn public_model_ids_remain_useful_and_correlation_is_stable() {
        let mut record = ProtocolDiagnostic::new("tools", Some("vendor/arbitrary-model"));
        record.request(Some("req-fixture"));
        record.adapter(Some("adapter-1.2.3"));
        assert_eq!(record.model_id.as_deref(), Some("vendor/arbitrary-model"));
        assert_eq!(record.request_id, opaque(Some("req-fixture")));
        assert_eq!(
            digest_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert!(!safe_identifier("bad\nforged-log"));
    }
}
