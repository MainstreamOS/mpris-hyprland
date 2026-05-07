# firefox-mpris-hyprland

Per-tab MPRIS bridge for Firefox — exposes Media Session metadata
(title, artist, position, **YouTube thumbnails**, full duration) on D-Bus for
Hyprland/Wayland status bars (waybar, quickshell, etc.) and `playerctl`.

Firefox ships its own MPRIS implementation since v81, but it's barebones:
one global player, no artwork on most sites, missing position/length, no
per-tab routing. This project gives you the rich, per-tab MPRIS players that
KDE users get from `plasma-browser-integration` — but desktop-agnostic, so
it works the same on bare Hyprland.

## Architecture

```
┌──────────────┐   postMessage   ┌────────────┐   sendMessage   ┌──────────────┐
│  page realm  │ ──────────────► │  content   │ ──────────────► │  background  │
│  inject.js   │ ◄────────────── │   script   │ ◄────────────── │    script    │
└──────────────┘                 └────────────┘                 └──────┬───────┘
   hooks navigator.mediaSession                                        │
   watches <video>/<audio>                                             │ native
                                                                       │ messaging
                                                                       ▼
                                                                ┌──────────────┐
                                                                │  Rust host   │
                                                                │   (zbus)     │
                                                                └──────┬───────┘
                                                                       │ session bus
                                                                       ▼
                                                            org.mpris.MediaPlayer2
                                                                .firefox.instance
                                                                <pid>_t<tabId>
```

Each tab with active media gets its own MPRIS bus name. `playerctl -l`,
waybar's `mpris` module, quickshell's `Mpris` service, etc. all see them as
distinct players, dedup as they wish.

## Supported browsers

Anything Firefox-based that ships unmodified WebExtension + native-messaging
support. `install.sh` auto-detects which forks are on your system and writes
the manifest to each one's per-fork directory:

| Browser    | Manifest path (per-user)                        |
| ---------- | ----------------------------------------------- |
| Firefox    | `~/.mozilla/native-messaging-hosts/`            |
| LibreWolf  | `~/.librewolf/native-messaging-hosts/`          |
| Zen        | `~/.zen/native-messaging-hosts/`                |
| Floorp     | `~/.floorp/native-messaging-hosts/`             |
| Waterfox   | `~/.waterfox/native-messaging-hosts/`           |

The bus name prefix this project emits is always `firefox.instance<...>`
regardless of which fork is running — that's what status bars (waybar,
quickshell, playerctl, etc.) match against to identify the source.

To force a specific set, use `--browsers`:

```sh
./install.sh --browsers Zen,Firefox
./install.sh --all-browsers   # write everywhere, even forks not detected
```

Tor Browser and Mullvad Browser **are not supported** — they ship with
WebExtension native-messaging disabled by design (privacy hardening).

## Install

### Per-user (recommended for development)

```sh
git clone https://github.com/MainstreamOS/firefox-mpris-hyprland
cd firefox-mpris-hyprland
./install.sh
```

This builds the Rust host (`cargo build --release`), installs it to
`~/.local/bin/firefox-mpris-host`, drops the native messaging manifest into
`~/.mozilla/native-messaging-hosts/`, and prints next steps.

Then load the extension into Firefox:

1. Open `about:debugging#/runtime/this-firefox`
2. **Load Temporary Add-on…**
3. Select `extension/manifest.json`

The extension will stay loaded until you restart Firefox. To make it
permanent on stable Firefox you'll need to either self-distribute via
[addons.mozilla.org self-distribution](https://extensionworkshop.com/documentation/publish/self-distribution/),
or use Firefox Developer Edition / Nightly with `xpinstall.signatures.required = false`
in `about:config` and drag-and-drop `build/firefox-mpris-hyprland.zip`
onto a Firefox window.

### Arch package

```sh
makepkg -si
```

Installs the host to `/usr/bin/firefox-mpris-host` and the native messaging
manifest system-wide. The packaged `.zip` ends up under
`/usr/share/firefox-mpris-hyprland/` for manual loading.

### Uninstall

```sh
./install.sh --uninstall
```

…then remove the extension from `about:addons`.

## Verifying

With a YouTube tab playing:

```sh
playerctl -l
# firefox.instance12345_t42

playerctl --player firefox.instance12345_t42 metadata
# Mpris2 Title:     Some Music Video
# Mpris2 Artist:    [Channel Name]
# mpris:artUrl      https://i.ytimg.com/vi/.../maxresdefault.jpg
# mpris:length      234500000
# xesam:url         https://www.youtube.com/watch?v=...
```

Your Hyprland media controls (e.g. the `quickshell ii` config in
`dots-hyprland`) will pick the player up automatically — `MprisController.players`
sees it the same way it sees any other MPRIS source.

## Compatibility with Firefox's built-in MPRIS

Firefox's own MPRIS (`org.mpris.MediaPlayer2.firefox.instance<pid>`) will
coexist. The dedup logic in most status bars (including the dots-hyprland
quickshell config) prefers the player with non-empty cover art — that'll be
this one. If you want to silence the built-in entirely:

```
about:config → widget.gtk.legacy-mpris.enabled = false
about:config → media.hardwaremediakeys.enabled = false   (also disables media keys)
```

You usually don't need to.

## What's exposed

| MPRIS field          | Source                                                 |
| -------------------- | ------------------------------------------------------ |
| `xesam:title`        | `navigator.mediaSession.metadata.title`                |
| `xesam:artist`       | `navigator.mediaSession.metadata.artist`               |
| `xesam:album`        | `navigator.mediaSession.metadata.album`                |
| `mpris:artUrl`       | Largest entry from `metadata.artwork` (YouTube fallback uses `i.ytimg.com/vi/<id>/maxresdefault.jpg`) |
| `mpris:length`       | Active `<video>`/`<audio>` `duration`                  |
| `Position`           | `<video>`/`<audio>` `currentTime`, interpolated against wall clock |
| `xesam:url`          | Page URL                                               |
| `PlaybackStatus`     | Playing if media is playing, otherwise Paused/Stopped  |
| `Volume`             | Active media element volume                            |
| `CanSeek`            | True iff duration is finite                            |
| `CanGoNext`/`CanGoPrevious` | True iff page registered the corresponding `setActionHandler` |

| MPRIS method         | Effect                                                 |
| -------------------- | ------------------------------------------------------ |
| `Play` / `Pause` / `PlayPause` | Calls page's handler if registered, else toggles the active media element |
| `Next` / `Previous`  | Calls page's `setActionHandler('nexttrack' / 'previoustrack')` |
| `Seek(offset)`       | `media.currentTime += offset`                          |
| `SetPosition(_, position)` | `media.currentTime = position`                   |
| `Stop`               | Pauses and resets to 0                                 |

`Volume` is read/write.

## Troubleshooting

**No player shows up.** Open a terminal and run Firefox from it:

```sh
firefox
```

You should see lines like `[mpris-host] connecting to native host: …` in the
console (or in the WebExtension's Inspect view at `about:debugging`). The
host's stderr is forwarded to the Firefox parent process — see if there are
errors.

**Player shows up but methods don't work.** Check the extension's background
script console (`about:debugging` → Inspect on the extension). The content
script may have failed to inject due to a strict CSP page; that's fine for
most sites but pages that block `moz-extension://` URLs in `script-src`
won't work.

**Playerctl shows the player twice.** That's Firefox's built-in MPRIS plus
ours. Status bars usually dedup. To silence the built-in see the section
above.

## License

MIT — see [LICENSE](LICENSE).

## Disclaimer

This is an independent, community-maintained project. It is **not affiliated
with, endorsed by, or sponsored by Mozilla**. "Firefox" is a trademark of the
Mozilla Foundation; this project uses the name nominatively to describe what
it integrates with.
