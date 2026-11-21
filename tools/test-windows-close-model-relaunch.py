"""Run the ignored real-desktop regression on a disposable copy of an installed USB runtime."""
import argparse
import json
import os
from pathlib import Path
import stat
import shutil
import subprocess
import tempfile
import urllib.request

import psutil
import websocket
import time
import uuid


def check_desktop(pid, image, port, expected):
    process = psutil.Process(pid)
    assert Path(process.exe()).samefile(image)
    deadline = time.monotonic() + 30
    while not any(c.status == psutil.CONN_LISTEN and c.laddr.ip == "127.0.0.1" and c.laddr.port == port
                  for c in process.net_connections("tcp")):
        assert process.is_running() and time.monotonic() < deadline, "test desktop debug endpoint did not become ready"
        time.sleep(0.2)
    http = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    deadline = time.monotonic() + 30
    page = None
    while not page and time.monotonic() < deadline:
        with http.open(f"http://127.0.0.1:{port}/json/list", timeout=3) as response:
            page = next((p for p in json.load(response) if p["type"] == "page" and p["url"].startswith("app://")), None)
        if not page:
            time.sleep(0.2)
    assert page and page["webSocketDebuggerUrl"].startswith(f"ws://127.0.0.1:{port}/")
    ws = websocket.create_connection(page["webSocketDebuggerUrl"], timeout=40, suppress_origin=True)
    def evaluate(number, expression):
        ws.send(json.dumps({"id": number, "method": "Runtime.evaluate", "params": {
            "expression": expression, "returnByValue": True, "awaitPromise": True}}))
        while True:
            message = json.loads(ws.recv())
            if message.get("id") == number:
                assert "error" not in message and "exceptionDetails" not in message["result"], "fixture evaluation failed"
                return message["result"]["result"].get("value")
    descendants = process.children(recursive=True)
    try:
        actual = evaluate(1, """new Promise((resolve,reject)=>{
          const id='usb-model-list-'+Date.now();
          const timer=setTimeout(()=>reject(Error('model list timed out')),30000);
          const onMessage=({data})=>{
            if (data?.type!=='mcp-response'||data.message?.id!==id) return;
            clearTimeout(timer); window.removeEventListener('message',onMessage);
            if(data.message.error) reject(Error('app-server rejected model/list'));
            else resolve(data.message.result.data.some(m=>m.model==='close-relaunch-fixture'));
          };
          window.addEventListener('message',onMessage);
          window.electronBridge.sendMessageFromView({type:'mcp-request',hostId:'local',retainResponse:false,
            request:{id,method:'model/list',params:{includeHidden:false,limit:100}}}).catch(reject);
        })""")
        assert actual is expected, (actual, expected)
        print(f"desktop pid={pid}: its app-server model/list contains added model={actual}", flush=True)
    finally:
        # On Windows, closing the last window can leave the app resident. Use
        # the desktop's Quit action, scoped to the renderer of this isolated PID.
        evaluate(2, "setTimeout(()=>window.electronBridge.sendMessageFromView({type:'quit-app'}),100); true")
        ws.close()
    assert process.wait(timeout=40) == 0
    return descendants


def extended(path):
    return "\\\\?\\" + str(path.resolve())


def cleanup_fixture(root, run, last_workers):
    # This newly created GUID-specific directory is wholly owned by this run.
    assert root.resolve() == Path(tempfile.gettempdir()).resolve() / ("codex-app-manager-smoke-" + run)
    # Windows may hold the working directory briefly after both main processes
    # have exited. Retry only fixture teardown on an actual sharing violation.
    def remove_readonly_file(operation, path, error):
        if (error.winerror == 5 and operation is os.unlink and
                os.lstat(path).st_file_attributes & stat.FILE_ATTRIBUTE_READONLY):
            os.chmod(path, stat.S_IWRITE)
            operation(path)
        else:
            raise error

    started = time.monotonic()
    deadline = started + 90
    inspected = False
    while True:
        try:
            shutil.rmtree(extended(root), onexc=remove_readonly_file)
            break
        except PermissionError as error:
            if error.winerror != 32 or time.monotonic() >= deadline:
                raise
            if not inspected:
                still_running = [p for p in last_workers if p.is_running()]
                print("Fixture teardown sharing violation; live observed worker PIDs:", [p.pid for p in still_running], flush=True)
                inspected = True
            time.sleep(0.5)
    print(f"Fixture directory released after {time.monotonic()-started:.1f}s", flush=True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("manager_tests", type=Path)
    parser.add_argument("installed_bundle", type=Path)
    args = parser.parse_args()
    run = "relaunch-" + uuid.uuid4().hex
    root = Path(tempfile.gettempdir()) / ("codex-app-manager-smoke-" + run)
    root.mkdir()
    runtime = root / "apps" / "Codex"
    shutil.copytree(extended(args.installed_bundle / "apps" / "Codex"), extended(runtime))
    shutil.copy2(args.installed_bundle / "CodexEntryShim.exe", root)
    home = root / "home"
    home.mkdir()
    env = {k: v for k, v in os.environ.items()
           if not k.upper().startswith(("CODEX_", "OPENAI_", "CHATGPT_", "OTEL_", "CAM_PACKAGED_"))}
    env.update(CAM_PACKAGED_SMOKE_RUN=run, CAM_PACKAGED_SMOKE_DATA_DIR=str(root), CODEX_HOME=str(home),
               CODEX_ELECTRON_USER_DATA_PATH=str(root / "electron"), APPDATA=str(root / "roaming"),
               LOCALAPPDATA=str(root / "local"), HTTP_PROXY="http://127.0.0.1:1",
               HTTPS_PROXY="http://127.0.0.1:1", ALL_PROXY="http://127.0.0.1:1",
               NO_PROXY="127.0.0.1,localhost", CODEX_SPARKLE_ENABLED="false", OTEL_SDK_DISABLED="true")
    for var in ["APPDATA", "LOCALAPPDATA", "CODEX_ELECTRON_USER_DATA_PATH"]:
        Path(env[var]).mkdir()
    (home / "config.toml").write_text(
        'model_provider = "custom"\ncli_auth_credentials_store = "file"\n'
        '[model_providers.custom]\nname = "Isolated offline test"\nbase_url = "http://127.0.0.1:1/v1"\n'
        'wire_api = "responses"\n[desktop]\nconversationDetailMode = "STEPS_COMMANDS"\n', encoding="utf-8")
    case = "commands::session_preservation_tests::real_desktop_close_add_and_relaunch"
    with subprocess.Popen([str(args.manager_tests.resolve()), "--exact", case, "--ignored", "--nocapture"],
                          cwd=root, env=env) as manager:
        pids = []
        last_workers = []
        for stage in ["first", "reopened"]:
            ready = root / (stage + ".pid")
            deadline = time.monotonic() + 120
            while not ready.exists() or not ready.stat().st_size:
                assert manager.poll() is None and time.monotonic() < deadline, f"{stage} launch failed"
                time.sleep(0.1)
            pids.append(int(ready.read_text()))
            port = int((root / (stage + ".port")).read_text())
            last_workers = check_desktop(pids[-1], runtime / "CodexCore.dat", port, stage == "reopened")
        assert pids[0] != pids[1] and manager.wait(timeout=30) == 0
    cleanup_fixture(root, run, last_workers)
    print("PASS: two confirmed windows, two clean exits, model transition, consumed reload marker.")


if __name__ == "__main__":
    main()
