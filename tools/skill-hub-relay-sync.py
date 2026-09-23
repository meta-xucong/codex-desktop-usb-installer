#!/usr/bin/env python3
"""Fetch and atomically publish the unsigned Skill Hub archive.

The relay is a static cache only.  It never executes repository content and it
publishes a release directory before switching the ``current`` symlink.
"""
from __future__ import annotations

import argparse
import fcntl
import hashlib
import json
import os
import re
import shutil
import tempfile
import urllib.error
import urllib.parse
import urllib.request
import uuid
from pathlib import Path
from typing import Any

EXPECTED_REPOSITORY = "meta-xucong/skill_hub"
EXPECTED_BRANCH = "master"
EXPECTED_GITHUB_COMMIT_URL = "https://api.github.com/repos/meta-xucong/skill_hub/commits/master"
EXPECTED_GITHUB_ARCHIVE_TEMPLATE = "https://github.com/meta-xucong/skill_hub/archive/{commit}.zip"
EXPECTED_EXPORT_ROOT = "/srv/codex-update-relay/export"
PAYLOAD_CURRENT = Path("/srv/codex-update-relay/export/current")
ALLOWED_GITHUB_HOSTS = frozenset({"api.github.com", "github.com", "codeload.github.com"})
SHA_RE = re.compile(r"^[0-9a-f]{64}$")
COMMIT_RE = re.compile(r"^[0-9a-f]{40}$")
MAX_ARCHIVE = 64 * 1024 * 1024
MAX_METADATA = 64 * 1024
USER_AGENT = "codex-skill-hub-relay/1"
METADATA_KEYS = frozenset(("schemaVersion", "repo", "branch", "commit", "archive", "size", "sha256"))


class RelayError(RuntimeError):
    pass


class LimitedRedirectHandler(urllib.request.HTTPRedirectHandler):
    def __init__(self, limit: int = 3) -> None:
        super().__init__()
        self.limit = limit
        self.count = 0

    def redirect_request(self, req: Any, fp: Any, code: int, msg: str, headers: Any, newurl: str) -> Any:
        self.count += 1
        if self.count > self.limit:
            raise RelayError("too many redirects")
        validate_github_url(newurl)
        return super().redirect_request(req, fp, code, msg, headers, newurl)


def read_json(path: Path) -> dict[str, Any]:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeDecodeError, json.JSONDecodeError) as exc:
        raise RelayError(f"invalid JSON {path}: {exc}") from exc
    if not isinstance(value, dict):
        raise RelayError("JSON root must be an object")
    return value


def validate_config(value: dict[str, Any]) -> dict[str, Any]:
    required = ("repository", "branch", "githubCommitUrl", "githubArchiveTemplate", "exportRoot", "workRoot", "stateRoot", "lockFile", "maxPublishedReleases")
    if any(key not in value for key in required):
        raise RelayError("config is missing a required field")
    repo = value["repository"]
    branch = value["branch"]
    if repo != EXPECTED_REPOSITORY:
        raise RelayError("repository is fixed to Skill Hub")
    if branch != EXPECTED_BRANCH:
        raise RelayError("branch is fixed to master")
    if value["githubCommitUrl"] != EXPECTED_GITHUB_COMMIT_URL:
        raise RelayError("githubCommitUrl is fixed to the master commit API")
    if value["githubArchiveTemplate"] != EXPECTED_GITHUB_ARCHIVE_TEMPLATE:
        raise RelayError("githubArchiveTemplate is fixed to commit archives")
    if value["exportRoot"] != EXPECTED_EXPORT_ROOT:
        raise RelayError("exportRoot is fixed to the relay export root")
    if not all(isinstance(value[key], str) for key in ("repository", "branch", "githubCommitUrl", "githubArchiveTemplate", "exportRoot", "workRoot", "stateRoot", "lockFile")):
        raise RelayError("config fields have invalid types")

    def resolve_absolute(key: str) -> Path:
        raw = value[key]
        if not raw or not Path(raw).is_absolute():
            raise RelayError(f"{key} must be an absolute path")
        try:
            return Path(raw).resolve()
        except (OSError, RuntimeError) as exc:
            raise RelayError(f"cannot resolve {key}: {exc}") from exc

    roots = {key: resolve_absolute(key) for key in ("exportRoot", "workRoot", "stateRoot")}
    lock = resolve_absolute("lockFile")
    skill_root = roots["exportRoot"] / "skill-hub"
    # Nginx serves this tree from a separate container user.  It is public
    # static content, so its directories must remain traversable by the edge.
    skill_root.mkdir(parents=True, exist_ok=True)
    skill_root.chmod(0o755)
    if lock == roots["stateRoot"]:
        raise RelayError("lockFile must be strictly below stateRoot")
    try:
        lock.relative_to(roots["stateRoot"])
    except ValueError as exc:
        raise RelayError("lockFile must be strictly below stateRoot") from exc

    configured_roots = (roots["exportRoot"], roots["workRoot"], roots["stateRoot"])
    if any(paths_overlap(left, right) for index, left in enumerate(configured_roots) for right in configured_roots[index + 1:]):
        raise RelayError("relay roots must not overlap")
    mutable_paths = {
        "skillRoot": skill_root,
        "workRoot": roots["workRoot"],
        "stateRoot": roots["stateRoot"],
        "lockFile": lock,
    }
    names = tuple(mutable_paths)
    if any(paths_overlap(mutable_paths[left], mutable_paths[right]) for index, left in enumerate(names) for right in names[index + 1:] if {left, right} != {"stateRoot", "lockFile"}):
        raise RelayError("relay write paths must not overlap")
    payload_current = Path(os.path.abspath(PAYLOAD_CURRENT))
    if any(paths_overlap(path, payload_current) for path in mutable_paths.values()):
        raise RelayError("relay path overlaps Codex payload current")

    keep = value["maxPublishedReleases"]
    if isinstance(keep, bool) or not isinstance(keep, int) or keep < 2:
        raise RelayError("maxPublishedReleases must be at least 2")
    return {
        "repo": repo,
        "branch": branch,
        "githubCommitUrl": value["githubCommitUrl"],
        "githubArchiveTemplate": value["githubArchiveTemplate"],
        **roots,
        "lockFile": lock,
        "maxPublishedReleases": keep,
    }


def paths_overlap(left: Path, right: Path) -> bool:
    return left == right or left in right.parents or right in left.parents


def validate_github_url(url: str) -> None:
    try:
        parsed = urllib.parse.urlsplit(url)
        port = parsed.port
    except (TypeError, ValueError) as exc:
        raise RelayError("invalid GitHub URL") from exc
    if (
        parsed.scheme != "https"
        or parsed.hostname not in ALLOWED_GITHUB_HOSTS
        or parsed.username is not None
        or parsed.password is not None
        or port not in (None, 443)
    ):
        raise RelayError("GitHub URLs and redirects must stay on HTTPS GitHub hosts")


def fetch(url: str, max_bytes: int) -> bytes:
    validate_github_url(url)
    handler = LimitedRedirectHandler(3)
    opener = urllib.request.build_opener(handler)
    request = urllib.request.Request(url, headers={"Accept": "application/json", "User-Agent": USER_AGENT})
    try:
        with opener.open(request, timeout=30) as response:
            validate_github_url(response.geturl())
            content_length = response.headers.get("Content-Length")
            if content_length is not None:
                try:
                    if int(content_length) > max_bytes:
                        raise RelayError("response exceeds configured limit")
                except ValueError as exc:
                    raise RelayError("invalid response length") from exc
            data = response.read(max_bytes + 1)
    except (OSError, urllib.error.URLError, urllib.error.HTTPError, RelayError) as exc:
        raise RelayError(f"download failed: {exc}") from exc
    if len(data) > max_bytes:
        raise RelayError("response exceeds configured limit")
    return data


def atomic_json(path: Path, value: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    temp = path.with_name(f".{path.name}.{uuid.uuid4().hex}.tmp")
    temp.write_text(json.dumps(value, ensure_ascii=True, sort_keys=True, indent=2) + "\n", encoding="utf-8")
    temp.chmod(0o644)
    with temp.open("rb") as handle:
        os.fsync(handle.fileno())
    os.replace(temp, path)


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def metadata(config: dict[str, Any], commit: str, archive: bytes) -> dict[str, Any]:
    return {
        "schemaVersion": 1,
        "repo": config["repo"],
        "branch": config["branch"],
        "commit": commit,
        "archive": "archive.zip",
        "size": len(archive),
        "sha256": sha256(archive),
    }


def validate_metadata(value: dict[str, Any], expected: dict[str, Any]) -> None:
    if value != expected:
        raise RelayError("published metadata does not match the downloaded archive")


def current_commit(export_root: Path, config: dict[str, Any]) -> str | None:
    current = export_root / "current"
    latest = current / "latest.json"
    if not current.is_symlink() or not latest.is_file():
        return None
    try:
        link_target = Path(os.readlink(current))
    except OSError:
        return None
    if link_target.is_absolute() or link_target.parent != Path("releases"):
        return None
    try:
        value = read_json(latest)
    except RelayError:
        return None
    commit = value.get("commit")
    size = value.get("size")
    digest = value.get("sha256")
    if (
        set(value) != METADATA_KEYS
        or value.get("schemaVersion") != 1
        or value.get("repo") != config["repo"]
        or value.get("branch") != config["branch"]
        or value.get("archive") != "archive.zip"
        or not isinstance(commit, str)
        or COMMIT_RE.fullmatch(commit) is None
        or isinstance(size, bool)
        or not isinstance(size, int)
        or size < 0
        or size > MAX_ARCHIVE
        or not isinstance(digest, str)
        or SHA_RE.fullmatch(digest) is None
        or link_target.name != commit
    ):
        return None
    archive_path = current / "archive.zip"
    if archive_path.is_symlink() or not archive_path.is_file():
        return None
    try:
        archive = archive_path.read_bytes()
    except OSError:
        return None
    if len(archive) != size or sha256(archive) != digest:
        return None
    return commit


def validate_release(release: Path, expected: dict[str, Any]) -> None:
    if release.is_symlink() or not release.is_dir():
        raise RelayError("published release is not a real directory")
    validate_metadata(read_json(release / "latest.json"), expected)
    archive_path = release / "archive.zip"
    if archive_path.is_symlink() or not archive_path.is_file():
        raise RelayError("published archive is missing")
    archive = archive_path.read_bytes()
    if len(archive) != expected["size"] or sha256(archive) != expected["sha256"]:
        raise RelayError("published archive failed verification")


def publish_current(export_root: Path, commit: str) -> None:
    current_tmp = export_root / f".current.{uuid.uuid4().hex}"
    try:
        current_tmp.symlink_to(Path("releases") / commit, target_is_directory=True)
        os.replace(current_tmp, export_root / "current")
    finally:
        if current_tmp.is_symlink() or current_tmp.exists():
            current_tmp.unlink()


def prune(releases: Path, current: str, keep: int) -> None:
    candidates = sorted((item for item in releases.iterdir() if item.is_dir() and not item.is_symlink()), key=lambda item: item.name)
    for item in candidates[:-keep]:
        if item.name != current:
            shutil.rmtree(item)


def sync(config_path: Path) -> None:
    config = validate_config(read_json(config_path))
    for path in (config["exportRoot"], config["workRoot"], config["stateRoot"]):
        path.mkdir(parents=True, exist_ok=True)
    lock_path: Path = config["lockFile"]
    lock_path.parent.mkdir(parents=True, exist_ok=True)
    with lock_path.open("a+") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        try:
            commit_value = json.loads(fetch(config["githubCommitUrl"], MAX_METADATA).decode("utf-8"))
            if not isinstance(commit_value, dict):
                raise RelayError("GitHub commit response must be an object")
            commit = str(commit_value.get("sha", "")).lower()
            if not COMMIT_RE.fullmatch(commit):
                raise RelayError("GitHub commit response is invalid")
            export_root = config["exportRoot"] / "skill-hub"
            releases = export_root / "releases"
            releases.mkdir(parents=True, exist_ok=True)
            releases.chmod(0o755)
            if current_commit(export_root, config) == commit:
                return
            archive_url = config["githubArchiveTemplate"].format(commit=commit)
            archive = fetch(archive_url, MAX_ARCHIVE)
            expected = metadata(config, commit, archive)
            release = releases / commit
            if release.exists():
                validate_release(release, expected)
            else:
                # The work and export roots may be different mounts on a relay host.
                # Stage the final release beside `releases/` so the publish rename is
                # always same-filesystem and remains atomic with respect to `current`.
                temporary = Path(tempfile.mkdtemp(prefix=f".{commit}.", dir=releases))
                try:
                    archive_path = temporary / "archive.zip"
                    archive_path.write_bytes(archive)
                    archive_path.chmod(0o644)
                    if archive_path.stat().st_size != expected["size"] or sha256(archive_path.read_bytes()) != expected["sha256"]:
                        raise RelayError("staged archive failed verification")
                    atomic_json(temporary / "latest.json", expected)
                    validate_metadata(read_json(temporary / "latest.json"), expected)
                    temporary.chmod(0o755)
                    os.replace(temporary, release)
                finally:
                    if temporary.exists():
                        shutil.rmtree(temporary)
            publish_current(export_root, commit)
            prune(releases, commit, config["maxPublishedReleases"])
        finally:
            fcntl.flock(lock, fcntl.LOCK_UN)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--config", default="/etc/codex-update-relay/skill-hub.json")
    args = parser.parse_args()
    try:
        sync(Path(args.config))
    except (RelayError, OSError, ValueError) as exc:
        print(f"skill-hub-relay: {exc}")
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
