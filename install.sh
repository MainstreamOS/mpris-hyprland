#!/usr/bin/env bash
# Build and install mpris-hyprland for the current user.
#
# Layout (per-user install — no sudo needed):
#   ~/.local/bin/mpris-hyprland-host
#   ~/.mozilla/native-messaging-hosts/io.github.mainstreamos.firefox_mpris_hyprland.json
#
# After running this script you still need to load the WebExtension into
# Firefox manually — see the section printed at the end. Firefox does not
# permit the host to install an unsigned extension automatically.

set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")"
PROJECT_ROOT="$(pwd)"

# ---------- root guard -----------------------------------------------------
# Browsers run as your normal user, not root. If we install the native
# messaging manifest under /root/<profile-dir>, the browser will never see
# it. Bail out with a clear message instead of silently producing a broken
# install.
if [[ ${EUID:-$(id -u)} -eq 0 ]]; then
    cat <<'EOF' >&2
ERROR: install.sh must NOT be run as root (or under sudo).

Browsers run as your normal user account, so the native-messaging manifest
needs to live under YOUR home directory (e.g. ~/.mozilla/, ~/.zen/, …),
not /root/.

Run again without sudo:
    ./install.sh

If you want a system-wide install for all users, use the Arch package
(makepkg -si) instead — that installs to /usr/bin and
/usr/lib/mozilla/native-messaging-hosts/.
EOF
    exit 1
fi

# ---------- supported browsers ---------------------------------------------
# Native messaging manifest path is per-fork on Linux:
#   Firefox    ~/.mozilla/native-messaging-hosts/
#   LibreWolf  ~/.librewolf/native-messaging-hosts/
#   Zen        ~/.zen/native-messaging-hosts/
#   Floorp     ~/.floorp/native-messaging-hosts/
#   Waterfox   ~/.waterfox/native-messaging-hosts/
#
# Format of each entry:  "Display name|profile-rel-dir|binary-name"
KNOWN_BROWSERS=(
    "Firefox|.mozilla|firefox"
    "LibreWolf|.librewolf|librewolf"
    "Zen|.config/zen|zen-browser"
    "Floorp|.floorp|floorp"
    "Waterfox|.waterfox|waterfox"
    "Chromium|.config/chromium|chromium"
    "Chrome|.config/google-chrome|google-chrome"
    "Brave|.config/BraveSoftware/Brave-Browser|brave-browser"
    "Vivaldi|.config/vivaldi|vivaldi-stable"
    "Edge|.config/microsoft-edge|microsoft-edge"
    "Opera|.config/opera|opera"
)

# ---------- options --------------------------------------------------------

INSTALL_PREFIX="${HOME}/.local"
MANIFEST_ROOT="${HOME}"
HOST_NAME="io.github.mainstreamos.firefox_mpris_hyprland"
SKIP_BUILD=0
ZIP_ONLY=0
UNINSTALL=0
BROWSERS_OPT=""   # comma-separated; empty = auto-detect
INSTALL_ALL=0     # --all-browsers: write manifest for every fork we know about

usage() {
    cat <<EOF
Usage: $0 [options]

Options:
  --prefix DIR        Install binary under DIR/bin (default: \$HOME/.local)
  --manifest-root DIR Home directory for native manifests (default: \$HOME)
  --browsers LIST     Comma-separated list (Firefox,LibreWolf,Zen,Floorp,Waterfox,Chromium,Chrome,Brave,Vivaldi,Edge,Opera).
                      Default: auto-detect every fork that's installed.
  --all-browsers      Install the manifest for every supported fork, even
                      ones not currently detected. Cheap & idempotent.
  --skip-build        Don't run cargo, assume host is already built
  --zip-only          Only build the extension .zip, skip host & manifests
  --uninstall         Remove installed files (host + every fork's manifest)
  -h, --help          Show this help

Files installed:
  \$prefix/bin/mpris-hyprland-host
  ~/.<browser>/native-messaging-hosts/${HOST_NAME}.json   (one per fork)
  ./build/mpris-hyprland.zip                      (load manually in browser)

Supported browsers: Firefox, LibreWolf, Zen, Floorp, Waterfox, Chromium, Chrome, Brave, Vivaldi, Edge, Opera
Requires python3 for extension packaging and native manifests.
EOF
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --prefix) INSTALL_PREFIX="$2"; shift 2 ;;
        --manifest-root) MANIFEST_ROOT="$2"; shift 2 ;;
        --browsers) BROWSERS_OPT="$2"; shift 2 ;;
        --all-browsers) INSTALL_ALL=1; shift ;;
        --skip-build) SKIP_BUILD=1; shift ;;
        --zip-only) ZIP_ONLY=1; shift ;;
        --uninstall) UNINSTALL=1; shift ;;
        -h|--help) usage; exit 0 ;;
        *) echo "unknown option: $1" >&2; usage >&2; exit 2 ;;
    esac
done

BIN_DIR="${INSTALL_PREFIX}/bin"
HOST_BIN="${BIN_DIR}/mpris-hyprland-host"
command -v python3 >/dev/null || { echo "python3 is required" >&2; exit 1; }

# ---------- pick which browsers to install for ----------------------------
#
# Selection rules:
#   - --browsers Foo,Bar      → exactly those (case-insensitive match)
#   - --all-browsers          → every entry in KNOWN_BROWSERS
#   - default (auto-detect)   → any KNOWN_BROWSERS entry whose binary is on
#                                $PATH or whose profile dir already exists.
#                                Falls back to Firefox if none detected, so
#                                a fresh install on a system where the user
#                                hasn't launched a browser yet still works.
#
# Result: TARGETS is an array of "Display|reldir" strings.
TARGETS=()

normalize() { tr '[:upper:]' '[:lower:]' <<<"$1"; }

if [[ -n "$BROWSERS_OPT" ]]; then
    IFS=',' read -ra requested <<< "$BROWSERS_OPT"
    for r in "${requested[@]}"; do
        rn=$(normalize "$r")
        matched=0
        for entry in "${KNOWN_BROWSERS[@]}"; do
            IFS='|' read -r name reldir bin <<< "$entry"
            if [[ "$(normalize "$name")" == "$rn" ]]; then
                TARGETS+=("$name|$reldir")
                matched=1
                break
            fi
        done
        if [[ $matched -eq 0 ]]; then
            echo "warning: unknown browser '$r' — supported: Firefox, LibreWolf, Zen, Floorp, Waterfox" >&2
        fi
    done
elif [[ $INSTALL_ALL -eq 1 ]]; then
    for entry in "${KNOWN_BROWSERS[@]}"; do
        IFS='|' read -r name reldir bin <<< "$entry"
        TARGETS+=("$name|$reldir")
    done
else
    for entry in "${KNOWN_BROWSERS[@]}"; do
        IFS='|' read -r name reldir bin <<< "$entry"
        if command -v "$bin" >/dev/null 2>&1 || [[ -d "$HOME/$reldir" ]]; then
            TARGETS+=("$name|$reldir")
        fi
    done
    if [[ ${#TARGETS[@]} -eq 0 ]]; then
        echo "→ no Firefox-family browser detected; defaulting to Firefox path" >&2
        TARGETS+=("Firefox|.mozilla")
    fi
fi

# ---------- uninstall ------------------------------------------------------

if [[ $UNINSTALL -eq 1 ]]; then
    rm -fv "$HOST_BIN" || true
    # Always sweep every known browser path on uninstall, regardless of
    # auto-detect — leftover manifests from a removed browser are harmless
    # but messy.
    for entry in "${KNOWN_BROWSERS[@]}"; do
        IFS='|' read -r name reldir _bin <<< "$entry"
        case "$name" in Chromium|Chrome|Brave|Vivaldi|Edge|Opera) nm_subdir=NativeMessagingHosts ;; *) nm_subdir=native-messaging-hosts ;; esac
        [[ "$name" == "Zen" ]] && reldir=.mozilla
        nm_path="${MANIFEST_ROOT}/${reldir}/${nm_subdir}/${HOST_NAME}.json"
        [[ -f "$nm_path" ]] && rm -fv "$nm_path"
    done
    echo
    echo "Uninstalled. The browser extension itself is still installed —"
    echo "remove it from about:addons (or about:debugging) if you no longer want it."
    exit 0
fi

# ---------- build extension zip -------------------------------------------

mkdir -p build
EXT_ZIP="${PROJECT_ROOT}/build/mpris-hyprland.zip"
rm -f "$EXT_ZIP"
python3 scripts/build-extension.py --browser firefox --output "$EXT_ZIP"
echo "→ extension package: $EXT_ZIP"

if [[ $ZIP_ONLY -eq 1 ]]; then
    exit 0
fi

# ---------- build host -----------------------------------------------------

if [[ $SKIP_BUILD -eq 0 ]]; then
    echo "→ building host (cargo build --release)…"
    ( cd host && cargo build --release )
fi

HOST_SOURCE="host/target/release/mpris-hyprland-host"
if [[ ! -x "$HOST_SOURCE" ]]; then
    echo "host binary not found at $HOST_SOURCE — run without --skip-build" >&2
    exit 1
fi

# ---------- install host ---------------------------------------------------

mkdir -p "$BIN_DIR"
install -m 0755 "$HOST_SOURCE" "$HOST_BIN"
echo "→ installed host:   $HOST_BIN"

# ---------- install native messaging manifest (one per detected browser) --

INSTALLED_TARGETS=()
for t in "${TARGETS[@]}"; do
    IFS='|' read -r name reldir <<< "$t"
    case "$name" in
        Chromium|Chrome|Brave|Vivaldi|Edge|Opera) template=packaging/chromium-host.json.in; nm_subdir=NativeMessagingHosts ;;
        *) template=packaging/firefox-host.json.in; nm_subdir=native-messaging-hosts ;;
    esac
    # Zen's profile lives in .config/zen, but its native-host lookup uses
    # Mozilla's user manifest directory (also shared with Firefox).
    [[ "$name" == "Zen" ]] && reldir=.mozilla
    nm_dir="${MANIFEST_ROOT}/${reldir}/${nm_subdir}"
    nm_path="${nm_dir}/${HOST_NAME}.json"
    mkdir -p "$nm_dir"
    temporary=$(mktemp "$nm_dir/.mpris-manifest-XXXXXX")
    if ! python3 - "$template" "$HOST_BIN" > "$temporary" <<'MANIFEST'
import json, os, sys
with open(sys.argv[1]) as stream:
    manifest = json.load(stream)
manifest["path"] = os.path.abspath(sys.argv[2])
print(json.dumps(manifest, indent=2))
MANIFEST
    then
        rm -f "$temporary"
        exit 1
    fi
    chmod 0644 "$temporary"
    mv -f "$temporary" "$nm_path"
    echo "→ installed manifest [$name]: $nm_path"
    INSTALLED_TARGETS+=("$name")
done

# ---------- next steps -----------------------------------------------------

JOINED_TARGETS=$(IFS=", "; echo "${INSTALLED_TARGETS[*]}")

cat <<EOF

Done. For Chromium-family browsers, unpack the output from
  python3 scripts/build-extension.py --browser chromium --output build/chromium.zip
and load the folder through chrome://extensions → Developer mode → Load unpacked.

For Firefox-family browsers, load the WebExtension into each browser you want it in
(${JOINED_TARGETS}):

  1. Open about:debugging#/runtime/this-firefox  (or the equivalent in your
     fork — Zen, LibreWolf, etc. all have the same about:debugging page).
  2. Click "Load Temporary Add-on…"
  3. Select:
        ${PROJECT_ROOT}/extension/manifest.json

The extension will stay loaded until you restart the browser. To make it
permanent on stable builds you'll need to either self-distribute via
https://addons.mozilla.org self-distribution, or use a developer/nightly
build with  xpinstall.signatures.required = false  in about:config and
drag-and-drop ${EXT_ZIP} onto the browser window.

Test:
  - Open a YouTube video.
  - Run:  playerctl -l
    You should see something like:
      firefox.instance<pid>_t<tab_id>
    (The bus-name prefix says "firefox" regardless of which fork is
    running — status bars use it to identify the source.)
  - Open your Hyprland media controls — the player should appear with
    title, channel, thumbnail, time, and a working seek bar.

Logs:
  Host stderr is forwarded to the browser's parent process. To see live
  logs, run the browser from a terminal:
      MOZ_LOG="nativeMessaging:5" zen-browser
EOF
