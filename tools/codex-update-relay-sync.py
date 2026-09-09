#!/usr/bin/env python3
"""Fetch and atomically publish the current Windows Codex payload.

This process is intentionally a one-way GitHub fetcher. It does not expose a
proxy, does not access Docker, never removes the active published release, and
only prunes older relay releases after a complete new release is published.
The Nginx export is expected to read only the `export/current` symlink.
"""

from __future__ import annotations

import argparse
import fcntl
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
import urllib.error
import urllib.parse
import urllib.request
import uuid
from pathlib import Path
from typing import Any, Dict, Iterable, Tuple


DEFAULT_CONFIG = "/etc/codex-update-relay/config.json"
USER_AGENT = "codex-update-relay/1"
HEX64 = re.compile(r"^[0-9a-fA-F]{64}$")


class RelayError(RuntimeError):
    pass


def fail(message: str) -> None:
    raise RelayError(message)


def log(message: str) -> None:
    print(f"codex-update-relay: {message}", flush=True)


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

    root = Path(config.get("root", "/srv/codex-update-relay")).resolve()
    values = {
        "repository": repo,
        "architecture": architecture,
        "root": root,
        "min_free_bytes": int(config.get("min_free_bytes", 536870912)),
        "max_published_releases": int(config.get("max_published_releases", 2)),
        "download_timeout_seconds": int(config.get("download_timeout_seconds", 7200)),
    }
    if values["min_free_bytes"] < 268435456:
        fail("config.min_free_bytes must be at least 256 MiB")
    if values["max_published_releases"] < 1:
        fail("config.max_published_releases must be positive")
    if values["download_timeout_seconds"] < 60:
        fail("config.download_timeout_seconds is too small")
    return values


def require_https_github(url: str, expected_host: str) -> str:
    parsed = urllib.parse.urlparse(url)
    if parsed.scheme != "https" or parsed.hostname != expected_host or parsed.username or parsed.password:
        fail(f"unexpected GitHub URL: {url}")
    return url


def github_json(url: str) -> Dict[str, Any]:
    require_https_github(url, "api.github.com")
    request = urllib.request.Request(
        url,
        headers={
            "Accept": "application/vnd.github+json",
            "User-Agent": USER_AGENT,
        },
    )
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


def release_assets(release: Dict[str, Any]) -> Dict[str, Dict[str, Any]]:
    assets = release.get("assets")
    if not isinstance(assets, list):
        fail("release.assets is missing or invalid")
    result: Dict[str, Dict[str, Any]] = {}
    for asset in assets:
        if not isinstance(asset, dict):
            continue
        name = asset.get("name")
        url = asset.get("browser_download_url")
        size = asset.get("size")
        if not isinstance(name, str) or not isinstance(url, str) or not isinstance(size, int):
            continue
        if "/" in name or "\\" in name or size < 0:
            continue
        require_https_github(url, "github.com")
        result[name] = asset
    return result


def choose_assets(release: Dict[str, Any], architecture: str) -> Tuple[Dict[str, Any], Dict[str, Any], Dict[str, Any]]:
    assets = release_assets(release)
    msix_candidates = [
        asset for name, asset in assets.items()
        if name.lower().endswith(".msix")
        and re.search(rf"_{re.escape(architecture)}__", name, re.IGNORECASE)
    ]
    if len(msix_candidates) != 1:
        names = [asset.get("name", "?") for asset in msix_candidates]
        fail(f"expected exactly one {architecture} MSIX, got {names}")

    def required(name: str) -> Dict[str, Any]:
        value = assets.get(name)
        if value is None:
            fail(f"required release asset is missing: {name}")
        return value

    return msix_candidates[0], required("release-manifest.json"), required("SHA256SUMS-windows.txt")


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        while True:
            chunk = handle.read(1024 * 1024)
            if not chunk:
                break
            digest.update(chunk)
    return digest.hexdigest()


def download_asset(asset: Dict[str, Any], destination: Path, timeout_seconds: int) -> None:
    url = require_https_github(str(asset["browser_download_url"]), "github.com")
    destination.parent.mkdir(parents=True, exist_ok=True)
    command = [
        "/usr/bin/curl",
        "--fail",
        "--silent",
        "--show-error",
        "--location",
        "--proto",
        "=https",
        "--retry",
        "3",
        "--retry-delay",
        "2",
        "--connect-timeout",
        "15",
        "--max-time",
        str(timeout_seconds),
        "--user-agent",
        USER_AGENT,
        "--header",
        "Accept: application/octet-stream",
        "--continue-at",
        "-",
        "--output",
        str(destination),
        url,
    ]
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
        name = Path(match.group(2)).name
        if name in result and result[name] != match.group(1).lower():
            fail(f"duplicate checksum with conflicting values: {name}")
        result[name] = match.group(1).lower()
    return result


def verify_download(path: Path, asset: Dict[str, Any], checksums: Dict[str, str] | None = None) -> str:
    name = str(asset["name"])
    expected_size = int(asset["size"])
    actual_size = path.stat().st_size
    if actual_size != expected_size:
        fail(f"size mismatch for {name}: expected {expected_size}, got {actual_size}")
    digest = sha256_file(path)
    github_digest = asset.get("digest")
    if isinstance(github_digest, str) and github_digest.startswith("sha256:"):
        expected_digest = github_digest[7:].lower()
        if not HEX64.fullmatch(expected_digest) or digest != expected_digest:
            fail(f"GitHub digest mismatch for {name}")
    if checksums is not None:
        expected_checksum = checksums.get(name)
        if expected_checksum is None or digest != expected_checksum:
            fail(f"SHA256SUMS-windows.txt mismatch for {name}")
    return digest


def write_json_atomic(path: Path, value: Dict[str, Any]) -> None:
    temporary = path.with_name(f".{path.name}.{uuid.uuid4().hex}.tmp")
    encoded = json.dumps(value, ensure_ascii=True, indent=2, sort_keys=True) + "\n"
    with temporary.open("w", encoding="utf-8") as handle:
        handle.write(encoded)
        handle.flush()
        os.fsync(handle.fileno())
    os.replace(temporary, path)


def current_release_id(export_root: Path) -> str | None:
    latest = export_root / "current" / "latest.json"
    if not latest.exists():
        return None
    try:
        value = read_json(latest)
    except RelayError:
        return None
    release_id = value.get("releaseId")
    return str(release_id) if release_id is not None else None


def published_release_count(releases_root: Path) -> int:
    if not releases_root.exists():
        return 0
    return sum(1 for item in releases_root.iterdir() if item.is_dir() and not item.is_symlink())


def published_release_bytes(releases_root: Path) -> int:
    """Return bytes held by real published release directories only."""
    if not releases_root.exists():
        return 0
    total = 0
    for release in releases_root.iterdir():
        if release.is_symlink() or not release.is_dir():
            continue
        for item in release.rglob("*"):
            if item.is_symlink() or not item.is_file():
                continue
            total += item.stat().st_size
    return total


def prune_published_releases(releases_root: Path, current_id: str, keep: int) -> None:
    """Prune oldest numeric releases after current has switched atomically."""
    candidates = [
        item
        for item in releases_root.iterdir()
        if item.is_dir() and not item.is_symlink() and item.name.isdigit()
    ]
    candidates.sort(key=lambda item: int(item.name))
    removable = [item for item in candidates if item.name != current_id]
    while len(candidates) > keep and removable:
        target = removable.pop(0)
        shutil.rmtree(target)
        candidates.remove(target)
        log(f"pruned old release {target.name}")


def publish(
    release: Dict[str, Any],
    config: Dict[str, Any],
    msix: Dict[str, Any],
    manifest: Dict[str, Any],
    checksums_asset: Dict[str, Any],
    stage: Path,
    msix_digest: str,
    manifest_digest: str,
    checksums_digest: str,
) -> None:
    root = config["root"]
    export_root = root / "export"
    releases_root = export_root / "releases"
    releases_root.mkdir(parents=True, exist_ok=True)
    release_id = str(release.get("id"))
    if not release_id or release_id == "None" or not re.fullmatch(r"[0-9]+", release_id):
        fail("release.id is missing or invalid")
    final_release = releases_root / release_id
    if final_release.exists():
        fail(f"release directory already exists and is not overwritten: {final_release}")
    descriptor = {
        "protocol": 1,
        "repository": config["repository"],
        "releaseId": int(release_id),
        "tag": release.get("tag_name"),
        "publishedAt": release.get("published_at"),
        "architecture": config["architecture"],
        "assets": {
            "windows-msix": {
                "path": msix["name"],
                "size": int(msix["size"]),
                "sha256": msix_digest,
                "githubDigest": msix.get("digest"),
            },
            "release-manifest": {
                "path": manifest["name"],
                "size": int(manifest["size"]),
                "sha256": manifest_digest,
            },
            "checksums": {
                "path": checksums_asset["name"],
                "size": int(checksums_asset["size"]),
                "sha256": checksums_digest,
            },
        },
    }
    write_json_atomic(stage / "latest.json", descriptor)
    for item in stage.iterdir():
        if item.is_file():
            item.chmod(0o644)
    # tempfile.mkdtemp creates the release directory as 0700.  The edge
    # Nginx worker is intentionally unprivileged, so publish the directory as
    # read/execute for traversal while keeping all writes owner-only.
    stage.chmod(0o755)
    os.replace(stage, final_release)
    next_link = export_root / f".current-{uuid.uuid4().hex}"
    os.symlink(os.path.join("releases", release_id), next_link, target_is_directory=True)
    os.replace(next_link, export_root / "current")
    log(f"published release {release_id} ({release.get('tag_name')})")
    prune_published_releases(
        releases_root,
        current_id=release_id,
        keep=config["max_published_releases"],
    )


def sync(config: Dict[str, Any], dry_run: bool) -> None:
    root: Path = config["root"]
    export_root = root / "export"
    work_root = root / "work"
    state_root = root / "state"
    for directory in (export_root, work_root, state_root):
        directory.mkdir(parents=True, exist_ok=True)

    lock_path = state_root / "sync.lock"
    with lock_path.open("a+") as lock_handle:
        try:
            fcntl.flock(lock_handle.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            fail("another sync is already running")

        api_url = f"https://api.github.com/repos/{config['repository']}/releases/latest"
        release = github_json(api_url)
        if release.get("draft") or release.get("prerelease"):
            fail("latest GitHub release is draft or prerelease")
        msix, manifest, checksums_asset = choose_assets(release, config["architecture"])
        release_id = str(release.get("id"))
        if current_release_id(export_root) == release_id:
            log(f"already current: {release.get('tag_name')}")
            return

        expected_bytes = int(msix["size"]) + int(manifest["size"]) + int(checksums_asset["size"])
        free_bytes = shutil.disk_usage(root).free
        retained_bytes = published_release_bytes(export_root / "releases")
        required_bytes = expected_bytes + retained_bytes + config["min_free_bytes"]
        if free_bytes < required_bytes:
            fail(
                f"insufficient free space: need {required_bytes} "
                f"(download={expected_bytes}, retained={retained_bytes}, "
                f"safety={config['min_free_bytes']}), have {free_bytes}"
            )

        if dry_run:
            log(
                f"dry-run: release={release.get('tag_name')} id={release_id} "
                f"msix={msix.get('name')} size={msix.get('size')} free={free_bytes}"
            )
            return

        stage = Path(tempfile.mkdtemp(prefix=f"release-{release_id}-", dir=str(work_root)))
        try:
            download_asset(msix, stage / msix["name"], config["download_timeout_seconds"])
            download_asset(manifest, stage / manifest["name"], config["download_timeout_seconds"])
            download_asset(checksums_asset, stage / checksums_asset["name"], config["download_timeout_seconds"])
            checksums = parse_checksums(stage / checksums_asset["name"])
            msix_digest = verify_download(stage / msix["name"], msix, checksums)
            manifest_digest = verify_download(stage / manifest["name"], manifest)
            checksums_digest = verify_download(stage / checksums_asset["name"], checksums_asset)
            manifest_value = read_json(stage / manifest["name"])
            if not manifest_value:
                fail("release-manifest.json is empty")
            publish(
                release,
                config,
                msix,
                manifest,
                checksums_asset,
                stage,
                msix_digest,
                manifest_digest,
                checksums_digest,
            )
        except Exception:
            shutil.rmtree(stage, ignore_errors=True)
            raise


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--config", default=DEFAULT_CONFIG)
    parser.add_argument("--dry-run", action="store_true")
    args = parser.parse_args()
    try:
        config = validate_config(read_json(Path(args.config)))
        sync(config, args.dry_run)
        return 0
    except RelayError as exc:
        log(f"ERROR: {exc}")
        return 2
    except Exception as exc:
        log(f"ERROR: unexpected failure: {exc}")
        return 3


if __name__ == "__main__":
    sys.exit(main())
