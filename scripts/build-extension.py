#!/usr/bin/env python3
"""Build an unsigned development extension from an explicit file list."""
import argparse
from pathlib import Path
import zipfile

ROOT = Path(__file__).resolve().parents[1]

def build(browser, output):
    files = ["background.js", "content.js", "icons/icon-48.png", "icons/icon-96.png"]
    manifest = "manifest.json"
    if browser == "chromium":
        manifest = "manifest.chromium.json"
        files.append("content-main.js")
    output.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(output, "w", zipfile.ZIP_DEFLATED) as archive:
        archive.write(ROOT / "extension" / manifest, "manifest.json")
        for name in files:
            archive.write(ROOT / "extension" / name, name)

if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--browser", choices=["firefox", "chromium"], default="firefox")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    build(args.browser, args.output)
