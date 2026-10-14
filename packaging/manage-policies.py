#!/usr/bin/env python3
"""Merge only this extension's policy, without owning the browser's policy file."""
import argparse
import json
import os
from pathlib import Path
import stat
import tempfile

EXTENSION_ID = "firefox-mpris-hyprland@MainstreamOS"
ENTRY = {"installation_mode": "force_installed", "install_url": "file:///usr/share/mpris-hyprland/mpris-hyprland.xpi"}
PATHS = ("etc/firefox/policies/policies.json", "etc/zen/policies/policies.json")
TEMPLATES = Path(__file__).resolve().parent / "policies"


def atomic_write(path, payload):
    path.parent.mkdir(parents=True, exist_ok=True)
    previous = path.stat() if path.exists() else None
    fd, temporary = tempfile.mkstemp(prefix=".mpris-policy-", dir=path.parent)
    try:
        with os.fdopen(fd, "wb") as stream:
            stream.write(payload)
            os.fchmod(stream.fileno(), stat.S_IMODE(previous.st_mode) if previous else 0o644)
            if previous and os.geteuid() == 0:
                os.fchown(stream.fileno(), previous.st_uid, previous.st_gid)
        os.replace(temporary, path)
    finally:
        Path(temporary).unlink(missing_ok=True)


def update(root, remove=False):
    for relative in PATHS:
        path = root / relative
        browser = relative.split("/")[1]
        backup = root / "var/lib/mpris-hyprland" / (browser + "-upgrade-policy.json")
        if not remove and not path.exists():
            # A file here replaces the browser's own distribution policies, so a new
            # one starts from the browser's baseline rather than from nothing.
            seed = backup if backup.exists() else TEMPLATES / (browser + ".json")
            if seed.exists():
                atomic_write(path, seed.read_bytes())
        try:
            data = json.loads(path.read_text()) if path.exists() else {}
            if not isinstance(data, dict):
                raise ValueError("policy document is not an object")
            policies = data.get("policies", {})
            if not isinstance(policies, dict):
                raise ValueError("policies is not an object")
            settings = policies.get("ExtensionSettings", {})
            if not isinstance(settings, dict):
                raise ValueError("ExtensionSettings is not an object")
            existing = settings.get(EXTENSION_ID)
            if existing is not None and existing != ENTRY:
                print(f"mpris-hyprland: preserving administrator override in {path}")
                continue
            if remove:
                if existing is None:
                    continue
                del settings[EXTENSION_ID]
                if not settings:
                    policies.pop("ExtensionSettings", None)
                if not policies:
                    data.pop("policies", None)
            else:
                settings[EXTENSION_ID] = dict(ENTRY)
                policies["ExtensionSettings"] = settings
                data["policies"] = policies
            if data:
                atomic_write(path, (json.dumps(data, indent=2) + "\n").encode())
            else:
                path.unlink(missing_ok=True)
            backup.unlink(missing_ok=True)
        except (OSError, ValueError) as error:
            print(f"mpris-hyprland: leaving policy unchanged: {path}: {error}")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=["install", "remove"])
    parser.add_argument("--root", type=Path, default=Path("/"))
    args = parser.parse_args()
    update(args.root, args.action == "remove")
