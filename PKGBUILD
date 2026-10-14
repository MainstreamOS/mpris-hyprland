# Maintainer: blackdroid <blackdriod@gmail.com>
pkgname=mpris-hyprland
pkgver=0.2.3
pkgrel=1
pkgdesc="Per-window MPRIS bridge for Firefox/Zen and Chromium — Media Session metadata, position, artwork, per-tab volume on D-Bus for Hyprland/Wayland status bars (lighter plasma-browser-integration alternative)"
arch=('x86_64' 'aarch64')
options=('!debug')
url="https://github.com/MainstreamOS/mpris-hyprland"
license=('MIT')
replaces=('firefox-mpris-hyprland')
conflicts=('firefox-mpris-hyprland')
provides=('firefox-mpris-hyprland')
depends=('dbus' 'python')
install=mpris-hyprland.install
optdepends=(
    'firefox: the browser this bridges (or any Firefox fork)'
    'zen-browser-bin: Firefox fork, default on Mainstream OS'
    'chromium: Chromium-family browser this also bridges (default on Mainstream OS)'
    'playerctl: control playback from CLI'
    'quickshell: Hyprland status bar with MPRIS support'
    'waybar: status bar with MPRIS module'
)
makedepends=('rust' 'cargo' 'zip' 'git')
source=("$pkgname::git+${MPRIS_HYPRLAND_REPO_URL:-$url.git}#${MPRIS_HYPRLAND_REF:-tag=v$pkgver}")
sha256sums=('SKIP')

build() {
    cd "$srcdir/$pkgname/host"
    cargo build --release --locked
}

package() {
    cd "$srcdir/$pkgname"

    # ── native host binary ──────────────────────────────────────────────────
    install -Dm0755 host/target/release/mpris-hyprland-host \
        "$pkgdir/usr/bin/mpris-hyprland-host"

    # ── native messaging manifest (system-wide) ─────────────────────────────
    # Firefox and forks (including Zen) read /usr/lib/mozilla/native-messaging-hosts/
    # in addition to the per-user dir, so one system manifest covers them all.
    sed 's|@HOST_BINARY@|/usr/bin/mpris-hyprland-host|g' \
        packaging/firefox-host.json.in \
        | install -Dm0644 /dev/stdin \
            "$pkgdir/usr/lib/mozilla/native-messaging-hosts/io.github.mainstreamos.firefox_mpris_hyprland.json"

    local xpi_ver="" crx_ver="" crx_id="ckiplbjpfoaeijjpijkpmhcmdfolhonm"
    if [[ -f dist/mpris-hyprland.xpi ]]; then
        xpi_ver=$(python3 -c 'import json,zipfile; print(json.loads(zipfile.ZipFile("dist/mpris-hyprland.xpi").read("manifest.json"))["version"])')
    fi
    if [[ "$xpi_ver" == "$pkgver" ]]; then
        install -Dm0644 dist/mpris-hyprland.xpi "$pkgdir/usr/share/$pkgname/mpris-hyprland.xpi"
    elif [[ "${MPRIS_HYPRLAND_TEST_BUILD:-0}" == 1 ]]; then
        python3 scripts/build-extension.py --output "$pkgdir/usr/share/$pkgname/mpris-hyprland.xpi"
    else
        echo "Signed Firefox bundle does not match $pkgver; rebuild/sign the release, or set MPRIS_HYPRLAND_TEST_BUILD=1 for a local unsigned test." >&2
        return 1
    fi

    install -Dm0755 packaging/manage-policies.py "$pkgdir/usr/lib/$pkgname/manage-policies.py"
    install -Dm0644 -t "$pkgdir/usr/lib/$pkgname/policies" packaging/policies/firefox.json packaging/policies/zen.json

    for directory in /etc/chromium/native-messaging-hosts /etc/opt/chrome/native-messaging-hosts /etc/opt/edge/native-messaging-hosts; do
        sed 's|@HOST_BINARY@|/usr/bin/mpris-hyprland-host|g' packaging/chromium-host.json.in \
            | install -Dm0644 /dev/stdin "$pkgdir$directory/io.github.mainstreamos.firefox_mpris_hyprland.json"
    done
    if [[ -f dist/mpris-hyprland.crx ]]; then
        crx_ver=$(python3 -c 'import json,zipfile; print(json.loads(zipfile.ZipFile("dist/mpris-hyprland.crx").read("manifest.json"))["version"])')
    fi
    if [[ "$crx_ver" == "$pkgver" ]]; then
        install -Dm0644 dist/mpris-hyprland.crx "$pkgdir/usr/share/chromium/extensions/${crx_id}.crx"
        printf '{"external_crx":"/usr/share/chromium/extensions/%s.crx","external_version":"%s"}\n' "$crx_id" "$crx_ver" \
            | install -Dm0644 /dev/stdin "$pkgdir/usr/share/chromium/extensions/${crx_id}.json"
    elif [[ "${MPRIS_HYPRLAND_TEST_BUILD:-0}" != 1 ]]; then
        echo "Signed Chromium bundle does not match $pkgver; rebuild the release CRX." >&2
        return 1
    fi

    install -Dm0644 README.md "$pkgdir/usr/share/doc/$pkgname/README.md"
    install -Dm0644 LICENSE "$pkgdir/usr/share/licenses/$pkgname/LICENSE"
}
