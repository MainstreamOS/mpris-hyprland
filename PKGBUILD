# Maintainer: blackdroid <blackdriod@gmail.com>
pkgname=firefox-mpris-hyprland
pkgver=0.2.0
pkgrel=2
pkgdesc="Per-tab MPRIS bridge for Firefox/Zen — Media Session metadata, position, artwork on D-Bus for Hyprland/Wayland status bars (lighter plasma-browser-integration alternative)"
arch=('x86_64' 'aarch64')
url="https://github.com/MainstreamOS/firefox-mpris-hyprland"
license=('MIT')
depends=('dbus')
optdepends=(
    'firefox: the browser this bridges (or any Firefox fork)'
    'zen-browser-bin: Firefox fork, default on Mainstream OS'
    'playerctl: control playback from CLI'
    'quickshell: Hyprland status bar with MPRIS support'
    'waybar: status bar with MPRIS module'
)
makedepends=('rust' 'cargo' 'zip' 'git')
# Build from the published repo so the ISO build is reproducible. For local
# iteration use ./install.sh instead of makepkg.
source=("$pkgname::git+$url.git#branch=master")
sha256sums=('SKIP')

build() {
    cd "$srcdir/$pkgname/host"
    cargo build --release --locked
}

package() {
    cd "$srcdir/$pkgname"

    # ── native host binary ──────────────────────────────────────────────────
    install -Dm0755 host/target/release/firefox-mpris-host \
        "$pkgdir/usr/bin/firefox-mpris-host"

    # ── native messaging manifest (system-wide) ─────────────────────────────
    # Firefox and forks (including Zen) read /usr/lib/mozilla/native-messaging-hosts/
    # in addition to the per-user dir, so one system manifest covers them all.
    sed 's|@HOST_BINARY@|/usr/bin/firefox-mpris-host|g' \
        packaging/firefox-mpris-host.json.in \
        | install -Dm0644 /dev/stdin \
            "$pkgdir/usr/lib/mozilla/native-messaging-hosts/io.github.mainstreamos.firefox_mpris_hyprland.json"

    # ── WebExtension .xpi ───────────────────────────────────────────────────
    # Prefer a maintainer-signed build (dist/, produced by scripts/sign.sh via
    # AMO) so the policy auto-installs it on vanilla Firefox too. Without it,
    # ship an unsigned zip — which auto-installs only on Zen and other unbranded
    # forks (Firefox rejects unsigned, even via policy).
    if [[ -f dist/firefox-mpris-hyprland.xpi ]]; then
        install -Dm0644 dist/firefox-mpris-hyprland.xpi \
            "$pkgdir/usr/share/$pkgname/firefox-mpris-hyprland.xpi"
    else
        ( cd extension && zip -qr "$srcdir/firefox-mpris-hyprland.xpi" . -x '*.DS_Store' )
        install -Dm0644 "$srcdir/firefox-mpris-hyprland.xpi" \
            "$pkgdir/usr/share/$pkgname/firefox-mpris-hyprland.xpi"
    fi

    # ── browser policies to auto-install the extension (unsigned) ───────────
    # Enterprise policy force-installs the .xpi from the local path above. Zen
    # ships xpinstall.signatures.required=false, so the unsigned .xpi installs
    # fine; the Zen policy replicates Zen's own baseline (DisableAppUpdate,
    # DefaultSerialGuardSetting) because the /etc policy overrides the browser's
    # distribution policy on Linux. Vanilla Mozilla Firefox enforces signing and
    # will reject the unsigned .xpi — there it needs an AMO-signed build (the
    # policy is shipped anyway so it works the moment a signed .xpi exists, or on
    # an unbranded/ESR Firefox with signatures disabled).
    install -Dm0644 packaging/policies/zen.json \
        "$pkgdir/etc/zen/policies/policies.json"
    install -Dm0644 packaging/policies/firefox.json \
        "$pkgdir/etc/firefox/policies/policies.json"

    install -Dm0644 README.md "$pkgdir/usr/share/doc/$pkgname/README.md"
    install -Dm0644 LICENSE "$pkgdir/usr/share/licenses/$pkgname/LICENSE"
}
