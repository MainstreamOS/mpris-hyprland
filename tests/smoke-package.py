#!/usr/bin/env python3
"""Stage package() locally; never run pacman or the system scriptlets."""
from pathlib import Path
import json
import os
import shutil
import subprocess
import tempfile
import zipfile
root=Path(__file__).resolve().parents[1]
with tempfile.TemporaryDirectory(prefix='mpris-package-test-') as temporary:
    base=Path(temporary)
    source=base/'source'/root.name
    destination=base/'package'
    # Keep release fixtures private: signed bundles in dist/ must stay untouched.
    for directory in ('dist','packaging','scripts','extension'):
        shutil.copytree(root/directory, source/directory)
    for name in ('README.md','LICENSE','host/target/release/mpris-hyprland-host'):
        target=source/name
        target.parent.mkdir(parents=True,exist_ok=True)
        shutil.copy2(root/name,target)
    version=json.loads((root/'extension/manifest.json').read_text())['version']
    command=['bash','-ec','source "$1/PKGBUILD"; srcdir="$2"; pkgdir="$3"; package','package-test',str(root),str(source.parent),str(destination)]
    subprocess.run(command,check=True,env=dict(os.environ,MPRIS_HYPRLAND_TEST_BUILD='1'))
    assert not (destination/'etc/zen/policies/policies.json').exists()
    assert not (destination/'etc/firefox/policies/policies.json').exists()
    assert (destination/'usr/lib/mpris-hyprland/manage-policies.py').exists()
    assert (destination/'usr/lib/mpris-hyprland/policies/zen.json').exists()
    assert (destination/'etc/opt/chrome/native-messaging-hosts/io.github.mainstreamos.firefox_mpris_hyprland.json').exists()
    with zipfile.ZipFile(destination/'usr/share/mpris-hyprland/mpris-hyprland.xpi') as archive:
        assert json.loads(archive.read('manifest.json'))['version']==version
    # A completed release must pass, while an incomplete one must be rejected.
    ready=all(json.loads(zipfile.ZipFile(source/'dist'/f'mpris-hyprland.{suffix}').read('manifest.json'))['version']==version for suffix in ('xpi','crx'))
    result=subprocess.run(command,env=dict(os.environ,MPRIS_HYPRLAND_TEST_BUILD='0'),capture_output=True,text=True)
    assert (result.returncode==0)==ready,result.stderr
    # Always test a stale bundle, including after both real bundles are updated.
    with zipfile.ZipFile(source/'dist/mpris-hyprland.xpi','w') as archive:
        archive.writestr('manifest.json',json.dumps({'version':'0.0.0'}))
    result=subprocess.run(command,env=dict(os.environ,MPRIS_HYPRLAND_TEST_BUILD='0'),capture_output=True,text=True)
    assert result.returncode!=0 and 'Signed Firefox bundle does not match' in result.stderr
print('Package staging passed; policies are not owned, Chrome host is registered, stale signed releases are rejected.')
