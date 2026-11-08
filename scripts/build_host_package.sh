#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

echo "=== Gamestream Host - Packaging Pipeline ==="
cd "$ROOT_DIR"

VERSION=$(grep '^version =' Cargo.toml | head -n1 | cut -d'"' -f2)
ARCH="x86_64"
DIST_DIR="$ROOT_DIR/dist"
mkdir -p "$DIST_DIR"

echo "1. Compiling optimized release binary..."
cargo build --release -p gamestream-host

BIN_PATH="$ROOT_DIR/target/release/gamestream-host"
if [ ! -f "$BIN_PATH" ]; then
    echo "Error: Binary not found at $BIN_PATH"
    exit 1
fi

echo "2. Creating portable tarball..."
TAR_DIR="$DIST_DIR/gamestream-host-${VERSION}-linux-${ARCH}"
rm -rf "$TAR_DIR"
mkdir -p "$TAR_DIR"

cp "$BIN_PATH" "$TAR_DIR/gamestream-host"
cp "$ROOT_DIR/README.md" "$TAR_DIR/README.md"
cp "$ROOT_DIR/DECISIONES.md" "$TAR_DIR/DECISIONES.md"

# Add launcher script that checks uinput permissions
cat << 'EOF' > "$TAR_DIR/run-host.sh"
#!/usr/bin/env bash
set -e

# Verify uinput permission for remote input
if [ ! -w /dev/uinput ]; then
    echo "Warning: /dev/uinput is not writable by current user."
    echo "Run: sudo usermod -aG input $USER and log in again."
fi

exec "$(dirname "$0")/gamestream-host" "$@"
EOF
chmod +x "$TAR_DIR/run-host.sh"

TAR_FILE="$DIST_DIR/gamestream-host-${VERSION}-linux-${ARCH}.tar.gz"
tar -czf "$TAR_FILE" -C "$DIST_DIR" "gamestream-host-${VERSION}-linux-${ARCH}"
echo "Portable package ready: $TAR_FILE"

echo "3. Creating Debian (.deb) package layout..."
DEB_ROOT="$DIST_DIR/deb-pkg"
rm -rf "$DEB_ROOT"
mkdir -p "$DEB_ROOT/DEBIAN"
mkdir -p "$DEB_ROOT/usr/bin"
mkdir -p "$DEB_ROOT/usr/share/applications"

cp "$BIN_PATH" "$DEB_ROOT/usr/bin/gamestream-host"
chmod 755 "$DEB_ROOT/usr/bin/gamestream-host"

cat << EOF > "$DEB_ROOT/DEBIAN/control"
Package: gamestream-host
Version: ${VERSION}
Section: video
Priority: optional
Architecture: amd64
Maintainer: Adrian <adrian@gamestream.local>
Depends: libc6, libavcodec60 | libavcodec59 | libavcodec58, libavutil58 | libavutil57 | libavutil56, libpulse0, libopus0
Description: Ultra-low latency game and desktop streaming host for Linux
 Open-source alternative to Parsec in Rust. Streams 60 FPS video with
 hardware VA-API/NVENC encoding and ChaCha20-Poly1305 encryption.
EOF

cat << EOF > "$DEB_ROOT/usr/share/applications/gamestream-host.desktop"
[Desktop Entry]
Name=Gamestream Host
Comment=Ultra-low latency game streaming host in Rust
Exec=gamestream-host --udp --stun
Terminal=true
Type=Application
Categories=Network;Game;AudioVideo;
EOF

dpkg-deb --build "$DEB_ROOT" "$DIST_DIR/gamestream-host_${VERSION}_amd64.deb" || true
echo "Debian package created: $DIST_DIR/gamestream-host_${VERSION}_amd64.deb"

echo "Packaging complete!"
