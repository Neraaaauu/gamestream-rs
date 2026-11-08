#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

echo "=== Gamestream Client - Windows Packaging Pipeline ==="
cd "$ROOT_DIR"

VERSION=$(grep '^version =' Cargo.toml | head -n1 | cut -d'"' -f2)
DIST_DIR="$ROOT_DIR/dist"
mkdir -p "$DIST_DIR"

echo "1. Checking cross-compilation toolchain for Windows..."
if command -v x86_64-w64-mingw32-dlltool >/dev/null 2>&1; then
    echo "Cross-compiling release client for Windows (target: x86_64-pc-windows-gnu)..."
    cargo build --release --target x86_64-pc-windows-gnu -p gamestream-client
    WIN_BIN="$ROOT_DIR/target/x86_64-pc-windows-gnu/release/gamestream-client.exe"
else
    echo "[INFO] mingw-w64 no está instalado localmente en esta máquina Linux."
    echo "       El CI automatizado de GitHub Actions (.github/workflows/ci.yml)"
    echo "       compila de manera nativa 'gamestream-client.exe' con el compilador oficial de MSVC"
    echo "       y genera automáticamente el archivo 'gamestream-client-windows-x86_64.zip'."
    echo ""
    echo "Para compilar localmente en Linux Mint sin Windows:"
    echo "       sudo apt install -y mingw-w64"
    WIN_BIN=""
fi

echo "2. Assembling portable Windows client package..."
CLIENT_DIR="$DIST_DIR/gamestream-client-${VERSION}-windows-x86_64"
rm -rf "$CLIENT_DIR"
mkdir -p "$CLIENT_DIR"

if [ -n "$WIN_BIN" ] && [ -f "$WIN_BIN" ]; then
    cp "$WIN_BIN" "$CLIENT_DIR/gamestream-client.exe"
fi

cat << 'EOF' > "$CLIENT_DIR/INSTRUCCIONES.txt"
========================================================================
GAMESTREAM CLIENT (WINDOWS) - ALTERNATIVA A PARSEC
========================================================================

Este cliente es 100% portable y no requiere instalación.
Funciona en Direct3D 11 con soporte para GPUs modestas (Feature Level 10_1,
incluyendo Intel HD 3000 en adelante).

CÓMO CONECTARTE AL JUEGO DE TU AMIGO:

1. Si están en la misma red local (LAN / Wi-Fi):
   gamestream-client.exe --udp --host IP_DE_TU_AMIGO:8554 --key CLAVE_HEX

2. Si se conectan por Internet (sin abrir puertos):
   gamestream-client.exe --udp --host IP_PUBLICA_STUN:PUERTO --key CLAVE_HEX --stun

CONTROLES:
- Ratón y teclado se transmiten en tiempo real de baja latencia al host.
- Mandos Xbox / XInput se detectan y transmiten automáticamente.
========================================================================
EOF

cat << 'EOF' > "$CLIENT_DIR/conectar_lan.bat"
@echo off
set /p HOST_IP="Introduce la IP de tu amigo (ej. 192.168.1.50:8554): "
set /p KEY="Introduce la clave de cifrado: "
gamestream-client.exe --udp --host %HOST_IP% --key %KEY%
pause
EOF

ZIP_FILE="$DIST_DIR/gamestream-client-${VERSION}-windows-x86_64.zip"
rm -f "$ZIP_FILE"
cd "$DIST_DIR"
zip -r "$ZIP_FILE" "gamestream-client-${VERSION}-windows-x86_64"

echo "Windows client zip package ready: $ZIP_FILE"
