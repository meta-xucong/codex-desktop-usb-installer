"""Build and verify the self-contained Codex capability pack.

The pack is already laid out in this directory.  This tool deliberately does
not fetch anything and does not execute installer code: it regenerates the
manifest/checksum metadata and verifies the resulting closure.  Text files
are hashed after CRLF/CR -> LF canonicalisation, matching the native manager
and the compatibility audit.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import subprocess
import sys
from pathlib import Path, PurePosixPath


TEXT_SUFFIXES = {
    ".cmd",
    ".env",
    ".in",
    ".json",
    ".lock",
    ".md",
    ".ps1",
    ".py",
    ".sha256",
    ".toml",
    ".txt",
    ".yaml",
    ".yml",
}
GENERATED_FILES = {"file-manifest.json", "checksums.sha256"}
TRANSIENT_DIRS = {".git", ".pytest_cache", "__pycache__"}


def fail(message: str) -> "NoReturn":
    raise SystemExit(f"build-pack: ERROR: {message}")


def canonical_bytes(path: Path) -> bytes:
    data = path.read_bytes()
    if path.suffix.lower() not in TEXT_SUFFIXES:
        return data
    return data.replace(b"\r\n", b"\n").replace(b"\r", b"\n")


def rel_path(root: Path, path: Path) -> str:
    return path.relative_to(root).as_posix()


def iter_pack_files(root: Path):
    for path in sorted(root.rglob("*"), key=lambda item: item.as_posix().casefold()):
        if not path.is_file():
            continue
        relative = rel_path(root, path)
        parts = set(PurePosixPath(relative).parts)
        if parts & TRANSIENT_DIRS or relative.endswith(".pyc"):
            continue
        yield relative, path


def sha256_hex(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def source_tree_hash(root: Path) -> str:
    """Hash source inputs without introducing a pack.json self-reference."""
    digest = hashlib.sha256()
    excluded = GENERATED_FILES | {"pack.json"}
    for relative, path in iter_pack_files(root):
        if relative in excluded:
            continue
        name = relative.encode("utf-8")
        content_digest = hashlib.sha256(canonical_bytes(path)).digest()
        digest.update(len(name).to_bytes(4, "big"))
        digest.update(name)
        digest.update(content_digest)
    return digest.hexdigest()


def read_json(path: Path) -> dict:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        fail(f"cannot read {path.name}: {error}")
    if not isinstance(value, dict):
        fail(f"{path.name} must contain a JSON object")
    return value


def git_source(root: Path) -> tuple[str, str]:
    try:
        completed = subprocess.run(
            ["git", "-C", str(root.parent), "rev-parse", "HEAD"],
            check=True,
            capture_output=True,
            text=True,
        )
        commit = completed.stdout.strip().lower()
        if len(commit) == 40 and all(character in "0123456789abcdef" for character in commit):
            return commit, "build-input"
    except (OSError, subprocess.CalledProcessError):
        pass
    source_file = root / "manifest" / "source-commit.txt"
    if source_file.is_file():
        commit = source_file.read_text(encoding="utf-8").strip().lower()
        if len(commit) == 40 and all(character in "0123456789abcdef" for character in commit):
            return commit, "upstream-base"
    fail("no 40-character Git source commit or manifest/source-commit.txt")


def manifest_entries(root: Path) -> list[dict]:
    entries = []
    for relative, path in iter_pack_files(root):
        if relative in GENERATED_FILES:
            continue
        entries.append(
            {
                "path": relative,
                "sha256": sha256_hex(canonical_bytes(path)),
                "bytes": path.stat().st_size,
                "owned": True,
            }
        )
    return entries


def write_json(path: Path, value: dict) -> None:
    path.write_text(json.dumps(value, ensure_ascii=False, indent=2) + "\n", encoding="utf-8", newline="\n")


def write_generated_metadata(root: Path) -> None:
    pack_path = root / "pack.json"
    pack = read_json(pack_path)
    source_commit, source_role = git_source(root)
    pack["sourceCommit"] = source_commit
    pack["sourceCommitRole"] = source_role
    pack["sourceTreeSha256"] = source_tree_hash(root)
    datetime_module = __import__("datetime")
    pack["sourceDate"] = datetime_module.datetime.now(datetime_module.timezone.utc).date().isoformat()
    write_json(pack_path, pack)

    entries = manifest_entries(root)
    manifest = {
        "schemaVersion": 2,
        "id": str(pack.get("id", "codex-starter")),
        "version": str(pack.get("version", "0.0.0")),
        "hashScope": "all pack files except file-manifest.json and checksums.sha256; text files use LF canonicalization",
        "files": entries,
    }
    manifest_path = root / "file-manifest.json"
    write_json(manifest_path, manifest)

    checksum_lines = []
    checksum_lines.append(f"{sha256_hex(canonical_bytes(manifest_path))} *file-manifest.json")
    for entry in entries:
        checksum_lines.append(f"{entry['sha256']} *{entry['path']}")
    (root / "checksums.sha256").write_text("\n".join(checksum_lines) + "\n", encoding="utf-8", newline="\n")


def load_manifest(root: Path) -> tuple[dict, dict, dict]:
    pack = read_json(root / "pack.json")
    manifest = read_json(root / "file-manifest.json")
    checksums_path = root / "checksums.sha256"
    if not checksums_path.is_file():
        fail("checksums.sha256 is missing")
    checksums = {}
    for line_number, line in enumerate(checksums_path.read_text(encoding="utf-8").splitlines(), 1):
        if not line:
            continue
        if len(line) < 66 or line[64:66] != " *":
            fail(f"checksums.sha256 line {line_number} is malformed")
        digest, relative = line[:64], line[66:]
        if len(digest) != 64 or any(character not in "0123456789abcdef" for character in digest):
            fail(f"checksums.sha256 line {line_number} has an invalid digest")
        if not relative or relative in checksums:
            fail(f"checksums.sha256 line {line_number} has a duplicate/empty path")
        checksums[relative] = digest
    return pack, manifest, checksums


def verify(root: Path) -> None:
    pack, manifest, checksums = load_manifest(root)
    files = manifest.get("files")
    if manifest.get("schemaVersion") != 2 or not isinstance(files, list):
        fail("file-manifest.json schema is invalid")
    listed = {}
    for entry in files:
        relative = str(entry.get("path", ""))
        if not relative or relative in GENERATED_FILES or relative in listed:
            fail(f"invalid or duplicate manifest path: {relative}")
        if PurePosixPath(relative).is_absolute() or ".." in PurePosixPath(relative).parts:
            fail(f"manifest path escapes package: {relative}")
        path = root / Path(*PurePosixPath(relative).parts)
        if not path.is_file():
            fail(f"manifest target is missing: {relative}")
        actual = sha256_hex(canonical_bytes(path))
        if actual != entry.get("sha256"):
            fail(f"manifest hash mismatch: {relative}")
        if path.stat().st_size != entry.get("bytes"):
            fail(f"manifest byte-size mismatch: {relative}")
        listed[relative] = actual

    actual_files = {
        relative: path
        for relative, path in iter_pack_files(root)
        if relative not in GENERATED_FILES
    }
    if set(actual_files) != set(listed):
        fail(f"manifest closure mismatch; extra={sorted(set(actual_files) - set(listed))}, missing={sorted(set(listed) - set(actual_files))}")

    if set(checksums) != {"file-manifest.json", *listed.keys()}:
        fail("checksums.sha256 does not cover exactly the package closure")
    manifest_hash = sha256_hex(canonical_bytes(root / "file-manifest.json"))
    if checksums.get("file-manifest.json") != manifest_hash:
        fail("checksums.sha256 file-manifest hash mismatch")
    for relative, digest in listed.items():
        if checksums.get(relative) != digest:
            fail(f"checksums.sha256 hash mismatch: {relative}")

    expected_tree = source_tree_hash(root)
    if pack.get("sourceTreeSha256") != expected_tree:
        fail(f"pack.json sourceTreeSha256 mismatch: expected {expected_tree}, got {pack.get('sourceTreeSha256')}")
    source_commit = str(pack.get("sourceCommit", ""))
    if len(source_commit) != 40 or any(character not in "0123456789abcdef" for character in source_commit.lower()):
        fail("pack.json sourceCommit is not a 40-character hexadecimal commit")
    if pack.get("sourceCommitRole") not in {"build-input", "upstream-base"}:
        fail("pack.json sourceCommitRole is invalid")
    print(f"build-pack: verified {len(listed)} files; sourceTreeSha256={expected_tree}")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true", help="regenerate package metadata")
    parser.add_argument("--verify", action="store_true", help="verify package metadata without writing")
    args = parser.parse_args()
    if args.check and args.verify:
        fail("--check and --verify are mutually exclusive")
    root = Path(__file__).resolve().parents[1]
    if args.check:
        write_generated_metadata(root)
        verify(root)
        print("build-pack: metadata regenerated")
        return 0
    verify(root)
    return 0


if __name__ == "__main__":
    main()
