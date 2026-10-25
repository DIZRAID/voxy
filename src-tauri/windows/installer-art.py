#!/usr/bin/env python3
"""Draw the NSIS installer bitmaps (Windows) from the app icon.

    python3 src-tauri/windows/installer-art.py src-tauri/icons/icon.png src-tauri/windows

Writes nsis-sidebar.bmp (164x314, the Welcome and Finish pages) and
nsis-header.bmp (150x57, the header of the other pages; MUI2 shows it on the
left, so the icon sits at the left edge). 24-bit BMP, the format NSIS needs.
tauri.conf.json points bundle.windows.nsis.sidebarImage / headerImage at
them. Needs Pillow and the Inter font (any bold/medium sans works: change
BOLD and REG below).
"""
import sys
from PIL import Image, ImageDraw, ImageFont, ImageFilter
icon_path, out_dir = sys.argv[1], sys.argv[2]
icon = Image.open(icon_path).convert("RGBA")
BOLD = "/usr/share/fonts/opentype/inter/InterDisplay-Bold.otf"
REG = "/usr/share/fonts/opentype/inter/Inter-Medium.otf"

def vgrad(w, h, top, bottom):
    img = Image.new("RGB", (w, h))
    d = ImageDraw.Draw(img)
    for y in range(h):
        t = y / (h - 1)
        d.line([(0, y), (w, y)], fill=tuple(int(a + (b - a) * t) for a, b in zip(top, bottom)))
    return img

# Sidebar: welcome and finish pages, 164x314 (MUI2 at 96 dpi).
W, H = 164, 314
side = vgrad(W, H, (36, 28, 64), (16, 12, 30)).convert("RGBA")
glow = Image.new("RGBA", (W, H), (0, 0, 0, 0))
ImageDraw.Draw(glow).ellipse([W // 2 - 70, 40, W // 2 + 70, 180], fill=(125, 122, 255, 90))
side = Image.alpha_composite(side, glow.filter(ImageFilter.GaussianBlur(28)))
ic = icon.resize((96, 96), Image.LANCZOS)
side.alpha_composite(ic, ((W - 96) // 2, 62))
d = ImageDraw.Draw(side)
f1 = ImageFont.truetype(BOLD, 30)
f2 = ImageFont.truetype(REG, 12)
for text, font, y, color in [("Voxy", f1, 172, (255, 255, 255)),
                             ("Push-to-talk dictation", f2, 212, (170, 168, 255))]:
    w = d.textlength(text, font=font)
    d.text(((W - w) / 2, y), text, font=font, fill=color)
side.convert("RGB").save(f"{out_dir}/nsis-sidebar.bmp")

# Header: inner pages, 150x57, on the dialog's white header strip.
HW, HH = 150, 57
head = Image.new("RGBA", (HW, HH), (255, 255, 255, 255))
hi = icon.resize((40, 40), Image.LANCZOS)
head.alpha_composite(hi, (10, (HH - 40) // 2))
head.convert("RGB").save(f"{out_dir}/nsis-header.bmp")
