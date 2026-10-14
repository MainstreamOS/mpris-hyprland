#!/usr/bin/env python3
"""Run under dbus-run-session; no test players enter the desktop session."""
import json
import os
from pathlib import Path
import select
import struct
import subprocess
import sys
import time

binary=str(Path(sys.argv[1]).resolve())
p=subprocess.Popen([binary],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE,env=dict(os.environ,MPRIS_HYPRLAND_DESKTOP_ENTRY='zen'))
name=f'org.mpris.MediaPlayer2.firefox.instance{p.pid}_t100'
def send(message):
    payload=json.dumps(message).encode();p.stdin.write(struct.pack('=I',len(payload))+payload);p.stdin.flush()
def call(method,*args):
    return subprocess.check_output(['gdbus','call','--session','--dest',name,'--object-path','/org/mpris/MediaPlayer2','--method',method,*args],stderr=subprocess.DEVNULL,text=True).strip()
def metadata():return call('org.freedesktop.DBus.Properties.Get','org.mpris.MediaPlayer2.Player','Metadata')
def command():
    assert select.select([p.stdout],[],[],2)[0]
    n=struct.unpack('=I',p.stdout.read(4))[0];return json.loads(p.stdout.read(n))
try:
    send(dict(type='update',tabId=100,title='Track',duration=100,position=10,playing=False,canSeek=True))
    for _ in range(60):
        try: data=metadata();break
        except subprocess.CalledProcessError:time.sleep(.05)
    else:raise AssertionError('No player appeared')
    call('org.mpris.MediaPlayer2.Player.SetPosition','/stale/track','99000000')
    assert not select.select([p.stdout],[],[],.1)[0]
    track='/io/github/mainstreamos/mpris_hyprland/w100/f0/n0'
    assert track in data
    call('org.mpris.MediaPlayer2.Player.SetPosition',track,'20000000')
    assert command()['value']==20
    huge=b'x'*(1024*1024+1)
    p.stdin.write(struct.pack('=I',len(huge))+huge);p.stdin.flush()
    send(dict(type='update',tabId=100,title='Survived oversized input',duration=100,position=15,playing=False,canSeek=True))
    for _ in range(60):
        assert p.poll() is None
        if 'Survived oversized input' in metadata():break
        time.sleep(.05)
    else:raise AssertionError('host did not recover from oversized frame')
    print('Private D-Bus smoke passed: stale seek ignored, valid seek relayed, oversized frame recovered.')
finally:
    if p.poll() is None:p.terminate()
    try:p.communicate(timeout=3)
    except subprocess.TimeoutExpired:p.kill();p.communicate()
