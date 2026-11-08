#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

echo "==============================================================================="
echo "GAMESTREAM-RS - SUITE DE BENCHMARKS Y DIAGNÓSTICO DEL SISTEMA"
echo "==============================================================================="
echo "Fecha: $(date)"
echo "Host OS: $(uname -srm)"
echo "CPU: $(lscpu | grep 'Model name' | head -n1 | sed 's/Model name:[ \t]*//')"
echo "Memoria Total: $(free -h | awk '/^Mem:/ {print $2}')"

# Diagnóstico de GPU y Aceleración por Hardware
echo "-------------------------------------------------------------------------------"
echo "DIAGNÓSTICO DE HARDWARE Y DRIVERS:"
if [ -e /dev/dri/renderD128 ]; then
    echo "  [OK] Dispositivo VA-API detectado en /dev/dri/renderD128"
    if command -v vainfo >/dev/null 2>&1; then
        VA_DRIVER=$(vainfo 2>&1 | grep "Driver version" || true)
        echo "       Driver: $VA_DRIVER"
    fi
else
    echo "  [INFO] No se detectó /dev/dri/renderD128 (Fallback de Software libx264 activo)"
fi

if [ -e /dev/nvidiactl ]; then
    echo "  [OK] Dispositivo NVIDIA detectado (NVENC disponible)"
fi

# Diagnóstico de Captura y Pantalla
if [ -n "${WAYLAND_DISPLAY:-}" ] || [ "${XDG_SESSION_TYPE:-}" = "wayland" ]; then
    echo "  [OK] Sesión gráfica: WAYLAND (Captura PipeWire / Portal Screencast)"
else
    echo "  [OK] Sesión gráfica: X11 (DISPLAY=${DISPLAY:-:0}, Captura MIT-SHM Zero-Copy)"
fi

# Diagnóstico de /dev/uinput para entrada remota
if [ -w /dev/uinput ]; then
    echo "  [OK] Permisos de /dev/uinput: Correctos (Inyección virtual activa)"
else
    echo "  [WARN] /dev/uinput no tiene permiso de escritura para usuario actual"
fi
echo "==============================================================================="
echo ""

cd "$ROOT_DIR"
echo "Compilando binario host con optimizaciones..."
cargo build --release -p gamestream-host
HOST_BIN="$ROOT_DIR/target/release/gamestream-host"

DURATION=4

run_profile() {
    local name="$1"
    local w="$2"
    local h="$3"
    local bitrate="$4"

    echo ""
    echo ">>> Probando Perfil: $name (${w}x${h} @ 60 FPS, Bitrate: ${bitrate} kbps) <<<"
    echo "-------------------------------------------------------------------------------"
    "$HOST_BIN" --bench --encode --width "$w" --height "$h" --bitrate "$bitrate" --duration "$DURATION" --target-fps 60
}

# Ejecución de los 4 perfiles estandarizados (con 60 FPS fijos garantizados)
run_profile "720p60 (HD Estándar)" 1280 720 4500
run_profile "576p60 (Sub-HD 1024x576)" 1024 576 3000
run_profile "480p60 (Sub-HD 854x480)" 854 480 2000
run_profile "360p60 (Sub-HD 640x360)" 640 360 1200

echo ""
echo "==============================================================================="
echo "TODOS LOS BENCHMARKS FINALIZADOS CON ÉXITO"
echo "==============================================================================="
