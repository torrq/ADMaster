#!/usr/bin/env python3
"""Render a GitHub README hero banner (assets/banner.png and assets/banner.svg)
by combining assets/icon.svg on the left with bold sans-serif 'ADMaster' typography.
"""

from __future__ import annotations

import argparse
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

from render_icon import ROOT, SVG_PATH, render_svg_master

DEFAULT_PNG_OUT = ROOT / "assets" / "banner.png"
DEFAULT_SVG_OUT = ROOT / "assets" / "banner.svg"

# Candidate bold sans-serif system fonts on Windows / macOS / Linux
FONT_CANDIDATES = [
    r"C:\Windows\Fonts\segoeuib.ttf",
    r"C:\Windows\Fonts\arialbd.ttf",
    r"C:\Windows\Fonts\calibrib.ttf",
    "/System/Library/Fonts/Supplemental/Arial Bold.ttf",
    "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
]


def load_bold_sans_font(size_px: int) -> ImageFont.FreeTypeFont | ImageFont.ImageFont:
    for candidate in FONT_CANDIDATES:
        p = Path(candidate)
        if p.is_file():
            return ImageFont.truetype(str(p), size_px)
    return ImageFont.load_default()


def render_banner_png(
    svg_path: Path,
    out_png: Path,
    title: str = "ADMaster",
    width: int = 1672,
    height: int = 500,
    bg_rgb: tuple[int, int, int] = (15, 18, 25),
) -> None:
    # Render at 2x supersampling for razor-sharp anti-aliased edges
    ss = 2
    W = width * ss
    H = height * ss

    canvas = Image.new("RGBA", (W, H), (*bg_rgb, 255))

    # Render the SVG app icon so its visible squircle (464/512 of viewBox) fills ~88% of banner height
    icon_box_px = int(round(H * 0.96))
    icon_img = render_svg_master(svg_path, render_px=icon_box_px)
    icon_x = int(round(W * 0.012))
    icon_y = (H - icon_box_px) // 2
    canvas.alpha_composite(icon_img, (icon_x, icon_y))

    # Visible right edge of the icon squircle (493 / 512 of icon_box_px)
    squircle_right = icon_x + int(round(icon_box_px * (493.0 / 512.0)))
    right_margin = int(round(W * 0.038))
    text_left = squircle_right + int(round(W * 0.052))
    avail_text_w = W - text_left - right_margin

    draw = ImageDraw.Draw(canvas)

    # Binary-search font size so the title fills avail_text_w cleanly (matching TruePeaks proportions)
    lo, hi = 60, int(H * 0.72)
    best_size = lo
    while lo <= hi:
        mid = (lo + hi) // 2
        font = load_bold_sans_font(mid)
        bbox = draw.textbbox((0, 0), title, font=font)
        tw = bbox[2] - bbox[0]
        th = bbox[3] - bbox[1]
        if tw <= avail_text_w and th <= int(H * 0.36):
            best_size = mid
            lo = mid + 1
        else:
            hi = mid - 1

    font = load_bold_sans_font(best_size)
    bbox = draw.textbbox((0, 0), title, font=font)
    tw = bbox[2] - bbox[0]
    th = bbox[3] - bbox[1]

    # Center vertically against the optical midpoint of the banner
    tx = text_left - bbox[0]
    ty = (H - th) // 2 - bbox[1]
    draw.text((tx, ty), title, font=font, fill=(255, 255, 255, 255))

    # Downsample 2x -> 1x with Lanczos and save optimized web-friendly PNG
    final_img = canvas.convert("RGB").resize((width, height), Image.Resampling.LANCZOS)
    out_png.parent.mkdir(parents=True, exist_ok=True)
    final_img.save(out_png, format="PNG", optimize=True)
    print(f"Wrote {out_png.relative_to(ROOT)} ({out_png.stat().st_size:,} bytes, {width}x{height})")


def render_banner_svg(
    svg_path: Path,
    out_svg: Path,
    title: str = "ADMaster",
    width: int = 1672,
    height: int = 500,
    bg_hex: str = "#0F1219",
) -> None:
    raw_svg = svg_path.read_text(encoding="utf-8")
    # Extract inner elements between <svg ...> and </svg>
    start_idx = raw_svg.find(">") + 1
    end_idx = raw_svg.rfind("</svg>")
    inner_elements = raw_svg[start_idx:end_idx].strip()

    icon_size = int(round(height * 0.96))
    icon_x = int(round(width * 0.012))
    icon_y = (height - icon_size) // 2
    scale = icon_size / 512.0

    banner_svg = f"""<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {width} {height}" width="{width}" height="{height}">
  <rect width="{width}" height="{height}" fill="{bg_hex}"/>
  <g transform="translate({icon_x}, {icon_y}) scale({scale:.6f})">
    {inner_elements}
  </g>
  <text x="580" y="{height // 2 + 56}" fill="#FFFFFF" font-family="Segoe UI, Inter, -apple-system, BlinkMacSystemFont, Arial, sans-serif" font-size="196" font-weight="700" letter-spacing="-1">{title}</text>
</svg>
"""
    out_svg.parent.mkdir(parents=True, exist_ok=True)
    out_svg.write_text(banner_svg, encoding="utf-8")
    print(f"Wrote {out_svg.relative_to(ROOT)} ({out_svg.stat().st_size:,} bytes)")


def main() -> None:
    parser = argparse.ArgumentParser(description="Render GitHub README banner from assets/icon.svg")
    parser.add_argument("--svg", type=Path, default=SVG_PATH, help="Input app icon SVG path")
    parser.add_argument("--out", type=Path, default=DEFAULT_PNG_OUT, help="Output PNG banner path")
    parser.add_argument("--out-svg", type=Path, default=DEFAULT_SVG_OUT, help="Output SVG banner path")
    parser.add_argument("--text", type=str, default="ADMaster", help="Banner title text")
    parser.add_argument("--width", type=int, default=1672, help="Banner width in pixels")
    parser.add_argument("--height", type=int, default=500, help="Banner height in pixels")
    args = parser.parse_args()

    render_banner_png(args.svg, args.out, title=args.text, width=args.width, height=args.height)
    render_banner_svg(args.svg, args.out_svg, title=args.text, width=args.width, height=args.height)


if __name__ == "__main__":
    main()
