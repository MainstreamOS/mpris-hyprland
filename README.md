# mpris-hyprland

Per-window MPRIS bridge for Firefox — exposes Media Session metadata
(title, artist, position, **YouTube thumbnails**, full duration) on D-Bus for
Hyprland/Wayland status bars (waybar, quickshell, etc.) and `playerctl`.

Firefox ships its own MPRIS implementation since v81, but it's barebones:
one global player, no artwork on most sites, missing position/length. This
project gives you a rich MPRIS player **per browser window** — reflecting that
window's active media (the playing tab, otherwise the most recently active) —
with the metadata KDE users get from `plasma-browser-integration`, but
desktop-agnostic so it works the same on bare Hyprland. (Internally it tracks
every media tab/frame and consolidates to one player per window; switching
which tab is playing switches the window player's content.)

## Architecture

```
┌────────────────────┐   sendMessage   ┌──────────────┐
│   content script    │ ──────────────► │  background  │
│  (per frame, Xray)  │ ◄────────────── │    script    │
└────────────────────┘                 └──────┬───────┘
  reads navigator.mediaSession                 │ background consolidates
  via window.wrappedJSObject                   │ all frames → one player
  watches <video>/<audio> directly             │ per window, then native
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
                                        <pid>_t<windowId>
```

A single content script does the whole page side. It reaches the page's Media
Session through Firefox's Xray (`window.wrappedJSObject` + `exportFunction`) —
**no `<script>` is injected into the page**, so strict-CSP sites (github, x.com)
work too. `<video>`/`<audio>` state is read directly from the content-script DOM.

The background script tracks every media tab/frame but exposes **one MPRIS
player per browser window** (`…instance<pid>_t<windowId>` — the id is a window
id; the `_t` prefix is retained for status-bar matching). Each window's player
reflects its active media: the playing tab, otherwise the most recently active;
commands route to whichever tab currently represents the window.
`playerctl -l`, waybar's `mpris` module, quickshell's `Mpris` service, etc. see
one player per window (typically just one, unless you run multiple browser
windows).

**Lightness:** a page with no media installs zero recurring timers (just a cheap
`MutationObserver`); a paused player has zero timers; one actively-playing
element runs a single ~1s position ticker. During steady playback the host emits
**no** D-Bus signals (Position is polled by clients, not pushed) — it only emits
`PropertiesChanged` for the specific properties that actually change.

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
git clone https://github.com/MainstreamOS/mpris-hyprland
cd mpris-hyprland
./install.sh
```

This builds the Rust host (`cargo build --release`), installs it to
`~/.local/bin/mpris-hyprland-host`, drops the native messaging manifest into
`~/.mozilla/native-messaging-hosts/`, and prints next steps.

Then load the extension into Firefox:

1. Open `about:debugging#/runtime/this-firefox`
2. **Load Temporary Add-on…**
3. Select `extension/manifest.json`

The extension will stay loaded until you restart Firefox. To make it
permanent on stable Firefox you'll need to either self-distribute via
[addons.mozilla.org self-distribution](https://extensionworkshop.com/documentation/publish/self-distribution/),
or use Firefox Developer Edition / Nightly with `xpinstall.signatures.required = false`
in `about:config` and drag-and-drop `build/mpris-hyprland.zip`
onto a Firefox window.

### Arch package (auto-installs the extension, no manual step)

```sh
makepkg -si
```

Installs:

- the host → `/usr/bin/mpris-hyprland-host`
- the native-messaging manifest (system-wide) → `/usr/lib/mozilla/native-messaging-hosts/`
- the extension as an `.xpi` → `/usr/share/mpris-hyprland/mpris-hyprland.xpi`
- browser **policies** that force-install that `.xpi` → `/etc/zen/policies/policies.json`
  and `/etc/firefox/policies/policies.json`

**Extension signing is required for policy auto-install — on Firefox *and* on
Zen.** Despite Zen defaulting `xpinstall.signatures.required=false`, it still
enforces signing for policy-installed extensions (an unsigned `.xpi` fails with
`ERROR_SIGNEDSTATE_REQUIRED`). So the package ships the **AMO-signed** build
(`dist/mpris-hyprland.xpi`, see [Signing](#signing-for-firefox-one-time-optional)),
which the `PKGBUILD` prefers; with it present, the extension auto-installs on
next launch — no `about:debugging` step. Restart the browser and check
`about:policies#active` / `about:addons`.

If only the unsigned zip is built (no signed `dist/` build), the extension
**won't** auto-install anywhere; load it manually via `about:debugging` (it
drops on browser restart).

Other forks (LibreWolf, Floorp, Waterfox) ship their own hardening policy in
`distribution/policies.json`; the `/etc` policy overrides it, so rather than
clobber their settings the package doesn't write `/etc/<fork>/policies.json` for
them — copy the `ExtensionSettings` block from `/etc/zen/policies/policies.json`
into that fork's existing policy file to enable auto-install there.

### Signing for Firefox (one-time, optional)

To make the policy auto-install work on **vanilla Firefox** too (it rejects
unsigned extensions), sign the extension once per release with Mozilla's
self-distribution flow. The signed `.xpi` then installs everywhere — Firefox and
all forks — via the same policy.

```sh
# 1. Create a free account at addons.mozilla.org and generate an API
#    key + secret: https://addons.mozilla.org/developers/addon/api/key/
export AMO_JWT_ISSUER='user:XXXXXX:YY'   # the "JWT issuer" (API key)
export AMO_JWT_SECRET='…'                 # the "JWT secret"

# 2. Sign (uses web-ext via npx; no global install needed).
./scripts/sign.sh
#    → produces dist/mpris-hyprland.xpi (a Mozilla-signed, self-hosted build)

# 3. Commit it; the package ships dist/mpris-hyprland.xpi when present.
git add dist/mpris-hyprland.xpi && git commit -m "release: signed xpi <version>"
```

The credentials are yours and stay in your environment — never commit them.
Bump `extension/manifest.json`'s `version` before each new signing run (AMO
won't re-sign a version it's already seen). The `PKGBUILD` automatically prefers
`dist/mpris-hyprland.xpi` over the unsigned zip when it exists.

### Uninstall

```sh
./install.sh --uninstall
```

…then remove the extension from `about:addons`.

## Verifying

With a YouTube tab playing:

```sh
playerctl -l
# firefox.instance12345_t1     (one per browser window; the id is the window id)

playerctl --player firefox.instance12345_t1 metadata
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

Firefox's own MPRIS (`org.mpris.MediaPlayer2.firefox.instance<pid>`) is a
single global player with sparse metadata. It will coexist; most status bars
dedup and prefer the player with cover art (this one). To silence the built-in:

```
about:config → media.hardwaremediakeys.enabled = false   (the main switch; also
                                                           stops the browser
                                                           grabbing media keys)
about:config → widget.gtk.legacy-mpris.enabled = false    (secondary, older path)
```

**Do not** disable `dom.media.mediasession.enabled` — that's the JS Media
Session API this extension reads for title/artist/artwork; turning it off
breaks this extension too. Disabling the *built-in MPRIS* prefs above does not
affect the extension (different subsystem).

For a fully isolated test, run a dedicated profile with those two prefs in a
`user.js`. With more than one browser window (multiple players present), global
media-key routing depends on your desktop's active-player picker (e.g.
`playerctld`).

## What's exposed

| MPRIS field          | Source                                                 |
| -------------------- | ------------------------------------------------------ |
| `xesam:title`        | `navigator.mediaSession.metadata.title`                |
| `xesam:artist`       | `navigator.mediaSession.metadata.artist`               |
| `xesam:album`        | `navigator.mediaSession.metadata.album`                |
| `mpris:artUrl`       | Largest entry from `metadata.artwork` (YouTube fallback uses `i.ytimg.com/vi/<id>/maxresdefault.jpg`) |
| `mpris:length`       | `<video>`/`<audio>` `duration`, falling back to its `seekable` range and the page's `setPositionState` duration (so YouTube reports full length even when the element momentarily reads 0/NaN/∞) |
| `Position`           | `<video>`/`<audio>` `currentTime`, interpolated against wall clock |
| `xesam:url`          | Page URL                                               |
| `PlaybackStatus`     | Playing if media is playing, otherwise Paused/Stopped  |
| `Volume`             | Active media element volume (muted reports 0)          |
| `Rate`               | `media.playbackRate` (read/write, 0.25–4.0)            |
| `LoopStatus`         | `media.loop` (read/write; `None` / `Track`)            |
| `CanSeek`            | True iff duration is finite                            |
| `CanPlay`/`CanPause` | True iff a track with content is loaded                |
| `CanGoNext`/`CanGoPrevious` | True iff page registered the corresponding `setActionHandler` |
| `CanRaise`           | True — `Raise` focuses the owning tab                  |

| MPRIS method         | Effect                                                 |
| -------------------- | ------------------------------------------------------ |
| `Play` / `Pause` / `PlayPause` | Calls page's handler if registered, else toggles the active media element |
| `Next` / `Previous`  | Calls page's `setActionHandler('nexttrack' / 'previoustrack')` |
| `Seek(offset)`       | `media.currentTime += offset`                          |
| `SetPosition(_, position)` | `media.currentTime = position`                   |
| `Stop`               | Pauses, resets to 0, reports `Stopped`                 |
| `Raise`              | Focuses the owning tab + window                        |

`Volume`, `Rate`, and `LoopStatus` are read/write. `mpris:trackid` changes on a
genuine track change (e.g. YouTube autoplay) so clients reset Position.

## Debug logging

Two layers, one switch.

### 1. Rust host (file + stderr)

Always writes to `${XDG_STATE_HOME:-~/.local/state}/mpris-hyprland-host/host.log`
(size-capped at 10 MiB, rotated to `host.log.1`), and to stderr when the
browser forwards it (run the browser from a terminal to see it inline).

Default filter is `mpris_hyprland_host=info,warn` — lifecycle, player
create/remove, and D-Bus method calls. Routine per-message frames and the
per-update changed-property set are at `debug`; position interpolation detail
is at `trace`. Crank it via `RUST_LOG` before launching the browser:

```sh
RUST_LOG=mpris_hyprland_host=debug zen-browser   # + every frame & changed-prop set
RUST_LOG=mpris_hyprland_host=trace zen-browser   # + position/interpolation detail
RUST_LOG=trace zen-browser                       # + zbus internals (very noisy)
```

A useful test signal lives at `debug`: each update logs the exact
`changed: [Metadata,PlaybackStatus,…]` set it emitted — during steady playback
that line shouldn't appear at all (proof the no-traffic path works).

### 2. Extension (background + content), one flag

Open `about:debugging#/runtime/this-firefox` → "MPRIS for Hyprland" →
**Inspect**. The console shows `[mpris-bg …]` (background) and `[mpris-cs …]`
(content) lines. One flag drives both layers, persisted in `storage.local` and
pushed live to already-loaded content scripts (no reload). Toggle from the
Inspect console:

```js
mprisDebug(true)    // verbose, all layers
mprisDebug(false)   // quiet (default)
```

`DEBUG` defaults to **off** so the extension is silent (and doesn't log page
URLs) in normal use. With it on, content-script lines note timer start/stop and
bfcache restores — so you can watch the "zero timers when idle" behavior
directly.

## Troubleshooting

**No player shows up.** Open a terminal and run Firefox from it:

```sh
firefox
```

You should see lines like `[mpris-bg ...]` in the WebExtension's Inspect
view at `about:debugging`, and timestamped log lines from
`mpris-hyprland-host` in the terminal. The host's stderr is forwarded to
the browser's parent process — that's where you'll see what the host saw
and did.

**Player shows up but methods don't work.** Check the extension's background
script console (`about:debugging` → Inspect on the extension) with
`mprisDebug(true)`. Strict-CSP pages are fine now — the content script reads the
page through Firefox's Xray and injects no `<script>` — but a page that doesn't
register Media Session action handlers and has no controllable `<video>`/
`<audio>` element (rare) can't be driven.

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
