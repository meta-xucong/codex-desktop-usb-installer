//! Bounded synthetic Responses probes. No model-returned command is executed.
use crate::app::model_capability::{CapabilityState, EvidenceScope, ModelCapabilityReport};
use crate::app::model_diagnostics::{safe_identifier, ProbeError, ProtocolDiagnostic};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc,
};
use std::time::{Duration, Instant};

pub const BODY_LIMIT: usize = 1024 * 1024;
pub const REFRESH_BUDGET: Duration = Duration::from_secs(180);
/// Upper bound for lightweight gateway requests such as `/health`, `/models`,
/// and the compatibility directory.  This is deliberately shorter than the
/// full candidate budget while allowing a slow upstream route to finish.
pub const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
/// Responses generation can include real model reasoning and tool calls.  Keep
/// a separate per-request bound so a slow tool-capable model does not get cut
/// off by the lightweight request timeout.
pub const GENERATION_REQUEST_TIMEOUT: Duration = Duration::from_secs(90);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProbeTimeouts {
    pub request: Duration,
    pub generation: Duration,
}

impl ProbeTimeouts {
    pub const fn new(request: Duration, generation: Duration) -> Self {
        Self {
            request,
            generation,
        }
    }
}

impl Default for ProbeTimeouts {
    fn default() -> Self {
        Self::new(DEFAULT_REQUEST_TIMEOUT, GENERATION_REQUEST_TIMEOUT)
    }
}

pub struct ProbeClient<'a> {
    client: reqwest::Client,
    base: url::Url,
    api_key: &'a str,
    cancelled: Arc<AtomicBool>,
    deadline: Instant,
    run_id: Option<String>,
    requests_started: AtomicUsize,
    timeouts: ProbeTimeouts,
}
impl<'a> ProbeClient<'a> {
    pub fn new(
        base: &str,
        api_key: &'a str,
        cancelled: Arc<AtomicBool>,
        deadline: Instant,
    ) -> Result<Self, ProbeError> {
        Self::with_timeouts(base, api_key, cancelled, deadline, ProbeTimeouts::default())
    }

    pub fn with_timeouts(
        base: &str,
        api_key: &'a str,
        cancelled: Arc<AtomicBool>,
        deadline: Instant,
        timeouts: ProbeTimeouts,
    ) -> Result<Self, ProbeError> {
        let base = url::Url::parse(base).map_err(|_| ProbeError::InvalidResponse)?;
        let loopback_fixture = cfg!(test)
            && base.scheme() == "http"
            && matches!(
                base.host_str(),
                Some("127.0.0.1") | Some("localhost") | Some("[::1]")
            );
        if (base.scheme() != "https" && !loopback_fixture)
            || !base.username().is_empty()
            || base.password().is_some()
            || base.query().is_some()
            || base.fragment().is_some()
            || !base.path().ends_with('/')
        {
            return Err(ProbeError::InvalidResponse);
        }
        let _ = rustls::crypto::ring::default_provider().install_default();
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(8))
            .timeout(timeouts.request)
            .redirect(reqwest::redirect::Policy::none())
            .user_agent("Codex-App-Manager/local-capability-probe")
            .build()
            .map_err(|_| ProbeError::Network)?;
        Ok(Self {
            client,
            base,
            api_key,
            cancelled,
            deadline,
            run_id: None,
            requests_started: AtomicUsize::new(0),
            timeouts,
        })
    }
    pub fn with_run_id(mut self, run_id: &str) -> Self {
        self.run_id = safe_identifier(run_id).then(|| run_id.to_owned());
        self
    }
    /// Number of HTTP requests whose send operation has started for this run.
    pub fn requests_started(&self) -> usize {
        self.requests_started.load(Ordering::Acquire)
    }
    fn diagnostic(&self, stage: &'static str, model: Option<&str>) -> ProtocolDiagnostic {
        let mut diagnostic = ProtocolDiagnostic::new(stage, model);
        diagnostic.run(self.run_id.as_deref());
        diagnostic
    }
    pub fn check(&self) -> Result<(), ProbeError> {
        if self.cancelled.load(Ordering::Acquire) {
            return Err(ProbeError::Cancelled);
        }
        if Instant::now() >= self.deadline {
            return Err(ProbeError::BudgetExhausted);
        }
        Ok(())
    }
    fn endpoint(&self, path: &str) -> Result<url::Url, ProbeError> {
        self.base
            .join(path)
            .map_err(|_| ProbeError::InvalidResponse)
    }
    async fn receive(
        &self,
        request: reqwest::RequestBuilder,
        diag: &mut ProtocolDiagnostic,
        expected_content_type: Option<&str>,
    ) -> Result<Vec<u8>, ProbeError> {
        self.check()?;
        let result = {
            let mut operation = Box::pin(async {
                self.requests_started.fetch_add(1, Ordering::AcqRel);
                let mut response = request.send().await.map_err(|e| {
                    if e.is_timeout() {
                        ProbeError::Timeout
                    } else {
                        ProbeError::Network
                    }
                })?;
                diag.request(
                    response
                        .headers()
                        .get("x-request-id")
                        .and_then(|v| v.to_str().ok()),
                );
                let status = response.status().as_u16();
                if matches!(status, 401 | 403) {
                    return Err(ProbeError::Unauthorized);
                }
                if !response.status().is_success() {
                    return Err(ProbeError::Http);
                }
                if let Some(expected) = expected_content_type {
                    let actual = response
                        .headers()
                        .get("content-type")
                        .and_then(|v| v.to_str().ok())
                        .and_then(|v| v.split(';').next())
                        .map(str::trim);
                    if !actual.is_some_and(|value| value.eq_ignore_ascii_case(expected)) {
                        return Err(ProbeError::InvalidStream);
                    }
                }
                if response
                    .content_length()
                    .is_some_and(|n| n > BODY_LIMIT as u64)
                {
                    return Err(ProbeError::TooLarge);
                }
                let mut bytes = Vec::new();
                while let Some(chunk) = response.chunk().await.map_err(|e| {
                    if e.is_timeout() {
                        ProbeError::Timeout
                    } else {
                        ProbeError::Network
                    }
                })? {
                    self.check()?;
                    if bytes.len().saturating_add(chunk.len()) > BODY_LIMIT {
                        return Err(ProbeError::TooLarge);
                    }
                    bytes.extend_from_slice(&chunk);
                }
                self.check()?;
                Ok(bytes)
            });
            loop {
                if let Err(error) = self.check() {
                    break Err(error);
                }
                match tokio::time::timeout(Duration::from_millis(100), &mut operation).await {
                    Ok(result) => break result,
                    Err(_) => continue,
                }
            }
        };
        if let Err(error) = &result {
            diag.fail(*error);
        }
        if result.is_err() {
            diag.record();
        }
        result
    }
    pub async fn health(&self) -> Result<(), ProbeError> {
        let mut diag = self.diagnostic("health", None);
        // Health checks do not need the device credential.
        self.receive(
            self.client
                .get(self.endpoint("/health")?)
                .timeout(self.timeouts.request),
            &mut diag,
            None,
        )
        .await?;
        diag.record();
        Ok(())
    }
    pub async fn models(&self) -> Result<Value, ProbeError> {
        let mut diag = self.diagnostic("models", None);
        let bytes = self
            .receive(
                self.client
                    .get(self.endpoint("models")?)
                    .bearer_auth(self.api_key)
                    .timeout(self.timeouts.request),
                &mut diag,
                None,
            )
            .await?;
        let parsed = serde_json::from_slice(&bytes).map_err(|_| ProbeError::InvalidResponse);
        if let Err(error) = parsed.as_ref() {
            diag.fail(*error);
        }
        diag.record();
        parsed
    }
    pub async fn codex_compatibility(&self) -> Result<Value, ProbeError> {
        let mut diag = self.diagnostic("codex-compatibility", None);
        let bytes = self
            .receive(
                self.client
                    .get(self.endpoint("codex/compatibility")?)
                    .bearer_auth(self.api_key)
                    .timeout(self.timeouts.request),
                &mut diag,
                None,
            )
            .await?;
        let parsed = serde_json::from_slice(&bytes).map_err(|_| ProbeError::InvalidResponse);
        if let Err(error) = parsed.as_ref() {
            diag.fail(*error);
        }
        diag.record();
        parsed
    }
    async fn response(
        &self,
        model: &str,
        body: Value,
        stage: &'static str,
    ) -> Result<Value, ProbeError> {
        let mut diag = self.diagnostic(stage, Some(model));
        let bytes = self
            .receive(
                self.client
                    .post(self.endpoint("responses")?)
                    .bearer_auth(self.api_key)
                    .json(&body)
                    .timeout(self.timeouts.generation),
                &mut diag,
                None,
            )
            .await?;
        let result = serde_json::from_slice::<Value>(&bytes)
            .map_err(|_| ProbeError::InvalidResponse)
            .and_then(|value| {
                // Preserve only the opaque correlation ID when validation fails.
                diag.response(value.get("id").and_then(Value::as_str));
                validate_response(&value, model)?;
                Ok(value)
            });
        match &result {
            Ok(value) => diag.response(value.get("id").and_then(Value::as_str)),
            Err(error) => diag.fail(*error),
        }
        diag.record();
        result
    }
    async fn text(
        &self,
        model: &str,
        token: &str,
        previous: Option<&str>,
    ) -> Result<Value, ProbeError> {
        let input = if previous.is_some() {
            "Repeat the original token exactly, with no other text.".into()
        } else {
            format!("Remember this token for later turns. Reply exactly {token}")
        };
        let mut body = json!({"model":model,"input":input,"stream":false,"store":true});
        if let Some(id) = previous {
            body["previous_response_id"] = json!(id);
        }
        let response = self.response(model, body, "text").await?;
        if let Err(error) = require_text(&response, token) {
            let mut diag = self.diagnostic("text", Some(model));
            diag.response(response.get("id").and_then(Value::as_str));
            diag.fail(error);
            diag.record();
            return Err(error);
        }
        Ok(response)
    }
    /// Send one bounded text challenge and require an exact echo. This is the
    /// only generation request used by the selective catalog quick test.
    pub async fn quick_text(&self, model: &str, token: &str) -> Result<(), ProbeError> {
        self.text(model, token, None).await.map(|_| ())
    }
    async fn tool_start(
        &self,
        model: &str,
        token: &str,
        multiple: bool,
        error: bool,
    ) -> Result<(Value, Vec<ToolCall>), ProbeError> {
        let tokens = if multiple {
            vec![format!("{token}-a"), format!("{token}-b")]
        } else {
            vec![token.into()]
        };
        let prefix = if error { "RECOVERED|" } else { "" };
        let body = json!({"model":model,"stream":false,"store":true,
            "input":format!("Call codex_usb_echo exactly once for each token: {}. After receiving all function outputs, sort them by token and reply with their challenge values joined by |, prefixed with {}. If an output contains fixture_error, still read its challenge. Do not answer with a token or invent a challenge.",tokens.join(","), if prefix.is_empty() { "nothing" } else { prefix }),
            "tools":[echo_tool()],"tool_choice":"required","parallel_tool_calls":multiple});
        let response = self.response(model, body, "tool-call").await?;
        let calls = validate_calls(&response, &tokens).inspect_err(|&error| {
            let mut diag = self.diagnostic("tool-call", Some(model));
            diag.response(response.get("id").and_then(Value::as_str));
            diag.fail(error);
            diag.record();
        })?;
        for call in &calls {
            let mut diag = self.diagnostic("tool-call", Some(model));
            diag.response(response.get("id").and_then(Value::as_str));
            diag.call(Some(&call.call_id));
            diag.record();
        }
        Ok((response, calls))
    }
    async fn tool_finish(
        &self,
        model: &str,
        first: &Value,
        calls: &[ToolCall],
        error: bool,
    ) -> Result<(), ProbeError> {
        // The answer is created only AFTER the model's tool calls arrive.
        // It occurs exclusively in function_call_output, never in the prompt.
        let (input, expected) = challenge_outputs(calls, error);
        let response = self
            .response(
                model,
                json!({"model":model,"stream":false,"store":true,
            "previous_response_id":response_id(first)?,"input":input}),
                "tool-result",
            )
            .await?;
        let outcome = if response_id(first)? == response_id(&response)? {
            Err(ProbeError::InvalidResponse)
        } else {
            require_text(&response, &expected)
        };
        if let Err(error) = outcome {
            for call in calls {
                let mut diag = self.diagnostic("tool-result", Some(model));
                diag.response(response.get("id").and_then(Value::as_str));
                diag.call(Some(&call.call_id));
                diag.fail(error);
                diag.record();
            }
        }
        outcome
    }
    async fn stream(&self, model: &str, token: &str) -> Result<(), ProbeError> {
        let body = json!({"model":model,"stream":true,"store":false,
            "input":format!("Call codex_usb_echo exactly once with token {token}."),
            "tools":[echo_tool()],"tool_choice":{"type":"function","name":"codex_usb_echo"}});
        let mut diag = self.diagnostic("stream", Some(model));
        let bytes = self
            .receive(
                self.client
                    .post(self.endpoint("responses")?)
                    .bearer_auth(self.api_key)
                    .json(&body)
                    .timeout(self.timeouts.generation),
                &mut diag,
                Some("text/event-stream"),
            )
            .await?;
        let result = validate_stream(&bytes, model, token, &mut diag);
        if let Err(error) = result {
            diag.fail(error);
        }
        diag.record();
        result
    }
    pub async fn probe_model(&self, model: &mut ModelCapabilityReport, now: u64) {
        model.checks = Default::default();
        model.error_code = None;
        model.checked_at = Some(now);
        model.evidence_scope = if cfg!(test) {
            EvidenceScope::Mock
        } else {
            EvidenceScope::Gateway
        };
        let token = format!("usb{}", uuid::Uuid::new_v4().simple());
        let id = model.model_id.clone();
        let first = match self.text(&id, &token, None).await {
            Ok(value) => {
                model.checks.text = CapabilityState::Verified;
                value
            }
            Err(error) => {
                finish_error(model, "text", error, self.run_id.as_deref());
                return;
            }
        };
        let (single, calls) = match self.tool_start(&id, &token, false, false).await {
            Ok(value) => {
                model.checks.single_tool = CapabilityState::Verified;
                value
            }
            Err(error) => {
                finish_error(model, "single_tool", error, self.run_id.as_deref());
                return;
            }
        };
        if let Err(e) = self.tool_finish(&id, &single, &calls, false).await {
            finish_error(model, "tool_result", e, self.run_id.as_deref());
            return;
        }
        model.checks.tool_result = CapabilityState::Verified;
        match self.tool_start(&id, &token, true, false).await {
            Ok((value, calls)) => match self.tool_finish(&id, &value, &calls, false).await {
                Ok(()) => model.checks.multiple_tools = CapabilityState::Verified,
                Err(e) => {
                    finish_error(model, "multiple_tools", e, self.run_id.as_deref());
                    return;
                }
            },
            Err(e) => {
                finish_error(model, "multiple_tools", e, self.run_id.as_deref());
                return;
            }
        }
        match self.tool_start(&id, &token, false, true).await {
            Ok((value, calls)) => match self.tool_finish(&id, &value, &calls, true).await {
                Ok(()) => model.checks.tool_error = CapabilityState::Verified,
                Err(e) => {
                    finish_error(model, "tool_error", e, self.run_id.as_deref());
                    return;
                }
            },
            Err(e) => {
                finish_error(model, "tool_error", e, self.run_id.as_deref());
                return;
            }
        }
        if let Err(e) = self.stream(&id, &token).await {
            finish_error(model, "stream", e, self.run_id.as_deref());
            return;
        }
        model.checks.stream = CapabilityState::Verified;
        let mut previous = first;
        let mut ids = BTreeSet::new();
        if let Ok(id) = response_id(&previous) {
            ids.insert(id.to_string());
        }
        for _ in 1..5 {
            let prior = match response_id(&previous) {
                Ok(v) => v,
                Err(e) => {
                    finish_error(model, "multi_turn", e, self.run_id.as_deref());
                    return;
                }
            };
            match self.text(&id, &token, Some(prior)).await {
                Ok(value) => {
                    if !ids.insert(response_id(&value).unwrap_or_default().into()) {
                        finish_error(
                            model,
                            "multi_turn",
                            ProbeError::InvalidResponse,
                            self.run_id.as_deref(),
                        );
                        return;
                    }
                    previous = value;
                }
                Err(e) => {
                    finish_error(model, "multi_turn", e, self.run_id.as_deref());
                    return;
                }
            }
        }
        model.checks.multi_turn = CapabilityState::Verified;
        // Shell, Skills and native-catalog parsing are not inferred here.
    }
}

fn finish_error(
    model: &mut ModelCapabilityReport,
    stage: &'static str,
    error: ProbeError,
    run_id: Option<&str>,
) {
    let state = if error.retryable() {
        CapabilityState::Unknown
    } else {
        CapabilityState::Quarantined
    };
    match stage {
        "text" => model.checks.text = state,
        "single_tool" => model.checks.single_tool = state,
        "tool_result" => model.checks.tool_result = state,
        "multiple_tools" => model.checks.multiple_tools = state,
        "tool_error" => model.checks.tool_error = state,
        "stream" => model.checks.stream = state,
        "multi_turn" => model.checks.multi_turn = state,
        _ => {}
    }
    model.error_code = Some(error.code().into());
    let mut diag = ProtocolDiagnostic::new(stage, Some(&model.model_id));
    diag.run(run_id);
    diag.fail(error);
    diag.record();
}
fn echo_tool() -> Value {
    json!({"type":"function","name":"codex_usb_echo","description":"Returns the token and a newly generated challenge; no OS execution.",
        "strict":true,"parameters":{"type":"object","properties":{"token":{"type":"string"}},
        "required":["token"],"additionalProperties":false}})
}
fn response_id(value: &Value) -> Result<&str, ProbeError> {
    value
        .get("id")
        .and_then(Value::as_str)
        .filter(|v| safe_identifier(v))
        .ok_or(ProbeError::InvalidResponse)
}
fn validate_response(value: &Value, model: &str) -> Result<(), ProbeError> {
    response_id(value)?;
    if value.get("status").and_then(Value::as_str) != Some("completed")
        || value.get("model").and_then(Value::as_str) != Some(model)
        || value.get("error").is_some_and(|v| !v.is_null())
        || !value.get("output").is_some_and(Value::is_array)
    {
        return Err(ProbeError::InvalidResponse);
    }
    Ok(())
}
fn require_text(value: &Value, expected: &str) -> Result<(), ProbeError> {
    let mut texts = Vec::new();
    for item in value
        .get("output")
        .and_then(Value::as_array)
        .ok_or(ProbeError::InvalidResponse)?
    {
        match item.get("type").and_then(Value::as_str) {
            Some("reasoning") => continue,
            Some("message") => {
                if item.get("role").and_then(Value::as_str) != Some("assistant") {
                    return Err(ProbeError::InvalidResponse);
                }
                for part in item
                    .get("content")
                    .and_then(Value::as_array)
                    .ok_or(ProbeError::InvalidResponse)?
                {
                    if part.get("type").and_then(Value::as_str) != Some("output_text") {
                        return Err(ProbeError::InvalidResponse);
                    }
                    texts.push(
                        part.get("text")
                            .and_then(Value::as_str)
                            .ok_or(ProbeError::InvalidResponse)?,
                    );
                }
            }
            _ => return Err(ProbeError::InvalidResponse),
        }
    }
    if texts.join("").trim() != expected {
        return Err(ProbeError::InvalidResponse);
    }
    Ok(())
}
#[derive(Debug, Clone)]
struct ToolCall {
    call_id: String,
    item_id: String,
    token: String,
}
fn challenge_outputs(calls: &[ToolCall], error: bool) -> (Vec<Value>, String) {
    let mut ordered: Vec<&ToolCall> = calls.iter().collect();
    ordered.sort_by(|a, b| a.token.cmp(&b.token));
    let mut answers = Vec::with_capacity(ordered.len());
    let outputs = ordered
        .into_iter()
        .map(|call| {
            let challenge = uuid::Uuid::new_v4().simple().to_string();
            answers.push(challenge.clone());
            let mut output = json!({"token":call.token,"challenge":challenge});
            if error {
                output["error"] = json!("fixture_error");
            }
            json!({"type":"function_call_output","call_id":call.call_id,
            "output":output.to_string()})
        })
        .collect();
    let expected = format!(
        "{}{}",
        if error { "RECOVERED|" } else { "" },
        answers.join("|")
    );
    (outputs, expected)
}

fn validate_calls(value: &Value, tokens: &[String]) -> Result<Vec<ToolCall>, ProbeError> {
    let mut calls = Vec::new();
    let mut ids = BTreeSet::new();
    let mut item_ids = BTreeSet::new();
    let mut remaining: BTreeSet<&str> = tokens.iter().map(String::as_str).collect();
    for item in value
        .get("output")
        .and_then(Value::as_array)
        .ok_or(ProbeError::InvalidCall)?
    {
        let kind = item.get("type").and_then(Value::as_str);
        if matches!(kind, Some("message") | Some("reasoning")) {
            continue;
        }
        if kind != Some("function_call")
            || item.get("name").and_then(Value::as_str) != Some("codex_usb_echo")
            || item
                .get("status")
                .is_some_and(|v| v.as_str() != Some("completed"))
        {
            return Err(ProbeError::InvalidCall);
        }
        let id = item
            .get("call_id")
            .and_then(Value::as_str)
            .filter(|v| safe_identifier(v))
            .ok_or(ProbeError::InvalidCall)?;
        let item_id = item
            .get("id")
            .and_then(Value::as_str)
            .filter(|v| safe_identifier(v))
            .ok_or(ProbeError::InvalidCall)?;
        let args: Value = serde_json::from_str(
            item.get("arguments")
                .and_then(Value::as_str)
                .ok_or(ProbeError::InvalidCall)?,
        )
        .map_err(|_| ProbeError::InvalidCall)?;
        let token = args
            .get("token")
            .and_then(Value::as_str)
            .ok_or(ProbeError::InvalidCall)?;
        if args.as_object().map(|o| o.len()) != Some(1)
            || !remaining.remove(token)
            || !ids.insert(id)
            || !item_ids.insert(item_id)
        {
            return Err(ProbeError::InvalidCall);
        }
        calls.push(ToolCall {
            call_id: id.into(),
            item_id: item_id.into(),
            token: token.into(),
        });
    }
    if !remaining.is_empty() || calls.len() != tokens.len() {
        return Err(ProbeError::InvalidCall);
    }
    Ok(calls)
}

#[derive(Default)]
struct StreamCall {
    call_id: String,
    arguments: String,
    output_index: u64,
    arguments_done: bool,
    item_done: bool,
}
fn validate_stream(
    bytes: &[u8],
    model: &str,
    token: &str,
    diag: &mut ProtocolDiagnostic,
) -> Result<(), ProbeError> {
    let raw = std::str::from_utf8(bytes).map_err(|_| ProbeError::InvalidStream)?;
    let text = raw.replace("\r\n", "\n").replace('\r', "\n");
    if !text.ends_with("\n\n") {
        return Err(ProbeError::InvalidStream);
    }
    let mut phase = 0_u8;
    let mut response = String::new();
    let mut last_sequence = None;
    let mut calls: BTreeMap<String, StreamCall> = BTreeMap::new();
    let mut seen_call_ids = BTreeSet::new();
    let mut seen_output_indices = BTreeSet::new();
    let mut seen_item_ids = BTreeSet::new();
    let mut transport_done = false;
    for frame in text.split("\n\n") {
        let mut name = None;
        let mut data = Vec::new();
        for line in frame.lines() {
            if line.starts_with(':') {
                continue;
            }
            if let Some(value) = line.strip_prefix("event:") {
                name = Some(value.trim());
            }
            if let Some(value) = line.strip_prefix("data:") {
                data.push(value.strip_prefix(' ').unwrap_or(value));
            }
        }
        if data.is_empty() {
            continue;
        }
        if transport_done {
            return Err(ProbeError::InvalidStream);
        }
        let data = data.join("\n");
        if data == "[DONE]" {
            if phase != 3 {
                return Err(ProbeError::InvalidStream);
            }
            transport_done = true;
            continue;
        }
        if phase == 3 {
            return Err(ProbeError::InvalidStream);
        }
        let event: Value = serde_json::from_str(&data).map_err(|_| ProbeError::InvalidStream)?;
        let kind = event
            .get("type")
            .and_then(Value::as_str)
            .ok_or(ProbeError::InvalidStream)?;
        if name.is_some_and(|name| name != kind) {
            return Err(ProbeError::InvalidStream);
        }
        let sequence = event
            .get("sequence_number")
            .and_then(Value::as_u64)
            .ok_or(ProbeError::InvalidStream)?;
        if last_sequence.is_some_and(|last| sequence <= last) {
            return Err(ProbeError::InvalidStream);
        }
        last_sequence = Some(sequence);
        diag.event_sequence = Some(sequence);
        if !response.is_empty()
            && event
                .get("response_id")
                .is_some_and(|v| v.as_str() != Some(response.as_str()))
        {
            return Err(ProbeError::InvalidStream);
        }
        match kind {
            "response.created" => {
                if phase != 0 {
                    return Err(ProbeError::InvalidStream);
                }
                response = response_id(&event["response"])?.to_owned();
                diag.response(Some(&response));
                phase = 1;
            }
            "response.in_progress" => {
                if phase != 1 || response_id(&event["response"])? != response {
                    return Err(ProbeError::InvalidStream);
                }
                phase = 2;
            }
            "response.output_item.added" => {
                if phase != 2 {
                    return Err(ProbeError::InvalidStream);
                }
                let item = &event["item"];
                let item_id = response_id(item)?.to_owned();
                let output_index = event
                    .get("output_index")
                    .and_then(Value::as_u64)
                    .ok_or(ProbeError::InvalidStream)?;
                if !seen_item_ids.insert(item_id.clone())
                    || !seen_output_indices.insert(output_index)
                    || !matches!(
                        item.get("type").and_then(Value::as_str),
                        Some("function_call") | Some("message") | Some("reasoning")
                    )
                {
                    return Err(ProbeError::InvalidStream);
                }
                if item.get("type").and_then(Value::as_str) == Some("function_call") {
                    let call = item
                        .get("call_id")
                        .and_then(Value::as_str)
                        .filter(|v| safe_identifier(v))
                        .ok_or(ProbeError::InvalidStream)?;
                    if item.get("name").and_then(Value::as_str) != Some("codex_usb_echo")
                        || calls.contains_key(&item_id)
                        || !seen_call_ids.insert(call.to_owned())
                        || item.get("arguments").and_then(Value::as_str) != Some("")
                        || item
                            .get("status")
                            .is_some_and(|v| v.as_str() != Some("in_progress"))
                    {
                        return Err(ProbeError::InvalidStream);
                    }
                    calls.insert(
                        item_id,
                        StreamCall {
                            call_id: call.into(),
                            output_index,
                            ..Default::default()
                        },
                    );
                    diag.call(Some(call));
                }
            }
            "response.function_call_arguments.delta" | "response.function_call_arguments.done" => {
                if phase != 2 {
                    return Err(ProbeError::InvalidStream);
                }
                let item = event
                    .get("item_id")
                    .and_then(Value::as_str)
                    .ok_or(ProbeError::InvalidStream)?;
                let call = calls.get_mut(item).ok_or(ProbeError::InvalidStream)?;
                if call.arguments_done
                    || call.item_done
                    || event.get("output_index").and_then(Value::as_u64) != Some(call.output_index)
                    || event
                        .get("call_id")
                        .is_some_and(|v| v.as_str() != Some(call.call_id.as_str()))
                    || event
                        .get("name")
                        .is_some_and(|v| v.as_str() != Some("codex_usb_echo"))
                {
                    return Err(ProbeError::InvalidStream);
                }
                if kind.ends_with(".delta") {
                    call.arguments.push_str(
                        event
                            .get("delta")
                            .and_then(Value::as_str)
                            .ok_or(ProbeError::InvalidStream)?,
                    );
                } else {
                    if event.get("arguments").and_then(Value::as_str)
                        != Some(call.arguments.as_str())
                    {
                        return Err(ProbeError::InvalidStream);
                    }
                    call.arguments_done = true;
                }
            }
            "response.output_item.done" => {
                if phase != 2 {
                    return Err(ProbeError::InvalidStream);
                }
                let item = &event["item"];
                if item.get("type").and_then(Value::as_str) == Some("function_call") {
                    let call = calls
                        .get_mut(response_id(item)?)
                        .ok_or(ProbeError::InvalidStream)?;
                    if !call.arguments_done
                        || call.item_done
                        || item
                            .get("status")
                            .is_some_and(|v| v.as_str() != Some("completed"))
                        || item.get("call_id").and_then(Value::as_str)
                            != Some(call.call_id.as_str())
                        || item.get("name").and_then(Value::as_str) != Some("codex_usb_echo")
                        || item.get("arguments").and_then(Value::as_str)
                            != Some(call.arguments.as_str())
                        || event.get("output_index").and_then(Value::as_u64)
                            != Some(call.output_index)
                    {
                        return Err(ProbeError::InvalidStream);
                    }
                    call.item_done = true;
                }
            }
            "response.completed" => {
                let value = &event["response"];
                if phase != 2
                    || calls.len() != 1
                    || calls.values().any(|c| !c.item_done)
                    || response_id(value)? != response
                {
                    return Err(ProbeError::InvalidStream);
                }
                validate_response(value, model)?;
                let finished = validate_calls(value, &[token.into()])?;
                for call in finished {
                    let streamed = calls.get(&call.item_id).ok_or(ProbeError::InvalidStream)?;
                    let index = usize::try_from(streamed.output_index)
                        .map_err(|_| ProbeError::InvalidStream)?;
                    let final_item = value
                        .get("output")
                        .and_then(Value::as_array)
                        .and_then(|items| items.get(index))
                        .ok_or(ProbeError::InvalidStream)?;
                    if streamed.call_id != call.call_id
                        || final_item.get("id").and_then(Value::as_str)
                            != Some(call.item_id.as_str())
                        || final_item.get("arguments").and_then(Value::as_str)
                            != Some(streamed.arguments.as_str())
                    {
                        return Err(ProbeError::InvalidStream);
                    }
                }
                phase = 3;
            }
            // Extra non-tool text/reasoning events do not prove any capability.
            k if phase == 2
                && (k.starts_with("response.reasoning_")
                    || k.starts_with("response.output_text.")
                    || k.starts_with("response.content_part.")) => {}
            _ => return Err(ProbeError::InvalidStream),
        }
        diag.record();
    }
    if phase != 3 {
        return Err(ProbeError::InvalidStream);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn tool_response() -> Value {
        json!({"id":"resp-fixture","model":"fixture-model","status":"completed","output":[
            {"type":"function_call","id":"item-fixture","call_id":"call-fixture",
             "name":"codex_usb_echo","arguments":"{\"token\":\"abc\"}"}]})
    }
    fn stream_fixture() -> Vec<Value> {
        let response = tool_response();
        let item = &response["output"][0];
        vec![
            json!({"type":"response.created","sequence_number":0,"response":{"id":"resp-fixture"}}),
            json!({"type":"response.in_progress","sequence_number":1,"response":{"id":"resp-fixture"}}),
            json!({"type":"response.output_item.added","sequence_number":2,"output_index":0,
                "item":{"type":"function_call","id":"item-fixture","call_id":"call-fixture","name":"codex_usb_echo","arguments":""}}),
            json!({"type":"response.function_call_arguments.delta","sequence_number":3,"item_id":"item-fixture","output_index":0,"delta":"{\"token\":\"abc\"}"}),
            json!({"type":"response.function_call_arguments.done","sequence_number":4,"item_id":"item-fixture","output_index":0,"arguments":"{\"token\":\"abc\"}"}),
            json!({"type":"response.output_item.done","sequence_number":5,"output_index":0,"item":item}),
            json!({"type":"response.completed","sequence_number":6,"response":response}),
        ]
    }
    fn encoded(events: &[Value]) -> Vec<u8> {
        events
            .iter()
            .map(|v| {
                format!(
                    "event: {}\r\ndata: {}\r\n\r\n",
                    v["type"].as_str().unwrap(),
                    v
                )
            })
            .collect::<String>()
            .into_bytes()
    }
    #[test]
    fn validates_complete_tool_stream_and_rejects_reorder_truncation_and_call_mixup() {
        let events = stream_fixture();
        let mut diag = ProtocolDiagnostic::new("fixture", None);
        assert!(validate_stream(&encoded(&events), "fixture-model", "abc", &mut diag).is_ok());
        assert!(
            validate_stream(&encoded(&events[..6]), "fixture-model", "abc", &mut diag).is_err()
        );
        let mut bad = events.clone();
        bad.swap(3, 4);
        assert!(validate_stream(&encoded(&bad), "fixture-model", "abc", &mut diag).is_err());
        let mut bad = events.clone();
        bad[5]["item"]["call_id"] = json!("another-call");
        assert!(validate_stream(&encoded(&bad), "fixture-model", "abc", &mut diag).is_err());
        let mut bad = events;
        bad[4]["arguments"] = json!("{\"token\":\"different\"}");
        assert!(validate_stream(&encoded(&bad), "fixture-model", "abc", &mut diag).is_err());
    }
    #[test]
    fn final_stream_arguments_must_match_the_bytes_actually_streamed() {
        let mut events = stream_fixture();
        let different = r#"{"token":"not-the-requested-token"}"#;
        events[3]["delta"] = json!(different);
        events[4]["arguments"] = json!(different);
        events[5]["item"]["arguments"] = json!(different);
        // A correct final snapshot must not conceal different streamed input.
        let mut diag = ProtocolDiagnostic::new("fixture", None);
        assert!(validate_stream(&encoded(&events), "fixture-model", "abc", &mut diag).is_err());
    }

    #[test]
    fn stream_rejects_conflicting_indices_ids_and_failed_item_completion() {
        let valid = stream_fixture();
        let mut diag = ProtocolDiagnostic::new("fixture", None);
        let mut bad = valid.clone();
        for event in &mut bad[2..6] {
            event["output_index"] = json!(7);
        }
        assert!(validate_stream(&encoded(&bad), "fixture-model", "abc", &mut diag).is_err());
        let mut bad = valid.clone();
        bad[3]["response_id"] = json!("another-response");
        assert!(validate_stream(&encoded(&bad), "fixture-model", "abc", &mut diag).is_err());
        let mut bad = valid.clone();
        bad[4]["call_id"] = json!("another-call");
        assert!(validate_stream(&encoded(&bad), "fixture-model", "abc", &mut diag).is_err());
        let mut bad = valid.clone();
        bad[5]["item"]["status"] = json!("failed");
        assert!(validate_stream(&encoded(&bad), "fixture-model", "abc", &mut diag).is_err());
        for (index, item_id) in [(0, "another-item"), (1, "item-fixture")] {
            let mut bad = valid.clone();
            bad.insert(
                3,
                json!({"type":"response.output_item.added", "output_index":index,
                "item":{"type":"reasoning","id":item_id}}),
            );
            for (sequence, event) in bad.iter_mut().enumerate() {
                event["sequence_number"] = json!(sequence);
            }
            assert!(validate_stream(&encoded(&bad), "fixture-model", "abc", &mut diag).is_err());
        }
    }

    #[test]
    fn output_challenges_are_fresh_and_bound_to_the_original_calls() {
        let calls = validate_calls(&tool_response(), &["abc".into()]).unwrap();
        let (outputs, expected) = challenge_outputs(&calls, false);
        let data: Value = serde_json::from_str(outputs[0]["output"].as_str().unwrap()).unwrap();
        assert_eq!(outputs[0]["call_id"], "call-fixture");
        assert_eq!(data["token"], "abc");
        assert_eq!(data["challenge"].as_str(), Some(expected.as_str()));
        assert_ne!(expected, "abc");
        assert_ne!(expected, challenge_outputs(&calls, false).1);
        let (outputs, expected) = challenge_outputs(&calls, true);
        let data: Value = serde_json::from_str(outputs[0]["output"].as_str().unwrap()).unwrap();
        assert_eq!(data["error"], "fixture_error");
        assert_eq!(
            expected,
            format!("RECOVERED|{}", data["challenge"].as_str().unwrap())
        );
    }
    #[test]
    fn function_calls_require_exact_tokens_unique_ids_and_safe_schema() {
        assert!(validate_calls(&tool_response(), &["abc".into()]).is_ok());
        assert!(validate_calls(&tool_response(), &["different".into()]).is_err());
        let mut duplicate = tool_response();
        let first = duplicate["output"][0].clone();
        duplicate["output"].as_array_mut().unwrap().push(first);
        assert!(validate_calls(&duplicate, &["abc".into(), "def".into()]).is_err());
        let mut arbitrary = tool_response();
        arbitrary["output"][0]["name"] = json!("powershell");
        assert!(validate_calls(&arbitrary, &["abc".into()]).is_err());
    }
    #[test]
    fn completed_http_body_is_not_sufficient_for_text_verification() {
        assert!(require_text(&tool_response(), "abc").is_err());
        let mut wrong = tool_response();
        wrong["model"] = json!("another-model");
        assert!(validate_response(&wrong, "fixture-model").is_err());
    }
    #[test]
    fn cancel_is_checked_before_io() {
        let cancel = Arc::new(AtomicBool::new(true));
        let client = ProbeClient::new(
            "http://127.0.0.1:9/v1/",
            "fixture",
            cancel,
            Instant::now() + Duration::from_secs(1),
        )
        .unwrap();
        assert_eq!(client.check(), Err(ProbeError::Cancelled));
    }

    #[test]
    fn default_probe_timeouts_allow_slow_generation_without_shortening_lightweight_checks() {
        assert_eq!(
            ProbeTimeouts::default(),
            ProbeTimeouts::new(Duration::from_secs(60), Duration::from_secs(90))
        );
    }

    #[test]
    fn probe_client_accepts_a_custom_timeout_profile() {
        let timeouts = ProbeTimeouts::new(Duration::from_secs(7), Duration::from_secs(11));
        let client = ProbeClient::with_timeouts(
            "http://127.0.0.1:9/v1/",
            "fixture",
            Arc::new(AtomicBool::new(false)),
            Instant::now() + Duration::from_secs(1),
            timeouts,
        )
        .unwrap();
        assert_eq!(client.timeouts, timeouts);
    }

    #[test]
    fn compatibility_endpoint_preserves_provider_reverse_proxy_prefix() {
        let client = ProbeClient::new(
            "https://gateway.example.invalid/proxy/v1/",
            "fixture-only-key",
            Arc::new(AtomicBool::new(false)),
            Instant::now() + Duration::from_secs(5),
        )
        .unwrap();
        let endpoint = client.endpoint("codex/compatibility").unwrap();
        assert_eq!(endpoint.path(), "/proxy/v1/codex/compatibility");
    }
}

#[cfg(test)]
#[path = "model_probe_fixture.rs"]
mod http_fixture;
