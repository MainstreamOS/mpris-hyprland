#!/usr/bin/env bash
# Produce a Mozilla-signed, self-distributed .xpi so the extension auto-installs
# on vanilla Firefox (which hard-requires signing) as well as every fork, via
# the browser policy the package ships. Run once per release by the maintainer;
# commit the result at dist/.
#
# Prereqs:
#   - node/npm (uses `web-ext` if installed, else `npx --yes web-ext`)
#   - AMO API credentials in the environment. Create a free account at
#     addons.mozilla.org, then generate a key/secret at
#     https://addons.mozilla.org/developers/addon/api/key/ and export:
#       AMO_JWT_ISSUER   the "JWT issuer" (API key), e.g. user:12345:67
#       AMO_JWT_SECRET   the "JWT secret"
#     These are yours — never commit them. Pass them in the environment only.
#
# Output: dist/mpris-hyprland.xpi
#   The PKGBUILD ships this signed .xpi when present (and falls back to an
#   unsigned zip otherwise). After signing, commit dist/mpris-hyprland.xpi
#   and bump extension/manifest.json's version before the next signing run
#   (AMO refuses to re-sign an already-submitted version).

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

: "${AMO_JWT_ISSUER:?Set AMO_JWT_ISSUER (AMO API key / JWT issuer)}"
: "${AMO_JWT_SECRET:?Set AMO_JWT_SECRET (AMO API secret)}"

if command -v web-ext >/dev/null 2>&1; then
    WEBEXT=(web-ext)
elif command -v npx >/dev/null 2>&1; then
    WEBEXT=(npx --yes web-ext)
else
    echo "error: need web-ext or npx — install node/npm (or: npm i -g web-ext)" >&2
    exit 1
fi

ART="$ROOT/web-ext-artifacts"
rm -rf "$ART"
mkdir -p "$ART" "$ROOT/dist"

ver="$(grep -oE '"version"[[:space:]]*:[[:space:]]*"[^"]+"' "$ROOT/extension/manifest.json" | head -1 | grep -oE '[0-9][^"]*')"
echo "Signing extension v${ver:-?} via AMO (unlisted / self-distributed)…"

"${WEBEXT[@]}" sign \
    --source-dir "$ROOT/extension" \
    --artifacts-dir "$ART" \
    --channel unlisted \
    --api-key "$AMO_JWT_ISSUER" \
    --api-secret "$AMO_JWT_SECRET"

signed="$(ls -t "$ART"/*.xpi 2>/dev/null | head -1 || true)"
[[ -n "$signed" ]] || { echo "error: AMO returned no signed .xpi" >&2; exit 1; }

cp -f "$signed" "$ROOT/dist/mpris-hyprland.xpi"
echo "Signed → dist/mpris-hyprland.xpi"
echo "Next: git add dist/mpris-hyprland.xpi && commit + push, then rebuild the package."
