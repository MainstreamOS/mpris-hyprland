#!/usr/bin/env bash
# Pack the maintainer-signed Chromium .crx the package ships. Run once per
# release by the maintainer; commit the result at dist/.
#
# Unlike Firefox (AMO), Chromium installs a self-distributed .crx via the
# browser policy the package ships (external_crx). The extension ID is fixed by
# the public key baked into extension/manifest.chromium.json ("key"), whose
# private half is chromium-signing-key.pem — packing with that key keeps the ID
# constant (ckiplbjpfoaeijjpijkpmhcmdfolhonm), so an update never orphans the
# installed extension.
#
# Prereqs:
#   - chromium on PATH
#   - chromium-signing-key.pem present at the repo root (maintainer secret;
#     gitignored, NEVER committed — losing it changes the extension ID)
#
# Output: dist/mpris-hyprland.crx

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

KEY="$ROOT/chromium-signing-key.pem"
EXT="$ROOT/extension"
STAGE="$ROOT/build/chromium-unpacked"

command -v chromium >/dev/null 2>&1 || { echo "error: chromium not on PATH" >&2; exit 1; }
[[ -f "$KEY" ]] || { echo "error: missing $KEY (maintainer signing key)" >&2; exit 1; }

# Stage exactly the files the Chromium build ships: the Chromium manifest
# becomes manifest.json, plus the shared scripts and icons.
rm -rf "$STAGE"
mkdir -p "$STAGE/icons" "$ROOT/dist"
cp -f "$EXT/manifest.chromium.json" "$STAGE/manifest.json"
cp -f "$EXT/background.js" "$EXT/content.js" "$EXT/content-main.js" "$STAGE/"
cp -f "$EXT/icons/icon-48.png" "$EXT/icons/icon-96.png" "$STAGE/icons/"

# Pack against the fixed key, in a throwaway profile so a running browser's
# singleton lock doesn't block us.
profile="$(mktemp -d)"
trap 'rm -rf "$profile"' EXIT
chromium \
    --pack-extension="$STAGE" \
    --pack-extension-key="$KEY" \
    --no-message-box \
    --user-data-dir="$profile" >/dev/null 2>&1 || true

crx="$ROOT/build/chromium-unpacked.crx"
[[ -f "$crx" ]] || { echo "error: chromium produced no .crx" >&2; exit 1; }

# Guard against a key/manifest mismatch that would silently change the ID and
# orphan installed copies: the ID is sha256(DER pubkey)[:16] mapped 0-f → a-p.
want="$(cat "$ROOT/.crx-extid.txt")"
got="$(openssl rsa -in "$KEY" -pubout -outform DER 2>/dev/null \
    | openssl dgst -sha256 -binary | head -c 16 | od -An -v -tx1 | tr -d ' \n' | tr '0-9a-f' 'a-p')"
[[ "$got" == "$want" ]] || { echo "error: extension ID $got != expected $want" >&2; exit 1; }

cp -f "$crx" "$ROOT/dist/mpris-hyprland.crx"
echo "Packed → dist/mpris-hyprland.crx (id $got)"
echo "Next: git add dist/mpris-hyprland.crx && commit + push, then rebuild the package."
