# Maintainer: blackdroid <blackdriod@gmail.com>
pkgname=firefox-mpris-hyprland
pkgver=0.1.0
pkgrel=1
pkgdesc="Per-tab MPRIS bridge for Firefox — exposes Media Session metadata (title, artist, position, YouTube thumbnails) on D-Bus for Hyprland/Wayland status bars"
arch=('x86_64' 'aarch64')
url="https://github.com/MainstreamOS/firefox-mpris-hyprland"
license=('MIT')
depends=('firefox' 'dbus')
makedepends=('rust' 'cargo' 'zip')
optdepends=(
    'playerctl: control playback from CLI'
    'quickshell: Hyprland status bar with MPRIS support'
    'waybar: status bar with MPRIS module'
)
source=("$pkgname-$pkgver.tar.gz::file://$PWD")
sha256sums=('SKIP')

build() {
    cd "$srcdir"
    cd host
    cargo build --release --locked
}

package() {
    cd "$srcdir"

    # Native host binary
    install -Dm0755 host/target/release/firefox-mpris-host \
        "$pkgdir/usr/bin/firefox-mpris-host"

    # Native messaging manifest (system-wide). Firefox reads from
    # /usr/lib/mozilla/native-messaging-hosts/ in addition to the user's
    # ~/.mozilla/native-messaging-hosts.
    sed 's|@HOST_BINARY@|/usr/bin/firefox-mpris-host|g' \
        packaging/firefox-mpris-host.json.in \
        | install -Dm0644 /dev/stdin \
            "$pkgdir/usr/lib/mozilla/native-messaging-hosts/io.github.mainstreamos.firefox_mpris_hyprland.json"

    # Bundle the WebExtension as an unsigned .zip — users still need to
    # load it manually (see /usr/share/doc/$pkgname/README.md).
    ( cd extension && zip -qr "$srcdir/firefox-mpris-hyprland.zip" . )
    install -Dm0644 "$srcdir/firefox-mpris-hyprland.zip" \
        "$pkgdir/usr/share/$pkgname/firefox-mpris-hyprland.zip"

    install -Dm0644 README.md "$pkgdir/usr/share/doc/$pkgname/README.md"
    install -Dm0644 LICENSE "$pkgdir/usr/share/licenses/$pkgname/LICENSE"
}
