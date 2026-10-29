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

# Derive the expected ID from the tracked manifest, not a maintainer-local file.
want="$(python3 - "$EXT/manifest.chromium.json" <<'PY'
import base64, hashlib, json, sys
with open(sys.argv[1]) as manifest:
    public_key = base64.b64decode(json.load(manifest)['key'], validate=True)
print(hashlib.sha256(public_key).hexdigest()[:32].translate(str.maketrans('0123456789abcdef', 'abcdefghijklmnop')))
PY
)"
got="$(openssl pkey -in "$KEY" -pubout -outform DER 2>/dev/null \
    | openssl dgst -sha256 -binary | head -c 16 | od -An -v -tx1 | tr -d ' \n' | tr '0-9a-f' 'a-p')"
[[ "$got" == "$want" ]] || { echo "error: extension ID $got != expected $want" >&2; exit 1; }

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
crx="$ROOT/build/chromium-unpacked.crx"
rm -f "$crx"
chromium \
    --pack-extension="$STAGE" \
    --pack-extension-key="$KEY" \
    --no-message-box \
    --user-data-dir="$profile" >/dev/null 2>&1

[[ -f "$crx" ]] || { echo "error: chromium produced no .crx" >&2; exit 1; }

cp -f "$crx" "$ROOT/dist/mpris-hyprland.crx"
echo "Packed → dist/mpris-hyprland.crx (id $got)"
echo "Next: git add dist/mpris-hyprland.crx && commit + push, then rebuild the package."
