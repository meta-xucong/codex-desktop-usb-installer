#!/usr/bin/env python3
"""Reliable, fail-closed synchronizer for the Codex update relay."""

from __future__ import annotations

import argparse
try:
    import fcntl
except ImportError:  # test import on Windows; the deployed service is Linux
    fcntl = None  # type: ignore[assignment]
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request
import uuid
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Callable, Dict, Iterable, Tuple

DEFAULT_CONFIG = "/etc/codex-update-relay/config.json"
USER_AGENT = "codex-update-relay/2"
HEX64 = re.compile(r"^[0-9a-fA-F]{64}$")
RELEASE_ID = re.compile(r"^[0-9]+$")


class RelayError(RuntimeError):
    pass


def fail(message: str) -> None:
    raise RelayError(message)


def log(message: str) -> None:
    print(f"codex-update-relay: {message}", flush=True)


def now() -> str:
    return datetime.now(timezone.utc).replace(microsecond=0).isoformat().replace("+00:00", "Z")


def read_json(path: Path) -> Dict[str, Any]:
    try:
        with path.open("r", encoding="utf-8") as handle:
            value = json.load(handle)
    except (OSError, json.JSONDecodeError) as exc:
        fail(f"cannot read JSON {path}: {exc}")
    if not isinstance(value, dict):
        fail(f"JSON root must be an object: {path}")
    return value


def validate_config(config: Dict[str, Any]) -> Dict[str, Any]:
    repo = config.get("repository")
    if not isinstance(repo, str) or not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", repo):
        fail("config.repository must be owner/repository")
    architecture = config.get("architecture", "x64")
    if architecture not in {"x64", "arm64"}:
        fail("config.architecture must be x64 or arm64")

    def positive(name: str, default: int) -> int:
        value = int(config.get(name, default))
        if value < 1:
            fail(f"config.{name} must be positive")
        return value

    values = {
        "repository": repo,
        "architecture": architecture,
        "root": Path(config.get("root", "/srv/codex-update-relay")).resolve(),
        "min_free_bytes": int(config.get("min_free_bytes", 536870912)),
        "max_published_releases": positive("max_published_releases", 1),
        "download_timeout_seconds": positive("download_timeout_seconds", 7200),
        "retry_attempts": positive("retry_attempts", 3),
        "retry_base_seconds": positive("retry_base_seconds", 10),
        "retry_max_seconds": positive("retry_max_seconds", 300),
    }
    if values["min_free_bytes"] < 268435456:
        fail("config.min_free_bytes must be at least 256 MiB")
    if values["download_timeout_seconds"] < 60:
        fail("config.download_timeout_seconds is too small")
    if values["retry_max_seconds"] < values["retry_base_seconds"]:
        fail("config.retry_max_seconds must be >= retry_base_seconds")
    return values


def require_https_github(url: str, expected_host: str) -> str:
    parsed = urllib.parse.urlparse(url)
    if parsed.scheme != "https" or parsed.hostname != expected_host or parsed.username or parsed.password:
        fail(f"unexpected GitHub URL: {url}")
    return url


def github_json(url: str) -> Dict[str, Any]:
    require_https_github(url, "api.github.com")
    request = urllib.request.Request(url, headers={"Accept": "application/vnd.github+json", "User-Agent": USER_AGENT})
    try:
        with urllib.request.urlopen(request, timeout=30) as response:
            payload = response.read()
    except (OSError, urllib.error.URLError, urllib.error.HTTPError) as exc:
        fail(f"GitHub API request failed: {exc}")
    try:
        value = json.loads(payload.decode("utf-8"))
    except (UnicodeDecodeError, json.JSONDecodeError) as exc:
        fail(f"GitHub API returned invalid JSON: {exc}")
    if not isinstance(value, dict):
        fail("GitHub API response must be an object")
    return value


def retry_call(operation: Callable[[], Any], label: str, config: Dict[str, Any], sleep: Callable[[float], None] = time.sleep) -> Any:
    last: Exception | None = None
    attempts = config["retry_attempts"]
    for number in range(1, attempts + 1):
        try:
            return operation()
        except Exception as exc:
            last = exc
            if number == attempts:
                break
            delay = min(config["retry_max_seconds"], config["retry_base_seconds"] * (2 ** (number - 1)))
            if isinstance(config.get("root"), Path):
                write_status(config, lastAttemptAt=now(), lastError=str(exc), nextRetryAt=now())
            log(f"{label} failed (attempt {number}/{attempts}); retrying in {delay}s: {exc}")
            sleep(delay)
    assert last is not None
    raise last


def _asset_digest(asset: Dict[str, Any]) -> str:
    value = asset.get("digest")
    if not isinstance(value, str) or not value.startswith("sha256:") or not HEX64.fullmatch(value[7:]):
        fail(f"asset {asset.get('name', '?')} has no valid GitHub sha256 digest")
    return value[7:].lower()


def release_assets(release: Dict[str, Any]) -> Dict[str, Dict[str, Any]]:
    assets = release.get("assets")
    if not isinstance(assets, list):
        fail("release.assets is missing or invalid")
    result: Dict[str, Dict[str, Any]] = {}
    for asset in assets:
        if not isinstance(asset, dict):
            continue
        name, url, size, asset_id = asset.get("name"), asset.get("browser_download_url"), asset.get("size"), asset.get("id")
        if not isinstance(name, str) or not isinstance(url, str) or not isinstance(size, int) or not isinstance(asset_id, int):
            continue
        if "/" in name or chr(92) in name or size < 0 or asset_id < 1:
            continue
        require_https_github(url, "github.com")
        _asset_digest(asset)
        if name in result:
            fail(f"duplicate release asset name: {name}")
        result[name] = asset
    return result


def choose_assets(release: Dict[str, Any], architecture: str) -> Tuple[Dict[str, Any], Dict[str, Any], Dict[str, Any]]:
    assets = release_assets(release)
    msix_candidates = [asset for asset in assets.values() if asset["name"].lower().endswith(".msix") and re.search(rf"_{re.escape(architecture)}__", asset["name"], re.IGNORECASE)]
    if len(msix_candidates) != 1:
        fail(f"expected exactly one {architecture} MSIX, got {[a.get('name', '?') for a in msix_candidates]}")

    def required(name: str) -> Dict[str, Any]:
        value = assets.get(name)
        if value is None:
            fail(f"required release asset is missing: {name}")
        return value

    return msix_candidates[0], required("release-manifest.json"), required("SHA256SUMS-windows.txt")


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def download_asset(asset: Dict[str, Any], destination: Path, timeout_seconds: int) -> None:
    url = require_https_github(str(asset["browser_download_url"]), "github.com")
    destination.parent.mkdir(parents=True, exist_ok=True)
    command = ["/usr/bin/curl", "--fail", "--silent", "--show-error", "--location", "--proto", "=https", "--retry", "1", "--connect-timeout", "15", "--max-time", str(timeout_seconds), "--user-agent", USER_AGENT, "--header", "Accept: application/octet-stream", "--output", str(destination), url]
    try:
        subprocess.run(command, check=True, stdout=subprocess.DEVNULL)
    except (OSError, subprocess.CalledProcessError) as exc:
        fail(f"download failed for {asset.get('name')}: {exc}")


def parse_checksums(path: Path) -> Dict[str, str]:
    result: Dict[str, str] = {}
    for line in path.read_text(encoding="utf-8", errors="strict").splitlines():
        match = re.match(r"^\s*([0-9a-fA-F]{64})\s+\*?(.+?)\s*$", line)
        if not match:
            continue
        name, digest = Path(match.group(2)).name, match.group(1).lower()
        if name in result and result[name] != digest:
            fail(f"duplicate checksum with conflicting values: {name}")
        result[name] = digest
    if not result:
        fail("SHA256SUMS-windows.txt contains no valid checksums")
    return result


def verify_download(path: Path, asset: Dict[str, Any], checksums: Dict[str, str] | None = None) -> str:
    name, expected_size = str(asset["name"]), int(asset["size"])
    actual_size = path.stat().st_size
    if actual_size != expected_size:
        fail(f"size mismatch for {name}: expected {expected_size}, got {actual_size}")
    digest = sha256_file(path)
    if digest != _asset_digest(asset):
        fail(f"GitHub digest mismatch for {name}")
    if checksums is not None and (checksums.get(name) is None or digest != checksums[name]):
        fail(f"SHA256SUMS-windows.txt mismatch for {name}")
    return digest


def validate_manifest(path: Path, msix: Dict[str, Any], msix_digest: str) -> Dict[str, Any]:
    manifest = read_json(path)
    derived = manifest.get("derived")
    if not isinstance(derived, dict):
        fail("release-manifest.json has no derived section")
    checksums = derived.get("latestChecksums")
    if not isinstance(checksums, dict) or checksums.get(msix["name"]) != msix_digest:
        fail("release-manifest.json does not bind the selected MSIX digest")
    windows = manifest.get("sources", {}).get("windows", {})
    arch = windows.get("architectures", {}).get("x64" if "_x64__" in msix["name"].lower() else "arm64", {})
    if not isinstance(arch, dict) or int(arch.get("contentLength", -1)) != int(msix["size"]):
        fail("release-manifest.json does not bind the selected MSIX size")
    if not manifest.get("codexVersion") or not manifest.get("publishedAt"):
        fail("release-manifest.json lacks release identity")
    return manifest


def write_json_atomic(path: Path, value: Dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temporary = path.with_name(f".{path.name}.{uuid.uuid4().hex}.tmp")
    encoded = json.dumps(value, ensure_ascii=True, indent=2, sort_keys=True) + "\n"
    try:
        with temporary.open("w", encoding="utf-8") as handle:
            handle.write(encoded)
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(temporary, path)
    finally:
        temporary.unlink(missing_ok=True)


def current_descriptor(export_root: Path) -> Dict[str, Any] | None:
    latest = export_root / "current" / "latest.json"
    if not latest.exists():
        return None
    try:
        return read_json(latest)
    except RelayError:
        return None


def _inside(path: Path, parent: Path) -> bool:
    try:
        path.resolve(strict=False).relative_to(parent.resolve())
        return True
    except ValueError:
        return False


def _journal_path(root: Path, value: Any, label: str) -> Path:
    if not isinstance(value, str) or not value:
        fail(f"publish journal {label} path is missing")
    path = Path(value)
    if not path.is_absolute():
        path = (root / path).resolve(strict=False)
    else:
        path = path.resolve(strict=False)
    if not _inside(path, root):
        fail(f"publish journal {label} path escapes relay root")
    return path


def identity_from(release: Dict[str, Any], msix: Dict[str, Any], manifest: Dict[str, Any], checksums: Dict[str, Any], digests: Dict[str, str] | None = None) -> Dict[str, Any]:
    release_id = str(release.get("id"))
    if not RELEASE_ID.fullmatch(release_id):
        fail("release.id is missing or invalid")
    tag, published_at = release.get("tag_name"), release.get("published_at")
    if not isinstance(tag, str) or not tag or not isinstance(published_at, str) or not published_at:
        fail("release tag_name/published_at is missing")
    result: Dict[str, Any] = {"releaseId": int(release_id), "tag": tag, "publishedAt": published_at, "assets": {}}
    for key, asset in zip(("windows-msix", "release-manifest", "checksums"), (msix, manifest, checksums)):
        result["assets"][key] = {"id": int(asset["id"]), "path": asset["name"], "size": int(asset["size"]), "githubDigest": asset["digest"], "sha256": (digests or {}).get(key, _asset_digest(asset))}
    return result


def identity_equal(left: Dict[str, Any] | None, right: Dict[str, Any]) -> bool:
    if not left:
        return False
    return left.get("releaseId") == right.get("releaseId") and left.get("tag") == right.get("tag") and left.get("publishedAt") == right.get("publishedAt") and left.get("assets") == right.get("assets")


def switch_current(export_root: Path, target: str) -> None:
    if os.path.isabs(target):
        fail("current target must be relative")
    target_path = (export_root / target).resolve(strict=False)
    if not _inside(target_path, export_root / "releases") or target_path.parent != (export_root / "releases").resolve():
        fail("current target escapes published releases")
    current = export_root / "current"
    if current.exists() and not current.is_symlink():
        fail(f"current is not a symlink: {current}")
    temporary = export_root / f".current-{uuid.uuid4().hex}"
    try:
        os.symlink(target, temporary, target_is_directory=True)
        os.replace(temporary, current)
    finally:
        temporary.unlink(missing_ok=True)


def clear_journal(path: Path) -> None:
    path.unlink(missing_ok=True)


def acquire_lock(lock_handle: Any) -> None:
    if fcntl is None:
        return
    try:
        fcntl.flock(lock_handle.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
    except BlockingIOError:
        fail("another sync is already running")


def recover_publish(root: Path, journal_path: Path) -> None:
    if not journal_path.exists():
        return
    journal = read_json(journal_path)
    phase = journal.get("phase")
    if journal.get("schema") != 1:
        fail("publish journal schema is unsupported")
    if not isinstance(journal.get("transactionId"), str) or not re.fullmatch(r"[0-9a-f]{32}", journal["transactionId"]):
        fail("publish journal transactionId is invalid")
    export_root = root / "export"
    incoming = _journal_path(root, journal.get("incoming"), "incoming")
    final = _journal_path(root, journal.get("final"), "final")
    retired = _journal_path(root, journal.get("retired"), "retired") if journal.get("retired") else None
    releases_root = root / "export" / "releases"
    if not _inside(incoming, releases_root) or not _inside(final, releases_root) or incoming.parent != releases_root.resolve() or final.parent != releases_root.resolve() or (retired is not None and (not _inside(retired, releases_root) or retired.parent != releases_root.resolve())):
        fail("publish journal published path escapes releases root")
    if phase == "staged":
        stage = _journal_path(root, journal.get("stage"), "stage")
        if not _inside(stage, root / "work"):
            fail("publish journal stage path is outside work")
        if not stage.is_dir():
            fail("publish journal stage is not a directory")
        if stage.exists():
            shutil.rmtree(stage)
        clear_journal(journal_path)
        return
    if phase in {"incoming-ready", "current-switched"}:
        if not incoming.exists() and final.exists():
            switch_current(export_root, os.path.relpath(final, export_root))
            if retired and retired.exists():
                shutil.rmtree(retired)
            clear_journal(journal_path)
            return
        if not incoming.exists():
            fail("publish journal has no recoverable incoming or final directory")
        if phase == "incoming-ready":
            switch_current(export_root, os.path.relpath(incoming, export_root))
        if final.exists() and retired is not None and not retired.exists():
            os.replace(final, retired)
        os.replace(incoming, final)
        switch_current(export_root, os.path.relpath(final, export_root))
        if retired and retired.exists():
            shutil.rmtree(retired)
        clear_journal(journal_path)
        return
    if phase == "finalized":
        if final.exists():
            switch_current(export_root, os.path.relpath(final, export_root))
        if retired and retired.exists():
            shutil.rmtree(retired)
        clear_journal(journal_path)
        return
    fail(f"unknown publish journal phase: {phase}")


def prune_published_releases(releases_root: Path, current_id: str, keep: int) -> None:
    candidates = [item for item in releases_root.iterdir() if item.is_dir() and not item.is_symlink() and item.name.isdigit()]
    candidates.sort(key=lambda item: int(item.name))
    removable = [item for item in candidates if item.name != current_id]
    while len(candidates) > keep and removable:
        target = removable.pop(0)
        shutil.rmtree(target)
        candidates.remove(target)
        log(f"pruned old release {target.name}")


def publish(release: Dict[str, Any], config: Dict[str, Any], assets: Tuple[Dict[str, Any], Dict[str, Any], Dict[str, Any]], stage: Path, digests: Dict[str, str]) -> None:
    root: Path = config["root"]
    export_root, releases_root, journal_path = root / "export", root / "export" / "releases", root / "state" / "publish-journal.json"
    release_id = str(release["id"])
    final = releases_root / release_id
    identity = identity_from(release, *assets, digests)
    descriptor = {"protocol": 2, "repository": config["repository"], "architecture": config["architecture"], "identity": identity, "releaseId": int(release_id), "tag": release.get("tag_name"), "publishedAt": release.get("published_at"), "assets": {}}
    for key, asset in zip(("windows-msix", "release-manifest", "checksums"), assets):
        descriptor["assets"][key] = {"id": int(asset["id"]), "path": asset["name"], "size": int(asset["size"]), "sha256": digests[key], "githubDigest": asset["digest"]}
    write_json_atomic(stage / "latest.json", descriptor)
    for item in stage.iterdir():
        if item.is_file():
            item.chmod(0o644)
    stage.chmod(0o755)
    if final.exists() and (final / "latest.json").exists():
        existing = read_json(final / "latest.json")
        if identity_equal(existing.get("identity", existing), identity):
            shutil.rmtree(stage)
            switch_current(export_root, os.path.join("releases", release_id))
            clear_journal(journal_path)
            return
    tx = uuid.uuid4().hex
    incoming, retired = releases_root / f".incoming-{release_id}-{tx}", releases_root / f".retired-{release_id}-{tx}"
    current = export_root / "current"
    old_current = os.readlink(current) if current.is_symlink() else None
    base = {"schema": 1, "transactionId": tx, "releaseId": int(release_id), "incoming": str(incoming), "final": str(final), "retired": str(retired), "oldCurrent": old_current, "oldFinal": str(final) if final.exists() else None, "identity": identity, "updatedAt": now()}
    write_json_atomic(journal_path, {**base, "phase": "staged", "stage": str(stage)})
    os.replace(stage, incoming)
    write_json_atomic(journal_path, {**base, "phase": "incoming-ready"})
    switch_current(export_root, os.path.relpath(incoming, export_root))
    write_json_atomic(journal_path, {**base, "phase": "current-switched"})
    if final.exists():
        os.replace(final, retired)
    os.replace(incoming, final)
    switch_current(export_root, os.path.join("releases", release_id))
    if retired.exists():
        shutil.rmtree(retired)
    write_json_atomic(journal_path, {**base, "phase": "finalized"})
    clear_journal(journal_path)
    prune_published_releases(releases_root, release_id, config["max_published_releases"])
    log(f"published release {release_id} ({release.get('tag_name')})")


def write_status(config: Dict[str, Any], **updates: Any) -> None:
    path = config["root"] / "state" / "sync-status.json"
    value: Dict[str, Any] = {}
    if path.exists():
        try:
            value = read_json(path)
        except RelayError:
            value = {}
    value.update({"schema": 1, **updates})
    write_json_atomic(path, value)


def sync(config: Dict[str, Any], dry_run: bool) -> None:
    root: Path = config["root"]
    export_root, work_root, state_root = root / "export", root / "work", root / "state"
    for directory in (export_root, work_root, state_root, export_root / "releases"):
        directory.mkdir(parents=True, exist_ok=True)
    recover_publish(root, state_root / "publish-journal.json")
    lock_path = state_root / "sync.lock"
    with lock_path.open("a+") as lock_handle:
        acquire_lock(lock_handle)
        release = retry_call(lambda: github_json(f"https://api.github.com/repos/{config['repository']}/releases/latest"), "GitHub API", config)
        if release.get("draft") or release.get("prerelease"):
            fail("latest GitHub release is draft or prerelease")
        assets = choose_assets(release, config["architecture"])
        desired = identity_from(release, *assets)
        current = current_descriptor(export_root)
        write_status(config, desiredIdentity=desired, currentIdentity=(current or {}).get("identity", current), journalPhase="none")
        if identity_equal((current or {}).get("identity", current), desired):
            log(f"already current: {release.get('tag_name')} id={release.get('id')}")
            if not dry_run:
                prune_published_releases(export_root / "releases", str(release["id"]), config["max_published_releases"])
            return
        expected_bytes = sum(int(asset["size"]) for asset in assets)
        free_bytes = shutil.disk_usage(root).free
        # Existing release files already reduce filesystem free space. Counting
        # their size again here double-counts the rollback copy: keep current
        # intact while staging, then prune it only after the verified publish.
        required_bytes = expected_bytes + config["min_free_bytes"]
        if free_bytes < required_bytes:
            fail(f"insufficient free space: need {required_bytes} (download={expected_bytes}, safety={config['min_free_bytes']}), have {free_bytes}")
        if dry_run:
            log(f"dry-run: release={release.get('tag_name')} id={release.get('id')} msix={assets[0].get('name')} size={assets[0].get('size')} free={free_bytes}")
            return
        stage = Path(tempfile.mkdtemp(prefix=f"release-{release['id']}-", dir=str(work_root)))
        try:
            def fetch_and_verify() -> Dict[str, str]:
                for asset in assets:
                    download_asset(asset, stage / asset["name"], config["download_timeout_seconds"])
                checksums = parse_checksums(stage / assets[2]["name"])
                verified = {"windows-msix": verify_download(stage / assets[0]["name"], assets[0], checksums), "release-manifest": verify_download(stage / assets[1]["name"], assets[1]), "checksums": verify_download(stage / assets[2]["name"], assets[2])}
                validate_manifest(stage / assets[1]["name"], assets[0], verified["windows-msix"])
                return verified
            digests = retry_call(fetch_and_verify, "download and verify bundle", config)
            write_status(config, desiredIdentity=identity_from(release, *assets, digests), journalPhase="verified")
            def publish_recoverable() -> None:
                recover_publish(root, state_root / "publish-journal.json")
                if not stage.exists():
                    current_after_recovery = current_descriptor(export_root)
                    if identity_equal((current_after_recovery or {}).get("identity", current_after_recovery), identity_from(release, *assets, digests)):
                        return
                    fail("publish stage disappeared without a verified current")
                publish(release, config, assets, stage, digests)
            retry_call(publish_recoverable, "publish transaction", config)
            write_status(config, desiredIdentity=identity_from(release, *assets, digests), currentIdentity=current_descriptor(export_root), journalPhase="finalized")
        finally:
            if stage.exists():
                shutil.rmtree(stage, ignore_errors=True)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--config", default=DEFAULT_CONFIG)
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args()
    config: Dict[str, Any] | None = None
    try:
        config = validate_config(read_json(Path(args.config)))
        state = config["root"] / "state" / "status.json"
        previous = read_json(state) if state.exists() else {}
        write_status(config, schemaVersion=1, lastAttemptAt=now(), lastError=None, attemptCount=int(previous.get("attemptCount", 0)) + 1, nextRetryAt=None)
        sync(config, args.dry_run)
        write_status(config, schemaVersion=1, lastSuccessAt=now(), lastError=None, nextRetryAt=None)
        return 0
    except RelayError as exc:
        if config is not None:
            write_status(config, schemaVersion=1, lastError=str(exc), nextRetryAt=None, journalPhase="error")
        log(f"ERROR: {exc}")
        return 2
    except Exception as exc:
        if config is not None:
            write_status(config, schemaVersion=1, lastError=f"unexpected failure: {exc}", nextRetryAt=None, journalPhase="error")
        log(f"ERROR: unexpected failure: {exc}")
        return 3


if __name__ == "__main__":
    sys.exit(main())
