#!/usr/bin/env python3
"""Run under dbus-run-session; no test players enter the desktop session."""
import base64
import json
import os
from pathlib import Path
import select
import struct
import subprocess
import sys
import time
import tempfile

cache=tempfile.TemporaryDirectory(prefix='mpris-art-smoke-')
binary=str(Path(sys.argv[1]).resolve())
p=subprocess.Popen([binary],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE,env=dict(os.environ,MPRIS_HYPRLAND_DESKTOP_ENTRY='zen',XDG_CACHE_HOME=cache.name))
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
    monitor=subprocess.Popen(['gdbus','monitor','--session','--dest',name,'--object-path','/org/mpris/MediaPlayer2'],stdout=subprocess.PIPE,text=True)
    try:
        time.sleep(.15)
        art_bytes=b'\x89PNG\r\n\x1a\n'+b'x'*150000
        art='data:image/png;base64,'+base64.b64encode(art_bytes).decode()
        send(dict(type='update',tabId=100,title='Embedded cover',pageUrl='https://www.example.org/song',artUrl=art,duration=100,position=15,canSeek=True))
        for _ in range(60):
            data=metadata()
            if 'Embedded cover' in data:break
            time.sleep(.05)
        else:raise AssertionError('metadata update failed')
        identity=call('org.freedesktop.DBus.Properties.Get','org.mpris.MediaPlayer2','Identity')
        assert 'Zen (example.org)' in identity,identity
        import re
        from urllib.parse import urlparse, unquote
        url=re.search(r"'mpris:artUrl': <'([^']+)'",data).group(1)
        assert url.startswith('file:///'),url
        assert Path(unquote(urlparse(url).path)).read_bytes()==art_bytes
        for prop,value in [('Volume','<0.2>'),('Rate','<2.0>'),('LoopStatus',"<'Track'>")]:
            try:call('org.freedesktop.DBus.Properties.Set','org.mpris.MediaPlayer2.Player',prop,value)
            except subprocess.CalledProcessError:pass
            else:raise AssertionError(f'unsupported {prop} accepted')
        assert not select.select([p.stdout],[],[],.1)[0]
        assert "'Shuffle':" not in call('org.freedesktop.DBus.Properties.GetAll','org.mpris.MediaPlayer2.Player')
        send(dict(type='update',tabId=100,title='DOM control',duration=100,canSetVolume=True,canSetRate=True,canSetLoop=True))
        for _ in range(60):
            if 'DOM control' in metadata():break
            time.sleep(.05)
        call('org.freedesktop.DBus.Properties.Set','org.mpris.MediaPlayer2.Player','Volume','<0.4>')
        assert command()['action']=='setvolume'
    finally:
        monitor.terminate();signals=monitor.communicate(timeout=3)[0]
    assert 'Identity' in signals and 'example.org' in signals,signals
    print('Private D-Bus smoke passed: framing/seek, dynamic Identity signal, large embedded artwork, unsupported controls, and supported command relay.')
finally:
    if p.poll() is None:p.terminate()
    try:p.communicate(timeout=3)
    except subprocess.TimeoutExpired:p.kill();p.communicate()
    cache.cleanup()
