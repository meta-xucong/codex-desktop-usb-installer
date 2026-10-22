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
    
    # Word wrap for long titles
    max_chars_per_line = 15
    if len(title) > max_chars_per_line:
        words = []
        for i in range(0, len(title), max_chars_per_line):
            words.append(title[i:i+max_chars_per_line])
        title_wrapped = '\n'.join(words)
    else:
        title_wrapped = title
    
    draw.text(
        (title_x, title_y),
        title_wrapped,
        fill=primary_color,
        font=title_font,
        spacing=10
    )
    
    # Draw subtitle
    if subtitle:
        subtitle_y = int(height * 0.55)
        draw.text(
            (title_x, subtitle_y),
            subtitle,
            fill=hex_to_rgb(default_colors["secondary"]),
            font=subtitle_font
        )
    
    # Draw author
    if author:
        author_y = height - int(height * 0.12)
        draw.text(
            (title_x, author_y),
            f"作者：{author}",
            fill=text_color,
            font=author_font
        )
    
    # Draw date
    date_str = datetime.now().strftime("%Y.%m.%d")
    date_x = width - int(width * 0.1)
    date_y = height - int(height * 0.12)
    draw.text(
        (date_x, date_y),
        date_str,
        fill=text_color,
        font=author_font,
        anchor="rm"
    )
    
    # Save image
    if output_path is None:
        output_dir = resolve_article_data_dir() / "covers"
        output_dir.mkdir(parents=True, exist_ok=True)
        output_path = output_dir / f"cover_{datetime.now().strftime('%Y%m%d_%H%M%S_%f')}.jpg"
    else:
        output_path = Path(output_path).expanduser().resolve()
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
