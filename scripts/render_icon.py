#!/usr/bin/env python3
"""Render assets/icon.svg into multi-size assets/icon.ico."""

from __future__ import annotations

import io
import re
import struct
import xml.etree.ElementTree as ET
from pathlib import Path

import matplotlib.patches as mpatches
import matplotlib.path as mpath
import numpy as np
from matplotlib.backends.backend_agg import FigureCanvasAgg
from matplotlib.figure import Figure
from PIL import Image

ROOT = Path(__file__).resolve().parent.parent
SVG_PATH = ROOT / "assets" / "icon.svg"
ICO_PATH = ROOT / "assets" / "icon.ico"
ICNS_PATH = ROOT / "assets" / "icon.icns"
ICO_SIZES = [16, 24, 32, 48, 64, 256]
ICNS_TYPES = [
    (16, b"icp4"),
    (32, b"icp5"),
    (64, b"ic12"),
    (128, b"ic07"),
    (256, b"ic08"),
    (512, b"ic09"),
    (1024, b"ic10"),
]


def parse_svg_path(d: str) -> mpath.Path:
    """Parse an SVG path string (M, L, H, V, Q, C, Z) into a matplotlib Path."""
    tokens = re.findall(r"[MLHVQCZmlhvqcz]|[-+]?(?:\d+\.?\d*|\.\d+)", d)
    verts: list[tuple[float, float]] = []
    codes: list[int] = []
    cx, cy = 0.0, 0.0
    sx, sy = 0.0, 0.0
    i = 0
    cmd = ""
    while i < len(tokens):
        if tokens[i].isalpha():
            cmd = tokens[i]
            i += 1
            if cmd in ("Z", "z"):
                verts.append((sx, sy))
                codes.append(mpath.Path.CLOSEPOLY)
                cx, cy = sx, sy
                continue
        if cmd == "M":
            cx, cy = float(tokens[i]), float(tokens[i + 1])
            sx, sy = cx, cy
            verts.append((cx, cy))
            codes.append(mpath.Path.MOVETO)
            i += 2
            cmd = "L"
        elif cmd == "L":
            cx, cy = float(tokens[i]), float(tokens[i + 1])
            verts.append((cx, cy))
            codes.append(mpath.Path.LINETO)
            i += 2
        elif cmd == "H":
            cx = float(tokens[i])
            verts.append((cx, cy))
            codes.append(mpath.Path.LINETO)
            i += 1
        elif cmd == "V":
            cy = float(tokens[i])
            verts.append((cx, cy))
            codes.append(mpath.Path.LINETO)
            i += 1
        elif cmd == "Q":
            x1, y1 = float(tokens[i]), float(tokens[i + 1])
            cx, cy = float(tokens[i + 2]), float(tokens[i + 3])
            verts.extend([(x1, y1), (cx, cy)])
            codes.extend([mpath.Path.CURVE3, mpath.Path.CURVE3])
            i += 4
        elif cmd == "C":
            x1, y1 = float(tokens[i]), float(tokens[i + 1])
            x2, y2 = float(tokens[i + 2]), float(tokens[i + 3])
            cx, cy = float(tokens[i + 4]), float(tokens[i + 5])
            verts.extend([(x1, y1), (x2, y2), (cx, cy)])
            codes.extend([mpath.Path.CURVE4, mpath.Path.CURVE4, mpath.Path.CURVE4])
            i += 6
        else:
            raise ValueError(f"Unsupported SVG path command: {cmd}")
    return mpath.Path(verts, codes)


def render_svg_master(svg_path: Path, render_px: int = 1024) -> Image.Image:
    """Rasterize assets/icon.svg at high resolution for crisp Lanczos downsampling."""
    tree = ET.parse(svg_path)
    root = tree.getroot()
    vb = [float(v) for v in root.attrib.get("viewBox", "0 0 512 512").split()]
    vb_x, vb_y, vb_w, vb_h = vb

    dpi = 72
    fig = Figure(figsize=(render_px / dpi, render_px / dpi), dpi=dpi)
    fig.patch.set_alpha(0.0)
    canvas = FigureCanvasAgg(fig)
    ax = fig.add_axes([0.0, 0.0, 1.0, 1.0])
    ax.set_xlim(vb_x, vb_x + vb_w)
    ax.set_ylim(vb_y + vb_h, vb_y)  # SVG top-down y axis
    ax.axis("off")

    # 1 SVG user unit in matplotlib points (1 pt = 1 px when dpi=72)
    unit_to_pt = render_px / vb_w

    for elem in root:
        tag = elem.tag.split("}")[-1]
        raw_fill = elem.attrib.get("fill", "#000000")
        facecolor = "none" if raw_fill == "none" else raw_fill
        raw_stroke = elem.attrib.get("stroke", "none")
        edgecolor = "none" if raw_stroke == "none" else raw_stroke
        stroke_w = float(elem.attrib.get("stroke-width", "0")) * unit_to_pt
        capstyle = elem.attrib.get("stroke-linecap", "butt")

        if tag == "rect":
            x = float(elem.attrib.get("x", "0"))
            y = float(elem.attrib.get("y", "0"))
            w = float(elem.attrib.get("width", "0"))
            h = float(elem.attrib.get("height", "0"))
            rx = float(elem.attrib.get("rx", "0"))
            patch = mpatches.FancyBboxPatch(
                (x + rx, y + rx),
                w - 2 * rx,
                h - 2 * rx,
                boxstyle=f"round,pad={rx},rounding_size={rx}",
                facecolor=facecolor,
                edgecolor=edgecolor,
                linewidth=stroke_w,
                antialiased=True,
            )
            ax.add_patch(patch)
        elif tag == "path":
            path_obj = parse_svg_path(elem.attrib["d"])
            patch = mpatches.PathPatch(
                path_obj,
                facecolor=facecolor,
                edgecolor=edgecolor,
                linewidth=stroke_w,
                capstyle=capstyle,
                joinstyle="round",
                antialiased=True,
            )
            ax.add_patch(patch)
        elif tag == "circle":
            cx = float(elem.attrib.get("cx", "0"))
            cy = float(elem.attrib.get("cy", "0"))
            r = float(elem.attrib.get("r", "0"))
            patch = mpatches.Circle(
                (cx, cy),
                radius=r,
                facecolor=facecolor,
                edgecolor=edgecolor,
                linewidth=stroke_w,
                antialiased=True,
            )
            ax.add_patch(patch)

    canvas.draw()
    buf = np.asarray(canvas.buffer_rgba())
    return Image.fromarray(buf, mode="RGBA")


def encode_dib_frame(img: Image.Image) -> bytes:
    """Encode an RGBA image into a 32-bit BGRA BITMAPINFOHEADER + XOR + AND mask DIB."""
    sz = img.width
    rgba = np.asarray(img, dtype=np.uint8)[::-1]  # bottom-up rows
    bgra = rgba[:, :, [2, 1, 0, 3]].tobytes()

    and_row_bytes = ((sz + 31) // 32) * 4
    and_mask = bytearray(and_row_bytes * sz)
    alpha = rgba[:, :, 3]
    for r in range(sz):
        for c in range(sz):
            if alpha[r, c] == 0:
                and_mask[r * and_row_bytes + (c >> 3)] |= 0x80 >> (c & 7)

    bih = struct.pack(
        "<IIIHHIIIIII",
        40,
        sz,
        sz * 2,
        1,
        32,
        0,
        len(bgra) + len(and_mask),
        0,
        0,
        0,
        0,
    )
    return bih + bgra + bytes(and_mask)


def build_ico(master: Image.Image, sizes: list[int]) -> bytes:
    """Encode standard Win32 DIB frames (<256px) and a PNG frame (256px) into an ICO."""
    frames: list[tuple[int, bytes]] = []
    for sz in sizes:
        resized = master.resize((sz, sz), Image.Resampling.LANCZOS)
        if sz >= 256:
            bio = io.BytesIO()
            resized.save(bio, format="PNG", optimize=True)
            frames.append((sz, bio.getvalue()))
        else:
            frames.append((sz, encode_dib_frame(resized)))

    header = struct.pack("<HHH", 0, 1, len(frames))
    offset = 6 + 16 * len(frames)
    dir_entries = bytearray()
    payloads = bytearray()

    for sz, payload in frames:
        b_width = 0 if sz >= 256 else sz
        b_height = 0 if sz >= 256 else sz
        dir_entries.extend(
            struct.pack(
                "<BBBBHHII",
                b_width,
                b_height,
                0,
                0,
                1,
                32,
                len(payload),
                offset,
            )
        )
        payloads.extend(payload)
        offset += len(payload)

    return bytes(header) + bytes(dir_entries) + bytes(payloads)


def build_icns(master: Image.Image) -> bytes:
    """Encode modern PNG-compressed Apple ICNS frames for macOS Finder & Dock."""
    chunks = bytearray()
    for sz, ostype in ICNS_TYPES:
        resized = master.resize((sz, sz), Image.Resampling.LANCZOS)
        bio = io.BytesIO()
        resized.save(bio, format="PNG", optimize=True)
        png_bytes = bio.getvalue()
        chunks.extend(struct.pack(">4sI", ostype, 8 + len(png_bytes)))
        chunks.extend(png_bytes)

    total_len = 8 + len(chunks)
    return struct.pack(">4sI", b"icns", total_len) + bytes(chunks)


def main() -> None:
    master = render_svg_master(SVG_PATH, render_px=2048)
    ico_bytes = build_ico(master, ICO_SIZES)
    ICO_PATH.write_bytes(ico_bytes)
    print(f"Wrote {ICO_PATH.relative_to(ROOT)} ({len(ico_bytes):,} bytes, sizes={ICO_SIZES})")

    icns_bytes = build_icns(master)
    ICNS_PATH.write_bytes(icns_bytes)
    print(f"Wrote {ICNS_PATH.relative_to(ROOT)} ({len(icns_bytes):,} bytes)")


if __name__ == "__main__":
    main()
