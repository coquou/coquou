#!/usr/bin/env python3
"""Build the app + tray icons.

    python3 tools/build-icons.py                   # from tools/icons/source/ artwork (default)
    python3 tools/build-icons.py <concept id>      # from a drawn concept in tools/icons/icons.js

Writes src-tauri/icons/source.png (1024²) and, through `tauri icon`, every
platform size (icns, ico, PNGs); plus the tray icons:
  icons/tray-template.png  monochrome glyph (macOS menu bar, template image)
  icons/tray.png           coloured icon (Windows / Linux notification area)
Needs Google Chrome (headless) and Node (npx).
"""
import json
import os
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ICONS = ROOT / "src-tauri" / "icons"
CHROME = next((c for c in [
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
    r"C:\Program Files\Google\Chrome\Application\chrome.exe",
    "google-chrome", "chromium",
] if os.path.isfile(c) or shutil.which(c)), None)


def concepts():
    js = (ROOT / "tools" / "icons" / "icons.js").read_text(encoding="utf-8")
    out = subprocess.run(["node", "-e", js + "\nconsole.log(JSON.stringify(module.exports))"],
                         check=True, capture_output=True, text=True).stdout
    return {c["id"]: c for c in json.loads(out)}


def render(svg, size, dest):
    tmp = Path(tempfile.mkdtemp())
    page = tmp / "i.html"
    page.write_text(f"<!doctype html><style>html,body{{margin:0;background:transparent}}svg{{width:{size}px;height:{size}px;display:block}}</style>{svg}", encoding="utf-8")
    subprocess.run([CHROME, "--headless=new", "--disable-gpu", "--hide-scrollbars", "--force-device-scale-factor=1",
                    f"--window-size={size},{size}", "--default-background-color=00000000",
                    f"--screenshot={dest}", page.as_uri()], check=True, capture_output=True)


SOURCE = ROOT / "tools" / "icons" / "source"


def from_artwork(icon_dest, tray_dest):
    """tools/icons/source/coquou-{color,mono} → squircle icon + tray template."""
    import base64, html as H, re
    page = (ROOT / "tools" / "icons" / "from-images.html").as_uri()
    url = f"{page}?color={(SOURCE / 'coquou-color.webp').as_uri()}&mono={(SOURCE / 'coquou-mono.png').as_uri()}"
    dom = subprocess.run([CHROME, "--headless=new", "--disable-gpu", "--allow-file-access-from-files",
                          "--virtual-time-budget=20000", "--dump-dom", url], check=True, capture_output=True, text=True).stdout
    out = json.loads(H.unescape(re.search(r'<pre id="out">(.*?)</pre>', dom, re.S).group(1)))
    if "error" in out:
        sys.exit("icon processing failed: " + out["error"])
    icon_dest.write_bytes(base64.b64decode(out["icon"].split(",", 1)[1]))
    tray_dest.write_bytes(base64.b64decode(out["tray"].split(",", 1)[1]))


def main():
    if len(sys.argv) > 2:
        sys.exit(__doc__)
    if not CHROME:
        sys.exit("Chrome not found")
    ICONS.mkdir(parents=True, exist_ok=True)
    src = ICONS / "source.png"
    if len(sys.argv) == 2:
        all_ = concepts()
        c = all_.get(sys.argv[1]) or sys.exit(f"unknown concept; pick one of {', '.join(all_)}")
        render(c["icon"], 1024, src)
        render(c["tray"], 64, ICONS / "tray-template.png")
        name = f"concept {c['id']}"
    else:
        from_artwork(src, ICONS / "tray-template.png")
        name = "tools/icons/source artwork"
    npx = "npx.cmd" if os.name == "nt" else "npx"
    subprocess.run([npx, "-y", "@tauri-apps/cli@2", "icon", str(src), "-o", str(ICONS)], check=True, cwd=ROOT / "src-tauri")
    shutil.copy(ICONS / "64x64.png", ICONS / "tray.png")
    # The app window shows the icon too.
    assets = ROOT / "src" / "assets"
    assets.mkdir(exist_ok=True)
    shutil.copy(ICONS / "128x128@2x.png", assets / "app-icon.png")
    # Drop mobile icon sets `tauri icon` also emits.
    for extra in ("android", "ios"):
        shutil.rmtree(ICONS / extra, ignore_errors=True)
    print(f"icons built from {name} → {ICONS.relative_to(ROOT)}")


if __name__ == "__main__":
    main()
