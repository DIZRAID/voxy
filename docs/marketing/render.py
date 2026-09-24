#!/usr/bin/env python3
"""Render Voxy's marketing screenshots from the REAL UI (ui/*.html|css|js).

Usage (from the repository root):

    python3 docs/marketing/render.py             # hero, island, social-preview
    python3 docs/marketing/render.py hero island # only these shots

By default it renders only the images the repository uses (DEFAULT_SHOTS):
hero.png and island.png in the README, social-preview.png as GitHub's
social card. The single-tab Settings scenes (model, general, recording,
history) are still in compose.js and render when named explicitly.

What it does:
  1. starts a static file server for the repository root on a free
     127.0.0.1 port (never 4173, which is the usual `ui/` dev preview);
  2. opens docs/marketing/compose.html?shot=<name> in headless Google Chrome
     at device scale factor 2 (the social card at 1x). The composition page
     loads ui/settings.html + settings.css + settings.js into its own
     document (so the window's glass blurs the wallpaper behind it) and
     embeds ui/island.html in iframes, driving both through their demo
     hooks (window.__settingsDebug / window.__islandDebug);
  3. re-encodes each screenshot with Pillow (RGB, optimize=True) and writes
     it to docs/images/<name>.png;
  4. stops Chrome, the server, and removes the temporary Chrome profile.

The scene is fixed (Thu Sep 24 2026, 9:41; demo history, download at 57 %,
island frozen on one waveform frame), so re-renders are identical as long
as ui/ and Chrome do not change. Shots, layout and wallpapers live in
compose.js / compose.css; see the comments there.

Requirements: Google Chrome in /Applications (or $CHROME), Python 3 with
Pillow. Nothing in ui/ is modified; the app is never launched.

The island's timer is set in SF Mono in the real app (WebKit). Chrome cannot
reach that system font by name, so the server also answers
/__sysfont/SFNSMono.ttf with /System/Library/Fonts/SFNSMono.ttf, for the
render only; the font is never copied into the repository. Without it
(e.g. on another OS) the timer falls back to Menlo.
"""

from __future__ import annotations

import functools
import http.server
import os
import shutil
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path

try:
    from PIL import Image
except ImportError:  # pragma: no cover
    sys.exit("Pillow is required: python3 -m pip install Pillow")

ROOT = Path(__file__).resolve().parents[2]
OUT = ROOT / "docs" / "images"
PAGE = "docs/marketing/compose.html"

CHROME_CANDIDATES = [
    os.environ.get("CHROME", ""),
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    "/Applications/Chromium.app/Contents/MacOS/Chromium",
    shutil.which("google-chrome") or "",
    shutil.which("chromium") or "",
]

# name → (CSS width, CSS height, device scale factor)
SHOTS = {
    "hero": (1440, 960, 2),
    "model": (1200, 800, 2),
    "general": (1200, 800, 2),
    "recording": (1200, 800, 2),
    "history": (1200, 800, 2),
    "island": (1600, 330, 2),
    "social-preview": (1280, 640, 1),
}

# rendered when no names are given: the images the repository actually uses
DEFAULT_SHOTS = ["hero", "island", "social-preview"]

# Virtual time the page gets before the capture: fetches + the settings
# demo boot + the island's recording/transcribing sequence all finish well
# inside this (see compose.js).
VIRTUAL_TIME_MS = 6000


# private render-time URL → system font (see the module docstring)
SYSFONTS = {"/__sysfont/SFNSMono.ttf": Path("/System/Library/Fonts/SFNSMono.ttf")}


class QuietHandler(http.server.SimpleHTTPRequestHandler):
    def log_message(self, *args):  # keep the console clean
        pass

    def translate_path(self, path):
        font = SYSFONTS.get(path.split("?", 1)[0])
        if font is not None:
            return str(font)  # a missing font is simply a 404
        return super().translate_path(path)

    def end_headers(self):
        self.send_header("Cache-Control", "no-store")
        super().end_headers()


def find_chrome() -> str:
    for c in CHROME_CANDIDATES:
        if c and Path(c).exists():
            return c
    sys.exit("Google Chrome not found; set $CHROME to its binary")


def start_server() -> http.server.ThreadingHTTPServer:
    handler = functools.partial(QuietHandler, directory=str(ROOT))
    httpd = http.server.ThreadingHTTPServer(("127.0.0.1", 0), handler)
    if httpd.server_address[1] == 4173:  # vanishingly unlikely, but never 4173
        httpd.server_close()
        return start_server()
    threading.Thread(target=httpd.serve_forever, daemon=True).start()
    return httpd


def capture(chrome: str, profile: str, url: str, w: int, h: int, dsf: int, dest: Path) -> None:
    cmd = [
        chrome,
        "--headless=new",
        f"--user-data-dir={profile}",
        "--no-first-run",
        "--no-default-browser-check",
        "--disable-extensions",
        "--disable-sync",
        "--disable-background-networking",
        "--disable-component-update",
        "--mute-audio",
        "--hide-scrollbars",
        "--force-color-profile=srgb",
        "--run-all-compositor-stages-before-draw",
        f"--force-device-scale-factor={dsf}",
        f"--window-size={w},{h}",
        f"--virtual-time-budget={VIRTUAL_TIME_MS}",
        f"--screenshot={dest}",
        url,
    ]
    if dest.exists():
        dest.unlink()
    # New headless Chrome on macOS writes the screenshot but does not always
    # quit afterwards, so wait for a complete PNG and then stop the whole
    # process group ourselves.
    log = open(dest.with_suffix(".log"), "wb")
    proc = subprocess.Popen(cmd, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
    deadline = time.time() + 120
    ok = False
    try:
        while time.time() < deadline:
            if dest.exists() and png_complete(dest):
                ok = True
                break
            if proc.poll() is not None and not dest.exists():
                break
            time.sleep(0.25)
    finally:
        stop(proc)
        log.close()
    if not ok:
        sys.stderr.write(dest.with_suffix(".log").read_bytes().decode(errors="replace")[-2000:])
        raise RuntimeError(f"Chrome produced no screenshot for {url}")


def png_complete(path: Path) -> bool:
    try:
        with Image.open(path) as im:
            im.load()
        return True
    except Exception:
        return False


def stop(proc: subprocess.Popen) -> None:
    if proc.poll() is None:
        try:
            os.killpg(proc.pid, 15)
        except ProcessLookupError:
            pass
        try:
            proc.wait(timeout=10)
        except subprocess.TimeoutExpired:
            os.killpg(proc.pid, 9)
            proc.wait(timeout=10)
    # helpers (GPU, network, renderers) share the process group
    try:
        os.killpg(proc.pid, 15)
    except (ProcessLookupError, PermissionError):
        pass


class WrongSize(RuntimeError):
    pass


def finish(raw: Path, dest: Path, w: int, h: int, dsf: int) -> int:
    im = Image.open(raw)
    im.load()
    want = (w * dsf, h * dsf)
    if im.size != want:
        # Never pad or crop: that would ship a flat strip or a cut edge.
        raise WrongSize(f"{dest.name}: Chrome returned {im.size[0]}×{im.size[1]}, expected {want[0]}×{want[1]}")
    im = im.convert("RGB")  # screenshots are opaque; drop the alpha channel
    im.save(dest, "PNG", optimize=True)
    return dest.stat().st_size


ATTEMPTS = 3  # new headless occasionally sizes the window a few px off


def main(argv: list[str]) -> None:
    names = argv or DEFAULT_SHOTS
    unknown = [n for n in names if n not in SHOTS]
    if unknown:
        sys.exit(f"unknown shot(s): {', '.join(unknown)}; known: {', '.join(SHOTS)}")

    chrome = find_chrome()
    OUT.mkdir(parents=True, exist_ok=True)
    httpd = start_server()
    port = httpd.server_address[1]
    profile = tempfile.mkdtemp(prefix="voxy-shots-profile-")
    scratch = Path(tempfile.mkdtemp(prefix="voxy-shots-"))
    print(f"serving {ROOT} on http://127.0.0.1:{port}/")
    total = 0
    try:
        for name in names:
            w, h, dsf = SHOTS[name]
            url = f"http://127.0.0.1:{port}/{PAGE}?shot={name}"
            raw = scratch / f"{name}.raw.png"
            t0 = time.time()
            for attempt in range(1, ATTEMPTS + 1):
                capture(chrome, profile, url, w, h, dsf, raw)
                try:
                    size = finish(raw, OUT / f"{name}.png", w, h, dsf)
                    break
                except WrongSize as e:
                    print(f"  !! {e} (attempt {attempt} of {ATTEMPTS})", file=sys.stderr)
                    if attempt == ATTEMPTS:
                        raise
            total += size
            print(f"  {name + '.png':<20} {w * dsf}×{h * dsf}  {size / 1e6:5.2f} MB  ({time.time() - t0:.1f} s)")
    finally:
        httpd.shutdown()
        httpd.server_close()
        shutil.rmtree(profile, ignore_errors=True)
        shutil.rmtree(scratch, ignore_errors=True)
    print(f"done → {OUT.relative_to(ROOT)}/  total {total / 1e6:.2f} MB")


if __name__ == "__main__":
    main(sys.argv[1:])
