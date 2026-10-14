#!/usr/bin/env python3
"""Stage package() locally; never run pacman or the system scriptlets."""
from pathlib import Path
import json
import os
import subprocess
import tempfile
import zipfile
root=Path(__file__).resolve().parents[1]
with tempfile.TemporaryDirectory(prefix='mpris-package-test-') as temporary:
    destination=Path(temporary)
    command=['bash','-c','source "$1/PKGBUILD"; srcdir="$2"; pkgdir="$3"; package','package-test',str(root),str(root.parent),str(destination)]
    subprocess.run(command,check=True,env=dict(os.environ,MPRIS_HYPRLAND_TEST_BUILD='1'))
    assert not (destination/'etc/zen/policies/policies.json').exists()
    assert not (destination/'etc/firefox/policies/policies.json').exists()
    assert (destination/'usr/lib/mpris-hyprland/manage-policies.py').exists()
    assert (destination/'usr/lib/mpris-hyprland/policies/zen.json').exists()
    assert (destination/'etc/opt/chrome/native-messaging-hosts/io.github.mainstreamos.firefox_mpris_hyprland.json').exists()
    with zipfile.ZipFile(destination/'usr/share/mpris-hyprland/mpris-hyprland.xpi') as archive:
        assert json.loads(archive.read('manifest.json'))['version']=='0.2.3'
    result=subprocess.run(command,env=dict(os.environ,MPRIS_HYPRLAND_TEST_BUILD='0'),capture_output=True,text=True)
    assert result.returncode!=0 and 'Signed Firefox bundle does not match' in result.stderr
print('Package staging passed; policies are not owned, Chrome host is registered, stale signed releases are rejected.')
