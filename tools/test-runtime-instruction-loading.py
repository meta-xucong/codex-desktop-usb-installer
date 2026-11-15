"""Offline thread/start check in fresh fixture homes. Never sends a model turn."""
import argparse
import json
import os
from pathlib import Path
import queue
import subprocess
import threading
import time


def request(process, messages, request_id, method, params):
    process.stdin.write(json.dumps({"id": request_id, "method": method, "params": params}) + "\n")
    process.stdin.flush()
    deadline = time.monotonic() + 30
    while time.monotonic() < deadline:
        try:
            message = messages.get(timeout=max(0.01, deadline - time.monotonic()))
        except queue.Empty:
            break
        if message.get("id") == request_id:
            if "error" in message:
                raise RuntimeError(f"{method}: protocol error code={message['error'].get('code')}")
            return message["result"]
    raise TimeoutError(f"{method}: no response within 30 seconds")


def probe(codex, writer, root, override):
    home, workspace = root / "home", root / "workspace"
    home.mkdir(parents=True, exist_ok=False)
    workspace.mkdir()
    # The production AGENTS writer is compiled into the standalone fixture writer.
    subprocess.run([str(writer), str(home)], check=True, capture_output=True,
                   creationflags=subprocess.CREATE_NO_WINDOW, timeout=15)
    guidance = (home / "AGENTS.md").read_text(encoding="utf-8")
    assert "USB-TOOLS-v1" in guidance
    if override:
        (home / "AGENTS.override.md").write_text("User override fixture.\n", encoding="utf-8")
    # No real account, model endpoint or production config is copied into this home.
    (home / "config.toml").write_text(
        'model = "offline-fixture"\nmodel_provider = "offline"\n'
        '[model_providers.offline]\nname = "Offline fixture"\n'
        'base_url = "http://127.0.0.1:9/v1"\nwire_api = "responses"\n'
        'requires_openai_auth = false\n', encoding="utf-8")
    environment = dict(os.environ, CODEX_HOME=str(home))
    messages = queue.Queue()
    with (root / "app-server.stderr.log").open("w", encoding="utf-8") as stderr:
        process = subprocess.Popen([str(codex), "app-server", "--stdio"], cwd=workspace,
            env=environment, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=stderr,
            text=True, encoding="utf-8", errors="replace", creationflags=subprocess.CREATE_NO_WINDOW)
        def read_messages():
            for line in process.stdout:
                try:
                    message = json.loads(line)
                    if isinstance(message, dict):
                        messages.put(message)
                except json.JSONDecodeError:
                    pass
        threading.Thread(target=read_messages, daemon=True).start()
        try:
            request(process, messages, 1, "initialize", {
                "clientInfo": {"name": "usb-guidance-fixture", "version": "1"},
                "capabilities": {"experimentalApi": True}})
            process.stdin.write('{"method":"initialized","params":{}}\n')
            process.stdin.flush()
            result = request(process, messages, 2, "thread/start", {
                "cwd": str(workspace), "ephemeral": True,
                "approvalPolicy": "never", "sandbox": "read-only"})
            sources = result.get("instructionSources", [])
            normalized = {os.path.normcase(os.path.normpath(path)) for path in sources}
            expected = home / ("AGENTS.override.md" if override else "AGENTS.md")
            loaded = os.path.normcase(os.path.normpath(str(expected))) in normalized
            base_loaded = os.path.normcase(os.path.normpath(str(home / "AGENTS.md"))) in normalized
            record = {"scenario": "user-override" if override else "global-guidance",
                      "instructionSources": sources, "expectedSourceLoaded": loaded,
                      "globalAgentsLoaded": base_loaded, "modelTurnSent": False}
            (root / "result.json").write_text(json.dumps(record, ensure_ascii=False, indent=2), encoding="utf-8")
            print(json.dumps(record, ensure_ascii=False))
            assert loaded, "Expected instruction source was not reported"
            assert base_loaded != override, "Global override precedence differs from the expected contract"
            assert (home / "AGENTS.md").read_text(encoding="utf-8") == guidance
        finally:
            process.stdin.close()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.terminate()  # Only this harness-owned, isolated app-server child.
                process.wait(timeout=5)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--codex", required=True, type=Path)
    parser.add_argument("--fixture-writer", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=False)
    probe(args.codex.resolve(), args.fixture_writer.resolve(), args.output / "global", False)
    probe(args.codex.resolve(), args.fixture_writer.resolve(), args.output / "override", True)
