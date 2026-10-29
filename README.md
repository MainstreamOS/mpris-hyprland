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

Release packaging rejects missing or outdated signed bundles. An unsigned
development package requires `MPRIS_HYPRLAND_TEST_BUILD=1`; its Firefox extension
can be loaded manually via `about:debugging` and drops on browser restart.

Other forks (LibreWolf, Floorp, Waterfox) ship their own hardening policy in
`distribution/policies.json`; the `/etc` policy overrides it, so rather than
clobber their settings the package doesn't write `/etc/<fork>/policies.json` for
them — copy the `ExtensionSettings` block from `/etc/zen/policies/policies.json`
into that fork's existing policy file to enable auto-install there.

### Signing for Firefox (required for each extension release)

The Firefox manifest declares the page URLs, media metadata/artwork, and playback
activity sent to the local native host and exposed over the user's session D-Bus.
Mozilla includes native messaging in its data-transmission declaration even
though this is a local desktop bridge, so `required: ["none"]` would be inaccurate.
Firefox 140 or later is required to show the browser's built-in consent prompt.
The extension does not send analytics or upload this metadata to a developer-run
service.

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
won't re-sign a version it's already seen). The signed bundle's version must
match `pkgver` for a release package to build.

To sign through the website instead of setting API credentials:

1. Build the Firefox upload from the current source:

   ```sh
   python3 scripts/build-extension.py --browser firefox --output build/firefox-release.zip
   ```

2. Open [your existing add-on in the Mozilla Developer Hub](https://addons.mozilla.org/developers/addons/)
   and upload a new version for self-distribution ("On your own"). Upload
   `build/firefox-release.zip`, keeping the existing add-on ID.
3. After Mozilla signs it, download the signed XPI and copy it to
   `dist/mpris-hyprland.xpi`. Do not replace that file with the unsigned ZIP.
4. Commit the signed XPI before tagging the release.

### Packing for Chromium (required for each extension release)

Run from the repository root:

```sh
./scripts/pack-crx.sh
git add dist/mpris-hyprland.crx
git commit -m "release: Chromium bundle <version>"
```

The script requires Chromium and the original, gitignored
`chromium-signing-key.pem`. It checks that the key matches the public key in
`extension/manifest.chromium.json` before generating the CRX. Keep that private
key for future updates so the installed extension retains its ID.

The Firefox and Chromium bundles are separate release artifacts. Changing
`pkgver` does not regenerate either one. Rebuild both when their scripts,
manifests, or icons change. A host-only change can instead bump `pkgrel` while
keeping the existing extension version and signed bundles.

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
| `CanSeek`            | True iff the source exposes a seekable range or seek handler                            |
| `CanPlay`/`CanPause` | True iff a track with content is loaded                |
| `CanGoNext`/`CanGoPrevious` | True iff page registered the corresponding `setActionHandler` |
| `CanRaise`           | True — `Raise` focuses the owning tab                  |

| MPRIS method         | Effect                                                 |
| -------------------- | ------------------------------------------------------ |
| `Play` / `Pause` / `PlayPause` | Calls page's handler if registered, else toggles the active media element |
| `Next` / `Previous`  | Calls page's `setActionHandler('nexttrack' / 'previoustrack')` |
| `Seek(offset)`       | `media.currentTime += offset`                          |
| `SetPosition(trackId, position)` | Validated against the current track, then applied within the seekable range                   |
| `Stop`               | Pauses, resets to 0, reports `Stopped`                 |
| `Raise`              | Focuses the owning tab + window                        |

`Volume`, `Rate`, and `LoopStatus` are writable when the page exposes a real
media element. Element-less Media Sessions keep their transport controls but
reject unsupported setters; their minimum and maximum rates match. `Shuffle`
is omitted because browsers expose no corresponding control. The shell reads
`mainstream:capabilitiesVersion` and the `mainstream:canSetVolume`,
`mainstream:canSetRate`, `mainstream:canSetLoop`, and `mainstream:canShuffle`
metadata flags to hide unavailable controls.

The browser/site Identity follows the current representative tab. Embedded
image artwork is decoded into private, content-addressed files under
`$XDG_CACHE_HOME/mpris-hyprland/artwork` (or `~/.cache`), allowing ordinary
MPRIS clients to load it without understanding data URLs. Invalid or oversized
artwork candidates are skipped before choosing another image, poster or favicon.

`mpris:trackid` changes on a
genuine track change (e.g. YouTube autoplay) so clients reset Position.

## Development and checks

Run `cargo test --locked --manifest-path host/Cargo.toml`,
`cargo clippy --locked --manifest-path host/Cargo.toml -- -D warnings`,
`node --test tests/extension.test.cjs`, and
`python3 -m unittest discover -s tests -p 'test_*.py'`.
After building the release host, `python3 tests/smoke-install.py` exercises all
native-manifest targets in a disposable directory and
`dbus-run-session -- python3 tests/smoke-host.py host/target/release/mpris-hyprland-host`
checks the real native host without publishing test players to your desktop.

For temporary Firefox/Zen testing, build an unsigned XPI with:

```sh
python3 scripts/build-extension.py --browser firefox --output build/test.xpi
```

Load it through `about:debugging` → This Firefox → Load Temporary Add-on.
Use a separate browser profile to keep your normal extension installation intact.
For Chromium, pass `--browser chromium`, unpack the ZIP, and load its directory
through `chrome://extensions` → Developer mode → Load unpacked.
The user installer registers Chrome, Brave, Vivaldi, Edge, and Opera manifests
as well as Firefox-family browsers and Chromium. Chromium-family native ports
use the same host, but the extension manifest differs from Firefox's.

## Releases and package integration

All package recipes target a version tag, not a moving branch. Release the
matching `v<version>` tag after signing the Firefox XPI and packing the Chromium
CRX. Keep the host, both extension manifests, and all three package recipes at
the same version. Release packages reject stale signed bundles. Set
`MPRIS_HYPRLAND_TEST_BUILD=1` only for local unsigned package smoke tests.
A package source override is available through `MPRIS_HYPRLAND_REPO_URL` and
`MPRIS_HYPRLAND_REF` for explicit development builds.

For example, once both 0.2.3 bundles are committed, create and publish `v0.2.3`
on that commit, then rebuild the package. A recipe pointing at `v0.2.3` cannot
fetch its source until that tag exists on the remote. Do not tag a release
while either bundle still contains the previous version.

The package merges only this extension's `ExtensionSettings` entry into the
Firefox and Zen policies. It preserves unrelated settings and administrator
overrides, and removes only its own entry on uninstall. Upgrading from the older
whole-policy package preserves the previous file before pacman removes it.

## Diagnostics

The host reports framing and player-update failures on stderr, which the browser
can forward when launched from a terminal. Routine playback is quiet. There is
no file logger or `mprisDebug()` console API. Debug the WebExtension through
`about:debugging` or Chromium's extension service-worker inspector.

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
the extension inspector. Strict-CSP pages are fine now — the content script reads the
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
