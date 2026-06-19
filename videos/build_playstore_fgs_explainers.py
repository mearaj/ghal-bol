#!/usr/bin/env python3
"""Build Play Console foreground-service explainer videos (1080x1920, ~30s)."""

from __future__ import annotations

import subprocess
import textwrap
from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

ROOT = Path(__file__).resolve().parent
OUT = ROOT
WORK = ROOT / "_work"
W, H = 1080, 1920
BG = (31, 44, 52)  # #1F2C34
ACCENT = (37, 211, 102)  # #25D366
WHITE = (255, 255, 255)
MUTED = (134, 150, 160)  # #8696A0
SLIDE_SEC = 5
FPS = 30

FONT_BOLD = "/usr/share/fonts/TTF/DejaVuSans-Bold.ttf"
FONT_REG = "/usr/share/fonts/TTF/DejaVuSans.ttf"

CAMERA_SLIDES: list[tuple[str, str]] = [
    (
        "Ghal Bol",
        "Peer-to-peer encrypted messenger.\nVideo calls are a core feature of the app.",
    ),
    (
        "Why the camera?",
        "A video call only works if your contact can see you live.\n"
        "Ghal Bol streams camera video directly to them—nothing is stored on our servers.",
    ),
    (
        "You stay in control",
        "Camera turns on only when you tap Video during an active call.\n"
        "Android shows the camera privacy indicator while capture runs.",
    ),
    (
        "Encrypted peer-to-peer",
        "Frames are encoded and sent over your open P2P connection to the person you called.\n"
        "No cloud upload. No third-party video hosting.",
    ),
    (
        "Why a foreground service?",
        "Calls run in Ghal Bol’s networking process so chat and media stay connected.\n"
        "FOREGROUND_SERVICE_CAMERA lets that process keep sending your video for the call.",
    ),
    (
        "When it stops",
        "Camera capture ends when you turn video off, hang up, or sign out.\n"
        "It is never used outside a call you started or accepted.",
    ),
]

MICROPHONE_SLIDES: list[tuple[str, str]] = [
    (
        "Ghal Bol",
        "Peer-to-peer encrypted messenger.\nVoice calls are a core feature of the app.",
    ),
    (
        "Why the microphone?",
        "Voice calls need live audio from your mic, sent in real time to your contact.\n"
        "This is direct P2P communication—not a server-side voice mailbox.",
    ),
    (
        "You stay in control",
        "The mic is used only after you start or accept a voice call.\n"
        "Android shows the microphone indicator while the call is active.",
    ),
    (
        "Encrypted peer-to-peer",
        "Audio is encoded and streamed over your open P2P connection.\n"
        "We do not record calls or store them on a central server.",
    ),
    (
        "Why a foreground service?",
        "Calls run in Ghal Bol’s networking process alongside encrypted chat.\n"
        "FOREGROUND_SERVICE_MICROPHONE keeps capture reliable for the whole call.",
    ),
    (
        "When it stops",
        "Microphone capture ends when you hang up or sign out.\n"
        "There is no always-on listening outside active calls.",
    ),
]

REMOTE_MESSAGING_SLIDES: list[tuple[str, str]] = [
    (
        "Ghal Bol",
        "Peer-to-peer encrypted messenger.\nText chat works directly between your devices.",
    ),
    (
        "Always-on when signed in",
        "After you unlock, Ghal Bol keeps your P2P connection ready so peers can reach you.",
    ),
    (
        "Visible notification",
        "A low-priority notification says “Listening for messages” so you know networking is active.",
    ),
    (
        "Receive texts in the background",
        "Inbound messages and delivery signals arrive even when the chat screen is not open.",
    ),
]


def load_font(path: str, size: int) -> ImageFont.FreeTypeFont:
    return ImageFont.truetype(path, size)


def wrap_paragraph(text: str, width: int) -> list[str]:
    lines: list[str] = []
    for para in text.split("\n"):
        para = para.strip()
        if not para:
            lines.append("")
            continue
        lines.extend(textwrap.wrap(para, width=width, break_long_words=False))
    return lines


def render_slide(path: Path, title: str, body: str, *, step: str) -> None:
    img = Image.new("RGB", (W, H), BG)
    draw = ImageDraw.Draw(img)

    # Top accent bar
    draw.rectangle((0, 0, W, 10), fill=ACCENT)

    title_font = load_font(FONT_BOLD, 54)
    body_font = load_font(FONT_REG, 38)
    foot_font = load_font(FONT_REG, 28)

    # Title
    title_bbox = draw.textbbox((0, 0), title, font=title_font)
    title_w = title_bbox[2] - title_bbox[0]
    draw.text(((W - title_w) / 2, H * 0.22), title, font=title_font, fill=ACCENT)

    # Body (centered block)
    body_lines = wrap_paragraph(body, width=34)
    line_heights = []
    for line in body_lines:
        if line:
            bb = draw.textbbox((0, 0), line, font=body_font)
            line_heights.append(bb[3] - bb[1] + 14)
        else:
            line_heights.append(20)
    block_h = sum(line_heights)
    y = (H * 0.38) - (block_h / 2)
    for line, lh in zip(body_lines, line_heights):
        if line:
            bb = draw.textbbox((0, 0), line, font=body_font)
            lw = bb[2] - bb[0]
            draw.text(((W - lw) / 2, y), line, font=body_font, fill=WHITE)
        y += lh

    # Footer
    footer = f"Ghal Bol · P2P · {step}"
    fb = draw.textbbox((0, 0), footer, font=foot_font)
    fw = fb[2] - fb[0]
    draw.text(((W - fw) / 2, H * 0.88), footer, font=foot_font, fill=MUTED)

    path.parent.mkdir(parents=True, exist_ok=True)
    img.save(path, "PNG")


def slides_to_mp4(slides: list[tuple[str, str]], prefix: str, out_name: str) -> Path:
    slide_dir = WORK / prefix
    if slide_dir.exists():
        for p in slide_dir.glob("*.png"):
            p.unlink()
    else:
        slide_dir.mkdir(parents=True, exist_ok=True)

    total = len(slides)
    pngs: list[Path] = []
    for i, (title, body) in enumerate(slides, start=1):
        png = slide_dir / f"{i:02d}.png"
        render_slide(png, title, body, step=f"{i}/{total}")
        pngs.append(png)

    list_file = WORK / f"{prefix}.txt"
    seg_files: list[Path] = []
    for i, png in enumerate(pngs):
        seg = WORK / f"{prefix}_{i:02d}.mp4"
        subprocess.run(
            [
                "ffmpeg",
                "-y",
                "-hide_banner",
                "-loglevel",
                "error",
                "-loop",
                "1",
                "-i",
                str(png),
                "-t",
                str(SLIDE_SEC),
                "-r",
                str(FPS),
                "-c:v",
                "libx264",
                "-pix_fmt",
                "yuv420p",
                str(seg),
            ],
            check=True,
        )
        seg_files.append(seg)

    with list_file.open("w", encoding="utf-8") as f:
        for seg in seg_files:
            f.write(f"file '{seg}'\n")

    out = OUT / out_name
    subprocess.run(
        [
            "ffmpeg",
            "-y",
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "concat",
            "-safe",
            "0",
            "-i",
            str(list_file),
            "-c",
            "copy",
            str(out),
        ],
        check=True,
    )
    return out


def main() -> None:
    WORK.mkdir(parents=True, exist_ok=True)
    built = [
        slides_to_mp4(
            CAMERA_SLIDES,
            "camera",
            "FOREGROUND_SERVICE_CAMERA_PLAYSTORE_EXPLANATION.mp4",
        ),
        slides_to_mp4(
            MICROPHONE_SLIDES,
            "microphone",
            "FOREGROUND_SERVICE_MICROPHONE_PLAYSTORE_EXPLANATION.mp4",
        ),
        slides_to_mp4(
            REMOTE_MESSAGING_SLIDES,
            "remote_messaging",
            "FOREGROUND_SERVICE_REMOTE_MESSAGING_PLAYSTORE_EXPLANATION.mp4",
        ),
    ]
    for p in built:
        dur = subprocess.check_output(
            [
                "ffprobe",
                "-v",
                "error",
                "-show_entries",
                "format=duration",
                "-of",
                "default=noprint_wrappers=1:nokey=1",
                str(p),
            ],
            text=True,
        ).strip()
        print(f"{p.name}: {dur}s ({p.stat().st_size // 1024} KiB)")


if __name__ == "__main__":
    main()
