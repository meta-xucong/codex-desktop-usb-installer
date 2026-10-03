//! Test-only loopback gateway; never compiled into the production manager.
use super::*;
use crate::app::model_capability::{CapabilityContext, ModelState};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::Mutex;
use std::thread;

#[derive(Default)]
struct Stored {
    token: String,
    goal: String,
    calls: Vec<(String, String)>,
    text_continuations: usize,
}
#[derive(Default)]
struct GatewayState {
    counter: u64,
    responses: BTreeMap<String, Stored>,
    continued_text: usize,
    continued_tools: usize,
}
struct Fixture {
    base: String,
    stop: Arc<AtomicBool>,
    state: Arc<Mutex<GatewayState>>,
    worker: Option<thread::JoinHandle<()>>,
}
impl Fixture {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("fixture loopback bind");
        listener.set_nonblocking(true).unwrap();
        let base = format!("http://{}/v1/", listener.local_addr().unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let stopping = stop.clone();
        let state = Arc::new(Mutex::new(GatewayState::default()));
        let shared = state.clone();
        let worker = thread::spawn(move || {
            while !stopping.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        // Keep the listener nonblocking, but use bounded blocking IO for each request.
                        stream
                            .set_nonblocking(false)
                            .expect("blocking fixture connection");
                        stream
                            .set_read_timeout(Some(Duration::from_secs(5)))
                            .unwrap();
                        stream
                            .set_write_timeout(Some(Duration::from_secs(5)))
                            .unwrap();
                        let (path, body, authorized) = read_request(&mut stream);
                        let (status, kind, payload) = if path == "/health" {
                            (200, "application/json", b"{}".to_vec())
                        } else if !authorized {
                            (401, "application/json", b"{}".to_vec())
                        } else if path == "/v1/codex/compatibility" {
                            (
                                200,
                                "application/json",
                                serde_json::to_vec(&json!({
                                    "schema_version": 1,
                                    "service_revision": "c4d48cf753c62f166d848c06c1fc25a23430316d",
                                    "profile_id": "codex-responses-tool-use-v1",
                                    "evidence_recorded_at": "2026-09-25T17:50:06Z",
                                    "evidence_time_source": "delivery_artifact_last_write_utc",
                                    "model_priority": ["fixture-good"],
                                    "default_model_id": "fixture-good",
                                    "models": [{"id": "fixture-good"}]
                                }))
                                .unwrap(),
                            )
                        } else if path == "/v1/models" {
                            (
                                200,
                                "application/json",
                                serde_json::to_vec(&json!({"data":[
                                {"id":"fixture-good","owned_by":"fixture"},
                                {"id":"fixture-bad-call","owned_by":"fixture"},
                                {"id":"fixture-bad-stream","owned_by":"fixture"}]}))
                                .unwrap(),
                            )
                        } else if path == "/v1/responses" {
                            let (kind, bytes) = handle_response(&body, &mut shared.lock().unwrap());
                            (200, kind, bytes)
                        } else {
                            (404, "application/json", b"{}".to_vec())
                        };
                        let header=format!("HTTP/1.1 {status} Fixture\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nx-request-id: fixture-request\r\nConnection: close\r\n\r\n",payload.len());
                        stream.write_all(header.as_bytes()).unwrap();
                        stream.write_all(&payload).unwrap();
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5))
                    }
                    Err(_) => break,
                }
            }
        });
        Self {
            base,
            stop,
            state,
            worker: Some(worker),
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
fn read_request(stream: &mut TcpStream) -> (String, Value, bool) {
    let mut bytes = Vec::new();
    let mut buf = [0_u8; 4096];
    let header_end = loop {
        let count = stream.read(&mut buf).expect("read fixture request");
        assert!(count > 0, "truncated fixture request");
        bytes.extend_from_slice(&buf[..count]);
        assert!(bytes.len() <= 65536, "fixture request size exceeded");
        if let Some(index) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            break index + 4;
        }
    };
    let header = std::str::from_utf8(&bytes[..header_end]).unwrap();
    let path = header
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .to_owned();
    let mut length = 0;
    let mut authorized = false;
    for line in header.lines().skip(1) {
        if let Some((name, value)) = line.split_once(':') {
            if name.eq_ignore_ascii_case("content-length") {
                length = value.trim().parse::<usize>().unwrap();
            }
            if name.eq_ignore_ascii_case("authorization") {
                authorized = value.trim() == "Bearer fixture-only-key";
            }
        }
    }
    assert!(length <= 65536, "fixture body exceeds limit");
    while bytes.len() < header_end + length {
        let count = stream.read(&mut buf).unwrap();
        assert!(count > 0, "truncated fixture body");
        bytes.extend_from_slice(&buf[..count]);
    }
    let body = if length == 0 {
        Value::Null
    } else {
        serde_json::from_slice(&bytes[header_end..header_end + length]).unwrap()
    };
    (path, body, authorized)
}
fn message(id: &str, model: &str, text: &str) -> Value {
    json!({"id":id,"model":model,"status":"completed","output":[{"id":format!("msg-{id}"),
        "type":"message","role":"assistant","content":[{"type":"output_text","text":text}]}]})
}
fn call_item(id: &str, token: &str, index: usize) -> Value {
    json!({"type":"function_call","id":format!("item-{id}-{index}"),
        "call_id":format!("call-{id}-{index}"),"name":"codex_usb_echo",
        "arguments":json!({"token":token}).to_string()})
}
fn handle_response(body: &Value, state: &mut GatewayState) -> (&'static str, Vec<u8>) {
    state.counter += 1;
    let id = format!("resp-{}", state.counter);
    let model = body["model"].as_str().unwrap();
    if body["stream"] == true {
        let input = body["input"].as_str().unwrap();
        let token = input
            .split_once("with token ")
            .unwrap()
            .1
            .trim_end_matches('.');
        let item = call_item(&id, token, 0);
        let item_id = item["id"].as_str().unwrap();
        let response = json!({"id":id,"model":model,"status":"completed","output":[item.clone()]});
        let mut added = item.clone();
        added["arguments"] = json!("");
        let mut events = vec![
            json!({"type":"response.created","sequence_number":0,"response":{"id":id}}),
            json!({"type":"response.in_progress","sequence_number":1,"response":{"id":id}}),
            json!({"type":"response.output_item.added","sequence_number":2,"output_index":0,"item":added}),
            json!({"type":"response.function_call_arguments.delta","sequence_number":3,"item_id":item_id,"output_index":0,"delta":item["arguments"]}),
            json!({"type":"response.function_call_arguments.done","sequence_number":4,"item_id":item_id,"output_index":0,"arguments":item["arguments"]}),
            json!({"type":"response.output_item.done","sequence_number":5,"output_index":0,"item":item}),
            json!({"type":"response.completed","sequence_number":6,"response":response}),
        ];
        if model == "fixture-bad-stream" {
            events.swap(3, 4);
        }
        let bytes = events
            .iter()
            .map(|v| format!("event: {}\ndata: {}\n\n", v["type"].as_str().unwrap(), v))
            .collect::<String>()
            .into_bytes();
        return ("text/event-stream", bytes);
    }
    let response = if let Some(prior) = body.get("previous_response_id").and_then(Value::as_str) {
        let previous = state
            .responses
            .get(prior)
            .expect("fixture requires valid continuation");
        if let Some(outputs) = body["input"].as_array() {
            assert_eq!(
                outputs.len(),
                previous.calls.len(),
                "tool output count must match"
            );
            for (call_id, token) in &previous.calls {
                let matching: Vec<_> = outputs
                    .iter()
                    .filter(|o| o["call_id"].as_str() == Some(call_id.as_str()))
                    .collect();
                assert_eq!(matching.len(), 1, "each call must be returned exactly once");
                assert_eq!(matching[0]["type"], "function_call_output");
                let output = matching[0]["output"].as_str().unwrap();
                let value: Value = serde_json::from_str(output).unwrap();
                assert_eq!(value["token"].as_str(), Some(token.as_str()));
                assert!(value["challenge"].as_str().is_some_and(|s| !s.is_empty()));
                if previous.goal == "RECOVERED|" {
                    assert_eq!(value["error"], "fixture_error");
                } else {
                    assert!(value.get("error").is_none());
                }
            }
            let mut returned = BTreeMap::new();
            for output in outputs {
                let value: Value =
                    serde_json::from_str(output["output"].as_str().unwrap()).unwrap();
                returned.insert(
                    value["token"].as_str().unwrap().to_owned(),
                    value["challenge"].as_str().unwrap().to_owned(),
                );
            }
            let answer = if model == "fixture-ignores-output" {
                previous.calls[0].1.clone()
            } else {
                format!(
                    "{}{}",
                    previous.goal,
                    returned.into_values().collect::<Vec<_>>().join("|")
                )
            };
            let response = message(&id, model, &answer);
            state.continued_tools += 1;
            response
        } else {
            assert_eq!(
                body["input"],
                "Repeat the original token exactly, with no other text."
            );
            let token = previous.token.clone();
            let rounds = previous.text_continuations + 1;
            let response = message(&id, model, &token);
            state.responses.insert(
                id.clone(),
                Stored {
                    token,
                    text_continuations: rounds,
                    ..Default::default()
                },
            );
            state.continued_text += 1;
            response
        }
    } else if body.get("tools").is_some() {
        assert_eq!(body["tools"][0]["name"], "codex_usb_echo");
        let input = body["input"].as_str().unwrap();
        let after = input.split_once("for each token: ").unwrap().1;
        let (tokens, _) = after
            .split_once(". After receiving all function outputs, sort them by token")
            .unwrap();
        let goal = if input.contains("prefixed with RECOVERED|") {
            "RECOVERED|"
        } else {
            ""
        };
        let tokens: Vec<_> = tokens.split(',').collect();
        let mut output: Vec<Value> = tokens
            .iter()
            .enumerate()
            .map(|(i, t)| call_item(&id, t, i))
            .collect();
        let calls = output
            .iter()
            .zip(&tokens)
            .map(|(item, token)| {
                (
                    item["call_id"].as_str().unwrap().into(),
                    (*token).to_owned(),
                )
            })
            .collect();
        state.responses.insert(
            id.clone(),
            Stored {
                goal: goal.into(),
                calls,
                ..Default::default()
            },
        );
        if model == "fixture-bad-call" {
            output[0].as_object_mut().unwrap().remove("call_id");
        }
        json!({"id":id,"model":model,"status":"completed","output":output})
    } else {
        let token = body["input"]
            .as_str()
            .unwrap()
            .split_once("Reply exactly ")
            .unwrap()
            .1;
        state.responses.insert(
            id.clone(),
            Stored {
                token: token.into(),
                ..Default::default()
            },
        );
        message(&id, model, token)
    };
    ("application/json", serde_json::to_vec(&response).unwrap())
}
#[test]
fn complete_loopback_workflow_never_certifies_native_shell_or_skills() {
    let fixture = Fixture::start();
    let cancel = Arc::new(AtomicBool::new(false));
    let client = ProbeClient::new(
        &fixture.base,
        "fixture-only-key",
        cancel,
        Instant::now() + Duration::from_secs(20),
    )
    .unwrap();
    let mut model =
        ModelCapabilityReport::discovered("fixture-good".into(), Some("fixture".into()));
    tauri::async_runtime::block_on(async {
        client.health().await.unwrap();
        let compatibility = client.codex_compatibility().await.unwrap();
        assert_eq!(compatibility["profile_id"], "codex-responses-tool-use-v1");
        assert_eq!(compatibility["default_model_id"], "fixture-good");
        assert!(client.models().await.unwrap()["data"].is_array());
        client.probe_model(&mut model, 100).await;
    });
    for actual in [
        model.checks.text,
        model.checks.single_tool,
        model.checks.tool_result,
        model.checks.multiple_tools,
        model.checks.tool_error,
        model.checks.stream,
        model.checks.multi_turn,
    ] {
        assert_eq!(actual, CapabilityState::Verified);
    }
    let context = CapabilityContext {
        binding_fingerprint: "fixture".into(),
        codex_version: None,
        native_runtime_version: None,
        adapter_version: None,
    };
    model.evaluate(&context, None, 100);
    assert_eq!(model.evidence_scope, EvidenceScope::Mock);
    assert_eq!(model.state, ModelState::Unverified);
    assert_eq!(model.checks.shell, CapabilityState::Unknown);
    assert_eq!(model.checks.skills, CapabilityState::Unknown);
    let state = fixture.state.lock().unwrap();
    assert_eq!(
        state.counter, 12,
        "the complete gateway matrix must issue exactly twelve generation requests"
    );
    assert_eq!(client.requests_started(), 15);
    assert_eq!(state.continued_text, 4);
    assert_eq!(state.continued_tools, 3);
}

#[test]
fn directory_only_check_sends_one_models_request_and_no_generation() {
    let fixture = Fixture::start();
    let client = ProbeClient::new(
        &fixture.base,
        "fixture-only-key",
        Arc::new(AtomicBool::new(false)),
        Instant::now() + Duration::from_secs(20),
    )
    .unwrap();
    tauri::async_runtime::block_on(client.models()).unwrap();
    assert_eq!(client.requests_started(), 1);
    assert_eq!(fixture.state.lock().unwrap().counter, 0);
}
#[test]
fn malformed_call_and_stream_are_isolated_per_model() {
    let fixture = Fixture::start();
    let cancel = Arc::new(AtomicBool::new(false));
    let client = ProbeClient::new(
        &fixture.base,
        "fixture-only-key",
        cancel,
        Instant::now() + Duration::from_secs(20),
    )
    .unwrap();
    for (name, check) in [
        ("fixture-bad-call", "call"),
        ("fixture-bad-stream", "stream"),
    ] {
        let mut model = ModelCapabilityReport::discovered(name.into(), None);
        tauri::async_runtime::block_on(client.probe_model(&mut model, 100));
        let failed = if check == "call" {
            model.checks.single_tool
        } else {
            model.checks.stream
        };
        assert_eq!(failed, CapabilityState::Quarantined);
        assert_eq!(model.checks.shell, CapabilityState::Unknown);
    }
    let mut good = ModelCapabilityReport::discovered("fixture-good".into(), None);
    tauri::async_runtime::block_on(client.probe_model(&mut good, 100));
    assert_eq!(good.checks.multi_turn, CapabilityState::Verified);
}

#[test]
fn ignored_tool_outputs_do_not_pass_continuation() {
    let fixture = Fixture::start();
    let client = ProbeClient::new(
        &fixture.base,
        "fixture-only-key",
        Arc::new(AtomicBool::new(false)),
        Instant::now() + Duration::from_secs(20),
    )
    .unwrap();
    let mut model = ModelCapabilityReport::discovered("fixture-ignores-output".into(), None);
    tauri::async_runtime::block_on(client.probe_model(&mut model, 100));
    assert_eq!(model.checks.text, CapabilityState::Verified);
    assert_eq!(model.checks.single_tool, CapabilityState::Verified);
    assert_eq!(model.checks.tool_result, CapabilityState::Quarantined);
    assert_ne!(model.state, ModelState::Ready);
}
