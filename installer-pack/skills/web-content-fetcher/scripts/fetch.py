#!/usr/bin/env python
"""
通用网页正文提取脚本（基于 Scrapling + html2text）
返回干净的 Markdown 格式，效果与 Jina Reader 相当。
用于 article-writer skill 的二级内容提取方案（Jina 超限或失败时使用）

用法：
  python fetch.py <url> [max_chars]

示例：
  python fetch.py https://example.com/article 12000
  python fetch.py https://mp.weixin.qq.com/s/xxx 30000

输出：
  Markdown 格式正文，截断至 max_chars（默认 30000）
  图片使用原始 URL 内嵌在 Markdown 中（data-src 懒加载已自动处理）
"""

import argparse
import base64
import re
import sys


_IMG_TAG_RE = re.compile(r"<img\b([^>]*)>", re.IGNORECASE)
_DATA_SRC_RE = re.compile(r"\s+data-src\s*=\s*(?:([\"'])(.*?)\1|([^\s>]+))", re.IGNORECASE)
_SRC_RE = re.compile(r"\s+src\s*=\s*(?:([\"'])(.*?)\1|([^\s>]+))", re.IGNORECASE)


def _validate_max_chars(max_chars):
    """Require a positive integer limit (with bool explicitly excluded)."""
    if isinstance(max_chars, bool) or not isinstance(max_chars, int):
        raise ValueError("max_chars must be a positive integer")
    if max_chars <= 0:
        raise ValueError("max_chars must be a positive integer")
    return max_chars


def _positive_int(value):
    try:
        parsed = int(value)
    except (TypeError, ValueError) as exc:
        raise argparse.ArgumentTypeError("max_chars must be a positive integer") from exc
    if parsed <= 0:
        raise argparse.ArgumentTypeError("max_chars must be a positive integer")
    return parsed


def _attribute_value(match):
    return match.group(2) if match.group(1) else match.group(3)


def _is_placeholder_src(value):
    normalized = value.strip().lower()
    if normalized in {"", "#", "placeholder", "about:blank", "data:,", "javascript:void(0)"}:
        return True
    if normalized.startswith("data:image/gif;base64,"):
        try:
            image = base64.b64decode(value.partition(",")[2], validate=True)
            if image[:6] in {b"GIF87a", b"GIF89a"} and len(image) >= 10:
                width = int.from_bytes(image[6:8], "little")
                height = int.from_bytes(image[8:10], "little")
                return width == 1 and height == 1
        except (ValueError, TypeError):
            return False
    if normalized.startswith("data:image/png;base64,"):
        try:
            image = base64.b64decode(value.partition(",")[2], validate=True)
            if image.startswith(b"\x89PNG\r\n\x1a\n") and len(image) >= 24:
                width = int.from_bytes(image[16:20], "big")
                height = int.from_bytes(image[20:24], "big")
                return width == 1 and height == 1
        except (ValueError, TypeError):
            return False
    return False


def fix_lazy_images(html_raw):
    """
    微信公众号等平台用 data-src 懒加载图片，src 为占位符。
    将 data-src 的值提升为 src，确保 html2text 能正确渲染图片。
    """
    def promote(match):
        attrs = match.group(1)
        data_match = _DATA_SRC_RE.search(attrs)
        if not data_match:
            return match.group(0)

        lazy_src = _attribute_value(data_match).replace('"', "&quot;")
        src_match = _SRC_RE.search(attrs)
        if src_match and not _is_placeholder_src(_attribute_value(src_match)):
            # A real src already exists; remove the stale lazy attribute without
            # introducing a duplicate src that converters may interpret wrongly.
            updated = attrs[:data_match.start()] + attrs[data_match.end():]
        elif src_match:
            updated = attrs[:src_match.start()] + f' src="{lazy_src}"' + attrs[src_match.end():]
            data_match = _DATA_SRC_RE.search(updated)
            if data_match:
                updated = updated[:data_match.start()] + updated[data_match.end():]
        else:
            updated = attrs[:data_match.start()] + f' src="{lazy_src}"' + attrs[data_match.start():]
            data_match = _DATA_SRC_RE.search(updated)
            if data_match:
                updated = updated[:data_match.start()] + updated[data_match.end():]
        return f"<img{updated}>"

    return _IMG_TAG_RE.sub(promote, html_raw)


def scrapling_fetch(url, max_chars=30000):
    max_chars = _validate_max_chars(max_chars)
    try:
        import html2text
        from scrapling.fetchers import Fetcher
    except ImportError as exc:
        missing = getattr(exc, "name", None) or "optional package"
        raise RuntimeError(
            f"Optional batch-fetch dependency is missing: {missing}. "
            "Install the locked web-content-fetcher runtime or use Codex native web reading."
        ) from exc

    page = Fetcher.get(
        url,
        headers={"Referer": "https://www.google.com/search?q=site"}
    )

    h = html2text.HTML2Text()
    h.ignore_links = False
    h.ignore_images = False
    h.body_width = 0  # 不自动换行

    # 微信公众号专用选择器
    if "mp.weixin.qq.com" in url:
        selectors = ["div#js_content", "div.rich_media_content"]
    else:
        selectors = [
            'article',
            'main',
            '.post-content',
            '.entry-content',
            '.article-body',
            '[class*="body"]',
            '[class*="content"]',
            '[class*="article"]',
        ]

    for selector in selectors:
        els = page.css(selector)
        if els:
            html_raw = fix_lazy_images(els[0].html_content)
            md = h.handle(html_raw)
            md = re.sub(r'\n{3,}', '\n\n', md).strip()
            if len(md) > 300:
                return md[:max_chars], selector, len(md) > max_chars

    # fallback：全页转 Markdown
    html_raw = fix_lazy_images(page.html_content)
    md = h.handle(html_raw)
    md = re.sub(r'\n{3,}', '\n\n', md).strip()
    return md[:max_chars], 'body(fallback)', len(md) > max_chars


def parse_args():
    parser = argparse.ArgumentParser(
        description="使用可选 Scrapling/html2text 运行时提取网页正文；没有依赖时改用 Codex 原生网页读取。"
    )
    parser.add_argument("url", nargs="?", help="要提取的 HTTP(S) URL")
    parser.add_argument("max_chars", nargs="?", type=_positive_int, default=30000, help="最大输出字符数，默认 30000")
    return parser.parse_args()


def main():
    args = parse_args()
    if not args.url:
        raise SystemExit("URL is required. Run with --help for usage.")
    try:
        text, _selector, truncated = scrapling_fetch(args.url, args.max_chars)
    except RuntimeError as exc:
        print(f"Error: {exc}", file=sys.stderr)
        return 1
    print(text)
    if truncated:
        print(
            f"\n[正文已截断：原始正文超过 {args.max_chars} 个字符；"
            "max_chars 上限仅适用于正文，不包含此提示。]"
        )
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
