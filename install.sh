#!/usr/bin/env bash
# Build and install firefox-mpris-hyprland for the current user.
#
# Layout (per-user install — no sudo needed):
#   ~/.local/bin/firefox-mpris-host
#   ~/.mozilla/native-messaging-hosts/io.github.mainstreamos.firefox_mpris_hyprland.json
#
# After running this script you still need to load the WebExtension into
# Firefox manually — see the section printed at the end. Firefox does not
# permit the host to install an unsigned extension automatically.

set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")"
PROJECT_ROOT="$(pwd)"

# ---------- root guard -----------------------------------------------------
# Firefox runs as your normal user, not root. If we install the native
# messaging manifest under /root/.mozilla, Firefox will never see it. Bail
# out with a clear message instead of silently producing a broken install.
if [[ ${EUID:-$(id -u)} -eq 0 ]]; then
    cat <<'EOF' >&2
ERROR: install.sh must NOT be run as root (or under sudo).

Firefox runs as your normal user account, so the native-messaging manifest
needs to live at:
    ~/.mozilla/native-messaging-hosts/io.github.mainstreamos.firefox_mpris_hyprland.json

…where ~ is YOUR home, not /root.

Run again without sudo:
    ./install.sh

If you want a system-wide install for all users, use the Arch package
(makepkg -si) instead — that installs to /usr/bin and
/usr/lib/mozilla/native-messaging-hosts/.
EOF
    exit 1
fi

# ---------- options --------------------------------------------------------

INSTALL_PREFIX="${HOME}/.local"
HOST_NAME="io.github.mainstreamos.firefox_mpris_hyprland"
NM_DIR="${HOME}/.mozilla/native-messaging-hosts"
SKIP_BUILD=0
ZIP_ONLY=0
UNINSTALL=0

usage() {
    cat <<EOF
Usage: $0 [options]

Options:
  --prefix DIR     Install binary under DIR/bin (default: \$HOME/.local)
  --skip-build     Don't run cargo, assume host is already built
  --zip-only       Only build the extension .zip, skip host & native manifest
  --uninstall      Remove installed files (host + manifest only)
  -h, --help       Show this help

Files installed:
  \$prefix/bin/firefox-mpris-host
  ~/.mozilla/native-messaging-hosts/${HOST_NAME}.json
  ./build/firefox-mpris-hyprland.zip   (extension package, load manually)
EOF
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --prefix) INSTALL_PREFIX="$2"; shift 2 ;;
        --skip-build) SKIP_BUILD=1; shift ;;
        --zip-only) ZIP_ONLY=1; shift ;;
        --uninstall) UNINSTALL=1; shift ;;
        -h|--help) usage; exit 0 ;;
        *) echo "unknown option: $1" >&2; usage >&2; exit 2 ;;
    esac
done

BIN_DIR="${INSTALL_PREFIX}/bin"
HOST_BIN="${BIN_DIR}/firefox-mpris-host"
NM_MANIFEST="${NM_DIR}/${HOST_NAME}.json"

# ---------- uninstall ------------------------------------------------------

if [[ $UNINSTALL -eq 1 ]]; then
    rm -fv "$HOST_BIN" "$NM_MANIFEST"
    echo
    echo "Uninstalled. The Firefox extension itself is still installed —"
    echo "remove it from about:addons if you no longer want it."
    exit 0
fi

# ---------- build extension zip -------------------------------------------

mkdir -p build
EXT_ZIP="${PROJECT_ROOT}/build/firefox-mpris-hyprland.zip"
rm -f "$EXT_ZIP"
( cd extension && zip -qr "$EXT_ZIP" . -x '*.DS_Store' )
echo "→ extension package: $EXT_ZIP"

if [[ $ZIP_ONLY -eq 1 ]]; then
    exit 0
fi

# ---------- build host -----------------------------------------------------

if [[ $SKIP_BUILD -eq 0 ]]; then
    echo "→ building host (cargo build --release)…"
    ( cd host && cargo build --release )
fi

HOST_SOURCE="host/target/release/firefox-mpris-host"
if [[ ! -x "$HOST_SOURCE" ]]; then
    echo "host binary not found at $HOST_SOURCE — run without --skip-build" >&2
    exit 1
fi

# ---------- install host ---------------------------------------------------

mkdir -p "$BIN_DIR" "$NM_DIR"
install -m 0755 "$HOST_SOURCE" "$HOST_BIN"
echo "→ installed host:   $HOST_BIN"

# ---------- install native messaging manifest ------------------------------

sed "s|@HOST_BINARY@|${HOST_BIN}|g" \
    packaging/firefox-mpris-host.json.in > "$NM_MANIFEST"
chmod 0644 "$NM_MANIFEST"
echo "→ installed manifest: $NM_MANIFEST"

# ---------- next steps -----------------------------------------------------

cat <<EOF

Done. Final step — load the WebExtension into Firefox:

  1. Open about:debugging#/runtime/this-firefox
  2. Click "Load Temporary Add-on…"
  3. Select:
        ${PROJECT_ROOT}/extension/manifest.json

The extension will stay loaded until you restart Firefox. To make it
permanent, either:
  - Use Firefox Developer Edition / Nightly with about:config setting
    xpinstall.signatures.required = false, then drag-and-drop
    ${EXT_ZIP} onto a Firefox window, or
  - Sign the extension via https://addons.mozilla.org self-distribution.

Test:
  - Open a YouTube video.
  - Run:  playerctl -l
    You should see something like:
      firefox.instance<pid>_t<tab_id>
  - Open your Hyprland media controls — the player should appear with
    title, channel, thumbnail, time, and a working seek bar.

Logs:
  Host stderr is sent to wherever Firefox writes its native messaging logs.
  To see live logs, run Firefox from a terminal with:
      MOZ_LOG="nativeMessaging:5" firefox
EOF
