# Gamestream-RS 🎮🦀

Alternativa de código abierto a Parsec con host nativo en Linux (escrito en Rust) y clientes para Windows.

Diseñado específicamente para streaming de juegos con latencia ultra baja (< 30 ms en LAN), priorizando **60 FPS estables** incluso en hardware modesto (desde Intel HD 3000 / Sandy Bridge en adelante).

---

## Estructura del Workspace de Cargo

```text
gamestream-rs/
├── crates/
│   ├── common/      # Tipos comunes, estructuras de video/audio, errores y metadatos
│   ├── capture/     # Captura de pantalla: X11 SHM (x11rb) y Wayland (PipeWire)
│   ├── encode/      # Codificación H.264 por VA-API / NVENC con fallback x264
│   ├── audio/       # Captura de audio (cpal) y codificación Opus low-delay
│   ├── net/         # Protocolo WebRTC (webrtc-rs), ICE/STUN, DTLS, DataChannels
│   ├── input/       # Inyección de controles en Linux (/dev/uinput vía evdev)
│   ├── host/        # Binario host para Linux (`gamestream-host`)
│   └── client/      # Binario cliente nativo para Windows (`gamestream-client`)
└── DECISIONES.md    # Documento de arquitectura y decisiones técnicas
```

---

## Requisitos de compilación en Linux Mint / Ubuntu

```bash
sudo apt update
sudo apt install build-essential pkg-config libx11-dev libva-dev libavcodec-dev libavutil-dev
```

---

## Probar la Fase 2 (Captura X11 y Benchmark de FPS)

### Compilar en modo release optimizado
```bash
cargo build --release --bin gamestream-host
```

### Ejecutar benchmark a 60 FPS fijos (mide latencia y uso de CPU/RAM)
```bash
./target/release/gamestream-host --bench --duration 5 --target-fps 60
```

### Ejecutar benchmark sin límite (throughput máximo del motor de captura)
```bash
./target/release/gamestream-host --bench --duration 3 --target-fps 0
```
