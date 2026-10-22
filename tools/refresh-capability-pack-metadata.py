"""Refresh integrity metadata for the product's embedded capability pack."""

from __future__ import annotations

import hashlib
import argparse
import json
from pathlib import Path


TEXT_EXTENSIONS = {
    ".cmd", ".env", ".in", ".json", ".lock", ".md", ".ps1", ".py",
    ".sha256", ".toml", ".txt", ".yaml", ".yml",
}
TRANSIENT_SEGMENTS = {".git", ".pytest_cache", "__pycache__"}
SOURCE_TREE_EXCLUDED = {"pack.json", "file-manifest.json", "checksums.sha256"}


def transient(relative: Path) -> bool:
    return any(part in TRANSIENT_SEGMENTS for part in relative.parts) or relative.suffix.lower() == ".pyc"


def canonical_bytes(path: Path) -> bytes:
    data = path.read_bytes()
    if path.suffix.lower() in TEXT_EXTENSIONS:
        return data.replace(b"\r\n", b"\n").replace(b"\r", b"\n")
    return data


def sha256(path: Path) -> str:
    return hashlib.sha256(canonical_bytes(path)).hexdigest()


def source_tree_sha256(root: Path) -> str:
    """Calculate the canonical package source identity without self-reference."""
    root = root.absolute()
    paths = sorted(
        (
            path for path in root.rglob("*")
            if path.is_file()
            and path.relative_to(root).as_posix() not in SOURCE_TREE_EXCLUDED
            and not transient(path.relative_to(root))
        ),
        key=lambda path: path.relative_to(root).as_posix().casefold(),
    )
    digest = hashlib.sha256()
    for path in paths:
        relative = path.relative_to(root).as_posix().encode("utf-8")
        digest.update(len(relative).to_bytes(4, "big"))
        digest.update(relative)
        digest.update(hashlib.sha256(canonical_bytes(path)).digest())
    return digest.hexdigest()


def sha256_bytes(path: Path, data: bytes) -> str:
    if path.suffix.lower() in TEXT_EXTENSIONS:
        data = data.replace(b"\r\n", b"\n").replace(b"\r", b"\n")
    return hashlib.sha256(data).hexdigest()


def build_expected(root: Path) -> tuple[dict, str, str, int]:
    # `resolve(strict=True)` can fail with WinError 5 in restricted Windows
    # sandboxes even when the current process can enumerate and write the pack.
    # An absolute lexical path is sufficient here; file existence and complete
    # manifest scope are validated immediately below.
    root = root.absolute()
    if not root.is_dir():
        raise NotADirectoryError(root)
    manifest_path = root / "file-manifest.json"
    checksum_path = root / "checksums.sha256"
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    entries = manifest.get("files")
    if not isinstance(entries, list):
        raise ValueError("file-manifest.json must contain a files array")

    listed: set[str] = set()
    for entry in entries:
        relative = Path(str(entry["path"]))
        if relative.is_absolute() or ".." in relative.parts:
            raise ValueError(f"unsafe manifest path: {relative}")
        target = root / relative
        if not target.is_file():
            raise FileNotFoundError(f"manifest file is missing: {relative}")
        key = relative.as_posix()
        if key in listed:
            raise ValueError(f"duplicate manifest path: {key}")
        listed.add(key)
        entry["sha256"] = sha256(target)
        # Store the same canonical byte length used by the SHA calculation.
        # .gitattributes may materialize text as CRLF on Windows while Git's
        # blob and the pack metadata are normalized to LF.
        entry["bytes"] = len(canonical_bytes(target))

    actual = {
        path.relative_to(root).as_posix()
        for path in root.rglob("*")
        if path.is_file()
        and path.name not in {"file-manifest.json", "checksums.sha256"}
        and not transient(path.relative_to(root))
    }
    if actual != listed:
        raise ValueError(
            f"manifest scope mismatch; unlisted={sorted(actual - listed)}; "
            f"missing={sorted(listed - actual)}"
        )

    manifest_text = json.dumps(manifest, ensure_ascii=False, indent=2) + "\n"
    manifest_bytes = manifest_text.encode("utf-8")
    manifest_hash = sha256_bytes(manifest_path, manifest_bytes)
    files = [("file-manifest.json", manifest_path)] + [
        (relative, root / relative) for relative in sorted(listed)
    ]
    checksum_text = "".join(
        f"{manifest_hash if relative == 'file-manifest.json' else sha256(path)} *{relative}\n"
        for relative, path in files
    )
    return manifest, manifest_text, checksum_text, len(listed)


def refresh(root: Path) -> dict[str, object]:
    root = root.absolute()
    pack_path = root / "pack.json"
    pack = json.loads(pack_path.read_text(encoding="utf-8"))
    pack["sourceTreeSha256"] = source_tree_sha256(root)
    pack_path.write_text(
        json.dumps(pack, ensure_ascii=False, indent=2) + "\n",
        encoding="utf-8",
        newline="\n",
    )
    manifest, manifest_text, checksum_text, manifest_count = build_expected(root)
    (root / "file-manifest.json").write_text(
        manifest_text, encoding="utf-8", newline="\n"
    )
    (root / "checksums.sha256").write_text(
        checksum_text, encoding="utf-8", newline="\n"
    )
    return {
        "manifestFiles": manifest_count,
        "checksumFiles": manifest_count + 1,
        "sourceTreeSha256": pack["sourceTreeSha256"],
    }


def check(root: Path) -> dict[str, object]:
    _, expected_manifest, expected_checksums, manifest_count = build_expected(root)
    root = root.absolute()
    pack = json.loads((root / "pack.json").read_text(encoding="utf-8"))
    expected_tree = source_tree_sha256(root)
    if pack.get("sourceTreeSha256") != expected_tree:
        raise ValueError(
            "capability-pack sourceTreeSha256 is stale: "
            f"expected {expected_tree}, got {pack.get('sourceTreeSha256')}"
        )
    manifest_path = root / "file-manifest.json"
    checksum_path = root / "checksums.sha256"
    mismatches = []
    if manifest_path.read_text(encoding="utf-8") != expected_manifest:
        mismatches.append("file-manifest.json")
    if checksum_path.read_text(encoding="utf-8") != expected_checksums:
        mismatches.append("checksums.sha256")
    if mismatches:
        raise ValueError(
            "capability-pack integrity metadata is stale: " + ", ".join(mismatches)
        )
    return {
        "manifestFiles": manifest_count,
        "checksumFiles": manifest_count + 1,
        "sourceTreeSha256": expected_tree,
    }


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("pack_root", nargs="?", type=Path,
                        default=Path(__file__).resolve().parents[1] / "installer-pack")
    parser.add_argument("--check", action="store_true",
                        help="validate metadata without changing any files")
    args = parser.parse_args()
    result = check(args.pack_root) if args.check else refresh(args.pack_root)
    print(json.dumps(result, ensure_ascii=False))
