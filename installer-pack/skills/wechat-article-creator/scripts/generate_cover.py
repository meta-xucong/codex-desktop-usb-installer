#!/usr/bin/env python
"""
Generate WeChat article cover images using PIL and optional AI
"""
import argparse
import json
import os
from datetime import datetime
from pathlib import Path
from PIL import Image, ImageDraw, ImageFont


def resolve_article_data_dir() -> Path:
    explicit = str(os.environ.get("WECHAT_ARTICLE_DATA_DIR") or "").strip()
    if explicit:
        return Path(explicit).expanduser().resolve()
    root = Path(os.environ.get("CODEX_DATA_DIR", Path.cwd() / "codex-data"))
    return (root.expanduser() / "wechat-article-creator").resolve()


def get_default_colors(style: str) -> dict:
    """Get color palette by style"""
    palettes = {
        "modern": {
            "bg": "#FFFFFF",
            "primary": "#FF6B6B",
            "secondary": "#4ECDC4",
            "text": "#333333",
            "accent": "#FFE66D"
        },
        "minimalist": {
            "bg": "#FAFAFA",
            "primary": "#000000",
            "secondary": "#999999",
            "text": "#333333",
            "accent": "#EEEEEE"
        },
        "bold": {
            "bg": "#1A1A1A",
            "primary": "#FF3B30",
            "secondary": "#00D4FF",
            "text": "#FFFFFF",
            "accent": "#FFD700"
        },
        "tech": {
            "bg": "#0F1419",
            "primary": "#00FF41",
            "secondary": "#FF006E",
            "text": "#FFFFFF",
            "accent": "#08F7FE"
        }
    }
    return palettes.get(style, palettes["modern"])


def hex_to_rgb(hex_color: str) -> tuple:
    """Convert hex color to RGB tuple"""
    hex_color = hex_color.lstrip('#')
    return tuple(int(hex_color[i:i+2], 16) for i in (0, 2, 4))


def _wrap_text_to_width(draw: ImageDraw.ImageDraw, text: str, font: ImageFont.FreeTypeFont, max_width: int) -> str:
    """Wrap text at measured pixel width, preserving explicit paragraphs."""
    lines = []
    for paragraph in str(text).split("\n"):
        line = ""
        for char in paragraph:
            candidate = line + char
            candidate_bounds = draw.textbbox((0, 0), candidate, font=font)
            if line and candidate_bounds[2] - candidate_bounds[0] > max_width:
                lines.append(line.rstrip())
                line = char.lstrip()
            else:
                line = candidate
        lines.append(line.rstrip())
    return "\n".join(lines)


def _fit_title_layout(
    draw: ImageDraw.ImageDraw,
    title: str,
    font_path: str,
    base_size: int,
    x: int,
    y: int,
    max_width: int,
    bottom: int,
    spacing: int = 10,
    minimum_size: int = 16,
):
    """Return the largest title font that fits the horizontal and vertical safe area."""
    for size in range(base_size, minimum_size - 1, -1):
        font = ImageFont.truetype(font_path, size)
        wrapped = _wrap_text_to_width(draw, title, font, max_width)
        bounds = draw.multiline_textbbox((x, y), wrapped, font=font, spacing=spacing)
        if bounds[0] >= x and bounds[2] <= x + max_width and bounds[3] <= bottom:
            return wrapped, font, bounds
    raise ValueError(
        f"标题无法放入封面安全区域（最小字号 {minimum_size}px，安全宽度 {max_width}px）；"
        "请缩短标题或调整封面尺寸。"
    )


def _fit_auxiliary_text(
    draw: ImageDraw.ImageDraw,
    text: str,
    font_path: str | None,
    fallback_font: ImageFont.ImageFont,
    base_size: int,
    x: int,
    y: int,
    max_width: int,
    bottom: int,
    label: str,
    minimum_size: int = 14,
    spacing: int = 4,
):
    """Wrap and shrink subtitle/author text to a measured safe rectangle."""
    sizes = range(base_size, minimum_size - 1, -1) if font_path else (None,)
    for size in sizes:
        font = ImageFont.truetype(font_path, size) if size is not None else fallback_font
        wrapped = _wrap_text_to_width(draw, text, font, max_width)
        bounds = draw.multiline_textbbox((x, y), wrapped, font=font, spacing=spacing)
        if bounds[0] >= x and bounds[2] <= x + max_width and bounds[3] <= bottom:
            return wrapped, font, bounds
    raise ValueError(f"{label}无法放入封面安全区域；请缩短内容。")


def generate_cover(
    title: str,
    subtitle: str = "",
    author: str = "",
    style: str = "modern",
    colors: dict = None,
    width: int = 1200,
    height: int = 630,
    output_path: str = None
):
    """Generate a WeChat article cover image"""
    resolved_output_path = None
    if output_path is not None:
        resolved_output_path = Path(output_path).expanduser().resolve()
        if resolved_output_path.suffix.casefold() not in {".jpg", ".jpeg", ".png"}:
            raise ValueError("--output 只支持 .jpg、.jpeg 或 .png 扩展名。")

    if not (256 <= width <= 8192 and 256 <= height <= 8192):
        raise ValueError("width 和 height 必须在 256 到 8192 之间。")
    if width * height > 40_000_000:
        raise ValueError("图片总像素不得超过 4000 万。")
    # Get colors
    default_colors = get_default_colors(style)
    if colors:
        default_colors.update(colors)
    
    # Create image
    bg_color = hex_to_rgb(default_colors["bg"])
    img = Image.new("RGB", (width, height), bg_color)
    draw = ImageDraw.Draw(img)
    
    # Try to load fonts (fallback to default if not available)
    title_font_path = None
    try:
        # Cross-platform font paths
        font_paths = []
        
        # Windows
        windir = os.environ.get('WINDIR', 'C:\\Windows')
        font_paths.extend([
            os.path.join(windir, 'Fonts', 'msyh.ttc'),      # Microsoft YaHei
            os.path.join(windir, 'Fonts', 'msyhbd.ttc'),     # Microsoft YaHei Bold
            os.path.join(windir, 'Fonts', 'simhei.ttf'),     # SimHei
            os.path.join(windir, 'Fonts', 'simsun.ttc'),     # SimSun
            os.path.join(windir, 'Fonts', 'arial.ttf'),      # Arial
        ])
        
        # macOS
        font_paths.extend([
            '/System/Library/Fonts/PingFang.ttc',
            '/System/Library/Fonts/STHeiti Light.ttc',
            '/System/Library/Fonts/Hiragino Sans GB.ttc',
            '/Library/Fonts/Arial Unicode.ttf',
        ])
        
        # Linux
        font_paths.extend([
            '/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc',
            '/usr/share/fonts/truetype/wqy/wqy-zenhei.ttc',
            '/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf',
            '/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf',
        ])
        
        font_path = None
        for path in font_paths:
            if os.path.exists(path):
                font_path = path
                break
        
        if font_path:
            title_font_path = font_path
            title_font = ImageFont.truetype(font_path, int(width * 0.08))
            subtitle_font = ImageFont.truetype(font_path, int(width * 0.04))
            author_font = ImageFont.truetype(font_path, int(width * 0.03))
        else:
            raise FileNotFoundError("No suitable font found")
    except:
        # Fallback to default font
        title_font = ImageFont.load_default()
        subtitle_font = ImageFont.load_default()
        author_font = ImageFont.load_default()
    
    # Draw background decoration
    primary_color = hex_to_rgb(default_colors["primary"])
    accent_color = hex_to_rgb(default_colors["accent"])
    
    # Draw accent shapes based on style
    if style == "bold":
        # Bold style: diagonal accent
        draw.polygon(
            [(0, 0), (width * 0.3, 0), (0, height * 0.3)],
            fill=primary_color
        )
    elif style == "tech":
        # Tech style: corner accents
        draw.rectangle([(0, 0), (width * 0.05, height)], fill=primary_color)
        draw.rectangle([(width * 0.95, 0), (width, height)], fill=accent_color)
    elif style == "modern":
        # Modern style: bottom accent bar
        draw.rectangle(
            [(0, height * 0.85), (width, height)],
            fill=accent_color
        )
    
    # Draw title
    text_color = hex_to_rgb(default_colors["text"])
    title_x = int(width * 0.1)
    title_y = int(height * 0.25)
    
    # Keep titles inside the 10%-90% horizontal safe area and above the subtitle.
    title_spacing = 10
    title_max_width = int(width * 0.8)
    subtitle_y = int(height * 0.55)
    title_bottom = subtitle_y - 16 if subtitle else int(height * 0.78)
    if title_font_path is not None:
        title_wrapped, title_font, _ = _fit_title_layout(
            draw=draw,
            title=title,
            font_path=title_font_path,
            base_size=int(width * 0.08),
            x=title_x,
            y=title_y,
            max_width=title_max_width,
            bottom=title_bottom,
            spacing=title_spacing,
        )
    else:
        title_wrapped = _wrap_text_to_width(draw, title, title_font, title_max_width)
        title_bounds = draw.multiline_textbbox((title_x, title_y), title_wrapped, font=title_font, spacing=title_spacing)
        if title_bounds[0] < title_x or title_bounds[2] > title_x + title_max_width or title_bounds[3] > title_bottom:
            raise ValueError("当前字体无法将标题放入封面安全区域；请安装支持的字体或缩短标题。")
    
    draw.text(
        (title_x, title_y),
        title_wrapped,
        fill=primary_color,
        font=title_font,
        spacing=title_spacing
    )
    
    # Reserve a measured bottom row for the author and date before fitting text.
    author_y = height - int(height * 0.12)
    date_str = datetime.now().strftime("%Y.%m.%d")
    date_x = width - int(width * 0.1)
    date_font = author_font
    date_bounds = draw.textbbox((date_x, author_y), date_str, font=date_font, anchor="rm")

    # Draw subtitle inside the gap between the title and bottom metadata row.
    if subtitle:
        subtitle_wrapped, subtitle_font, _ = _fit_auxiliary_text(
            draw=draw,
            text=subtitle,
            font_path=title_font_path,
            fallback_font=subtitle_font,
            base_size=int(width * 0.04),
            x=title_x,
            y=subtitle_y,
            max_width=title_max_width,
            bottom=author_y - 16,
            label="副标题",
        )
        draw.multiline_text(
            (title_x, subtitle_y),
            subtitle_wrapped,
            fill=hex_to_rgb(default_colors["secondary"]),
            font=subtitle_font,
            spacing=4,
        )

    # Keep author text left of the date; reject content that cannot fit.
    if author:
        author_text = f"作者：{author}"
        author_max_width = date_bounds[0] - title_x - 24
        author_wrapped, author_font, _ = _fit_auxiliary_text(
            draw=draw,
            text=author_text,
            font_path=title_font_path,
            fallback_font=author_font,
            base_size=int(width * 0.03),
            x=title_x,
            y=author_y,
            max_width=author_max_width,
            bottom=height - int(height * 0.05),
            label="作者",
            minimum_size=12,
            spacing=2,
        )
        draw.multiline_text(
            (title_x, author_y), author_wrapped, fill=text_color, font=author_font, spacing=2
        )

    # Draw date
    date_y = author_y
    draw.text(
        (date_x, date_y),
        date_str,
        fill=text_color,
        font=date_font,
        anchor="rm"
    )
    
    # Save image
    if output_path is None:
        output_dir = resolve_article_data_dir() / "covers"
        output_dir.mkdir(parents=True, exist_ok=True)
        output_path = output_dir / f"cover_{datetime.now().strftime('%Y%m%d_%H%M%S_%f')}.jpg"
    else:
        output_path = resolved_output_path
        output_path.parent.mkdir(parents=True, exist_ok=True)
    
    output_path = Path(output_path).resolve()
    suffix = output_path.suffix.casefold()
    if suffix in {".jpg", ".jpeg"}:
        img.save(output_path, "JPEG", quality=95)
    elif suffix == ".png":
        img.save(output_path, "PNG", optimize=True)
    else:
        raise ValueError("--output 只支持 .jpg、.jpeg 或 .png 扩展名。")
    print(f"[OK] Cover generated: {output_path}")
    return str(output_path)


def main():
    parser = argparse.ArgumentParser(description="Generate WeChat article cover")
    parser.add_argument("--title", required=True, help="Article title")
    parser.add_argument("--subtitle", default="", help="Subtitle (optional)")
    parser.add_argument("--author", default="", help="Author name (optional)")
    parser.add_argument("--style", default="modern", 
                       choices=["modern", "minimalist", "bold", "tech"],
                       help="Cover style")
    parser.add_argument("--colors", type=json.loads, default=None,
                       help='Custom colors as JSON (e.g., \'{"primary":"#FF0000"}\')')
    parser.add_argument("--width", type=int, default=1200, help="Image width (default: 1200)")
    parser.add_argument("--height", type=int, default=630, help="Image height (default: 630)")
    parser.add_argument("--output", default=None, help="Output file path (optional)")
    
    args = parser.parse_args()
    
    generate_cover(
        title=args.title,
        subtitle=args.subtitle,
        author=args.author,
        style=args.style,
        colors=args.colors,
        width=args.width,
        height=args.height,
        output_path=args.output
    )
    
    print("\nNext steps:")
    print("   1. Review the generated cover")
    print("   2. Upload to WeChat editor")
    print("   3. Publish your article!")


if __name__ == "__main__":
    main()
