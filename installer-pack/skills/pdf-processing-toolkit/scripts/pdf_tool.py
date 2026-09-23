"""Small offline PDF helper used by the bundled Codex PDF Skill.

The script deliberately fails closed when one of the pinned dependencies is
missing. It never contacts a network and never claims a file was written
unless the output exists and can be opened again.
"""

from __future__ import annotations

import argparse
import sys
import tempfile
from pathlib import Path


def _imports():
    try:
        from pypdf import PdfReader, PdfWriter
        import pdfplumber
        from reportlab.pdfgen import canvas
    except Exception as exc:  # pragma: no cover - exercised by installer health probe
        raise RuntimeError(
            "PDF runtime unavailable; install bundled pypdf, pdfplumber, and reportlab "
            f"before using this Skill ({exc})"
        ) from exc
    return PdfReader, PdfWriter, pdfplumber, canvas


def _required_file(path: str) -> Path:
    value = Path(path).expanduser()
    if not value.is_file():
        raise FileNotFoundError(f"PDF input does not exist: {value}")
    return value


def info(path: str) -> int:
    PdfReader, _, _, _ = _imports()
    pdf = _required_file(path)
    reader = PdfReader(str(pdf))
    print({"path": str(pdf), "pages": len(reader.pages), "encrypted": bool(reader.is_encrypted)})
    return 0


def extract(path: str, output: str | None) -> int:
    _, _, pdfplumber, _ = _imports()
    pdf = _required_file(path)
    chunks: list[str] = []
    with pdfplumber.open(str(pdf)) as document:
        for page in document.pages:
            chunks.append(page.extract_text() or "")
    text = "\n\n".join(chunks)
    if output:
        destination = Path(output).expanduser()
        destination.parent.mkdir(parents=True, exist_ok=True)
        destination.write_text(text, encoding="utf-8")
        if not destination.is_file():
            raise IOError(f"PDF text output was not created: {destination}")
    else:
        print(text)
    return 0


def merge(output: str, inputs: list[str]) -> int:
    PdfReader, PdfWriter, _, _ = _imports()
    if not inputs:
        raise ValueError("merge requires at least one input PDF")
    writer = PdfWriter()
    for item in inputs:
        reader = PdfReader(str(_required_file(item)))
        for page in reader.pages:
            writer.add_page(page)
    destination = Path(output).expanduser()
    destination.parent.mkdir(parents=True, exist_ok=True)
    with destination.open("wb") as handle:
        writer.write(handle)
    if not destination.is_file() or destination.stat().st_size == 0:
        raise IOError(f"Merged PDF was not created: {destination}")
    return 0


def generate(output: str, text: str) -> int:
    _, _, _, canvas = _imports()
    destination = Path(output).expanduser()
    destination.parent.mkdir(parents=True, exist_ok=True)
    document = canvas.Canvas(str(destination))
    document.drawString(72, 720, text)
    document.save()
    if not destination.is_file() or destination.stat().st_size == 0:
        raise IOError(f"Generated PDF was not created: {destination}")
    return 0


def smoke() -> int:
    PdfReader, _, pdfplumber, _ = _imports()
    with tempfile.TemporaryDirectory(prefix="codex-pdf-smoke-") as directory:
        root = Path(directory)
        generated = root / "generated.pdf"
        merged = root / "merged.pdf"
        generate(str(generated), "codex pdf smoke")
        reader = PdfReader(str(generated))
        if len(reader.pages) != 1:
            raise AssertionError("generated PDF did not contain one page")
        with pdfplumber.open(str(generated)) as document:
            extracted = "\n".join(page.extract_text() or "" for page in document.pages)
        if "codex pdf smoke" not in extracted:
            raise AssertionError("pdfplumber did not extract generated text")
        merge(str(merged), [str(generated)])
        if len(PdfReader(str(merged)).pages) != 1:
            raise AssertionError("merged PDF did not contain one page")
    print("pdf smoke: passed")
    return 0


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    subparsers = parser.add_subparsers(dest="command", required=True)
    p_info = subparsers.add_parser("info")
    p_info.add_argument("input")
    p_extract = subparsers.add_parser("extract")
    p_extract.add_argument("input")
    p_extract.add_argument("--output")
    p_merge = subparsers.add_parser("merge")
    p_merge.add_argument("output")
    p_merge.add_argument("inputs", nargs="+")
    p_generate = subparsers.add_parser("generate")
    p_generate.add_argument("output")
    p_generate.add_argument("--text", default="Codex PDF")
    subparsers.add_parser("smoke")
    args = parser.parse_args(argv)
    try:
        if args.command == "info":
            return info(args.input)
        if args.command == "extract":
            return extract(args.input, args.output)
        if args.command == "merge":
            return merge(args.output, args.inputs)
        if args.command == "generate":
            return generate(args.output, args.text)
        return smoke()
    except Exception as exc:
        print(f"pdf operation failed: {exc}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
