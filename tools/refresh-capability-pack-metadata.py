"""Refresh integrity metadata for the product's embedded capability pack."""

from __future__ import annotations

import hashlib
import json
import sys
from pathlib import Path


TEXT_EXTENSIONS = {
    ".cmd", ".env", ".in", ".json", ".lock", ".md", ".ps1", ".py",
    ".sha256", ".toml", ".txt", ".yaml", ".yml",
}
TRANSIENT_SEGMENTS = {".git", ".pytest_cache", "__pycache__"}


def transient(relative: Path) -> bool:
    return any(part in TRANSIENT_SEGMENTS for part in relative.parts) or relative.suffix.lower() == ".pyc"


def canonical_bytes(path: Path) -> bytes:
    data = path.read_bytes()
    if path.suffix.lower() in TEXT_EXTENSIONS:
        return data.replace(b"\r\n", b"\n").replace(b"\r", b"\n")
    return data


def sha256(path: Path) -> str:
    return hashlib.sha256(canonical_bytes(path)).hexdigest()


def refresh(root: Path) -> dict[str, int]:
    root = root.resolve(strict=True)
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
        entry["bytes"] = target.stat().st_size

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

    manifest_path.write_text(
        json.dumps(manifest, ensure_ascii=False, indent=2) + "\n",
        encoding="utf-8",
        newline="\n",
    )
    files = [("file-manifest.json", manifest_path)] + [
        (relative, root / relative) for relative in sorted(listed)
    ]
    checksum_path.write_text(
        "".join(f"{sha256(path)} *{relative}\n" for relative, path in files),
        encoding="utf-8",
        newline="\n",
    )
    return {"manifestFiles": len(listed), "checksumFiles": len(files)}


if __name__ == "__main__":
    pack_root = Path(sys.argv[1]) if len(sys.argv) > 1 else Path(__file__).resolve().parents[1] / "installer-pack"
    print(json.dumps(refresh(pack_root), ensure_ascii=False))
