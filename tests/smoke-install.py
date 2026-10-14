#!/usr/bin/env python3
"""Exercise the installer in a disposable prefix and manifest root."""
import json
from pathlib import Path
import subprocess
import tempfile
import zipfile

root=Path(__file__).resolve().parents[1]
with tempfile.TemporaryDirectory(prefix="mpris-install-test-") as temporary:
    base=Path(temporary)
    args=["bash",str(root/"install.sh"),"--skip-build","--prefix",str(base/"prefix"),"--manifest-root",str(base/"profiles"),"--all-browsers"]
    subprocess.run(args,cwd=root,check=True,stdout=subprocess.DEVNULL)
    manifests=list((base/"profiles").rglob("*.json"))
    assert len(manifests)==10, manifests
    for path in manifests:
        manifest=json.loads(path.read_text())
        assert manifest['path']==str(base/"prefix/bin/mpris-hyprland-host")
        assert 'allowed_origins' in manifest if path.parent.name=='NativeMessagingHosts' else 'allowed_extensions' in manifest
    with zipfile.ZipFile(root/"build/mpris-hyprland.zip") as archive:
        assert 'manifest.chromium.json' not in archive.namelist()
        assert '.amo-upload-uuid' not in archive.namelist()
    subprocess.run(args+["--uninstall"],cwd=root,check=True,stdout=subprocess.DEVNULL)
    assert not list((base/"profiles").rglob("*.json"))
    zen_args=["bash",str(root/"install.sh"),"--skip-build","--prefix",str(base/"prefix"),"--manifest-root",str(base/"profiles"),"--browsers","Zen"]
    subprocess.run(zen_args,cwd=root,check=True,stdout=subprocess.DEVNULL)
    assert (base/"profiles/.mozilla/native-messaging-hosts/io.github.mainstreamos.firefox_mpris_hyprland.json").exists()
    subprocess.run(zen_args+["--uninstall"],cwd=root,check=True,stdout=subprocess.DEVNULL)
print('Installer round trip passed for 11 browsers (10 manifest paths), including Zen-only install.')
