#!/usr/bin/env python3
"""Build every app and tray icon from the artwork in src/assets/icons/source/.

    python3 src/tools/build-icons.py

Writes, under src/assets/icons/:
  source.png          1024² colour icon (the artwork clipped to the macOS squircle)
  icon.icns / .ico / sized PNGs   via `tauri icon`
  tray-template.png   monochrome menu-bar glyph (black on transparent, template image)
  tray.png            coloured tray icon (Windows / Linux notification area)
and src/ui/app-icon.png for the app window.
Needs Google Chrome / Edge (headless canvas) and Node (npx).
"""
import base64
import html
import json
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
ICONS = ROOT / "src" / "assets" / "icons"
SOURCE = ICONS / "source"
PAGE = ROOT / "src" / "tools" / "icon-from-artwork.html"
CHROMES = [
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
    r"C:\Program Files\Google\Chrome\Application\chrome.exe",
    r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
    "google-chrome", "chromium",
]


def find_chrome():
    for c in CHROMES:
        if os.path.isfile(c) or shutil.which(c):
            return c
    sys.exit("Chrome / Edge not found")


def render_artwork():
    """Return (icon_png, tray_png) bytes from the source artwork."""
    url = f"{PAGE.as_uri()}?color={(SOURCE / 'coquou-color.webp').as_uri()}&mono={(SOURCE / 'coquou-mono.png').as_uri()}"
    dom = subprocess.run(
        [find_chrome(), "--headless=new", "--disable-gpu", "--allow-file-access-from-files",
         "--virtual-time-budget=20000", "--dump-dom", url],
        check=True, capture_output=True, text=True,
    ).stdout
    m = re.search(r'<pre id="out">(.*?)</pre>', dom, re.S)
    if not m or not m.group(1).strip():
        sys.exit("icon page produced no output")
    out = json.loads(html.unescape(m.group(1)))
    if "error" in out:
        sys.exit("icon processing failed: " + out["error"])
    decode = lambda data_url: base64.b64decode(data_url.split(",", 1)[1])
    return decode(out["icon"]), decode(out["tray"])


def main():
    icon, tray = render_artwork()
    src = ICONS / "source.png"
    src.write_bytes(icon)
    (ICONS / "tray-template.png").write_bytes(tray)
    npx = "npx.cmd" if os.name == "nt" else "npx"
    subprocess.run([npx, "-y", "@tauri-apps/cli@2", "icon", str(src), "-o", str(ICONS)], check=True, cwd=ROOT)
    # `tauri icon` also emits mobile icon sets; this is a desktop app.
    for extra in ("android", "ios"):
        shutil.rmtree(ICONS / extra, ignore_errors=True)
    shutil.copy(ICONS / "64x64.png", ICONS / "tray.png")
    shutil.copy(ICONS / "128x128@2x.png", ROOT / "src" / "ui" / "app-icon.png")
    print(f"icons built → {ICONS.relative_to(ROOT)}")


if __name__ == "__main__":
    main()
