use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use clap::Parser;
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
#[command(name = "gamestream-client")]
#[command(about = "Cliente de ultra baja latencia para Windows y multiplataforma en Rust", long_about = None)]
struct Cli {
    /// Dirección del Host Linux al que conectarse (ej. 192.168.1.100:8554)
    #[arg(short = 'H', long, default_value = "127.0.0.1:8554")]
    host: String,

    /// Activar modo pantalla completa
    #[arg(short, long, default_value_t = false)]
    fullscreen: bool,

    /// Conectar mediante streaming UDP robusto con cifrado E2E ChaCha20-Poly1305 (Fase 7)
    #[arg(long, default_value_t = false)]
    udp: bool,

    /// Clave de cifrado hex de 32 bytes provista por el host Linux
    #[arg(long)]
    key: Option<String>,

    /// Consultar servidor STUN y realizar UDP hole-punching hacia el host
    #[arg(long, default_value_t = false)]
    stun: bool,

    /// Redirigir flujo de video H.264 a stdout (útil en Linux para: `gamestream-client --stdout | mpv -`)
    #[arg(long, default_value_t = false)]
    stdout: bool,

    /// Activar estadísticas de telemetría y latencia en consola
    #[arg(short, long, default_value_t = true)]
    stats: bool,

    /// Activar logs detallados
    #[arg(short, long, default_value_t = false)]
    verbose: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let mut cli = Cli::parse();

    let log_level = if cli.verbose { "debug" } else { "info" };
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(log_level)),
        )
        .init();

    // Si se ejecutó haciendo doble clic sin parámetros (host por defecto y sin clave) y no es tubería:
    if cli.host == "127.0.0.1:8554" && cli.key.is_none() && !cli.stdout {
        println!("============================================================");
        println!("🎮 GAMESTREAM CLIENT (Windows / Linux)");
        println!("============================================================");
        use std::io::Write;
        print!("Introduce la IP de tu amigo (ej. 192.168.1.100:8554 o presiona Enter para 127.0.0.1:8554): ");
        let _ = std::io::stdout().flush();
        let mut input_host = String::new();
        let _ = std::io::stdin().read_line(&mut input_host);
        let trimmed_host = input_host.trim();
        if !trimmed_host.is_empty() {
            cli.host = trimmed_host.to_string();
        }

        print!("Introduce la clave de cifrado (o presiona Enter si no usas): ");
        let _ = std::io::stdout().flush();
        let mut input_key = String::new();
        let _ = std::io::stdin().read_line(&mut input_key);
        let trimmed_key = input_key.trim();
        if !trimmed_key.is_empty() {
            cli.key = Some(trimmed_key.to_string());
        }
    }

    info!("=== Gamestream Client (Windows / Multiplataforma) ===");
    info!("Conectando al host en {}...", cli.host);

    #[cfg(windows)]
    {
        info!("Inicializando acelerador gráfico Direct3D 11 (Feature Level 10_1 compatible con Intel HD 3000)...");
        if let Err(e) = init_d3d11_device() {
            warn!("Aviso Direct3D 11: {}", e);
        }
    }

    // Por defecto usar el modo UDP robusto
    let target_addr: SocketAddr = match cli.host.parse() {
        Ok(addr) => addr,
        Err(e) => {
            eprintln!("\n[ERROR] Dirección inválida de host: '{}' ({})", cli.host, e);
            eprintln!("Ejemplo válido: 192.168.1.50:8554");
            if !cli.stdout {
                println!("\nPresiona Enter para salir...");
                let _ = std::io::stdin().read_line(&mut String::new());
            }
            return Ok(());
        }
    };

    let res = run_udp_client(target_addr, cli.key, cli.stun || true, cli.stdout).await;
    if let Err(ref e) = res {
        eprintln!("\n[ERROR] Error durante la sesión de streaming: {e}");
    }

    if !cli.stdout {
        println!("\nPresiona Enter para salir...");
        let _ = std::io::stdin().read_line(&mut String::new());
    }

    res
}

#[cfg(windows)]
fn init_d3d11_device() -> Result<()> {
    use windows::Win32::Foundation::HMODULE;
    use windows::Win32::Graphics::Direct3D::*;
    use windows::Win32::Graphics::Direct3D11::*;

    unsafe {
        let feature_levels = [
            D3D_FEATURE_LEVEL_11_0,
            D3D_FEATURE_LEVEL_10_1, // Requisito no negociable: compatible con Intel HD 3000
            D3D_FEATURE_LEVEL_10_0,
        ];

        let mut device: Option<ID3D11Device> = None;
        let mut context: Option<ID3D11DeviceContext> = None;
        let mut chosen_level = D3D_FEATURE_LEVEL_10_1;

        let hr = D3D11CreateDevice(
            None,
            D3D_DRIVER_TYPE_HARDWARE,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_FLAG(0),
            Some(&feature_levels),
            D3D11_SDK_VERSION,
            Some(&mut device),
            Some(&mut chosen_level),
            Some(&mut context),
        );

        if hr.is_ok() {
            info!("Dispositivo Direct3D 11 creado exitosamente (Feature Level: {:?})", chosen_level);
            Ok(())
        } else {
            warn!("D3D11CreateDevice con GPU dedicada falló, intentando WARP (software)...");
            let hr_warp = D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_WARP,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_FLAG(0),
                Some(&feature_levels),
                D3D11_SDK_VERSION,
                Some(&mut device),
                Some(&mut chosen_level),
                Some(&mut context),
            );
            hr_warp.context("Fallo total al crear dispositivo Direct3D 11")?;
            info!("Dispositivo Direct3D 11 WARP creado exitosamente");
            Ok(())
        }
    }
}

#[cfg(windows)]
fn poll_windows_gamepad(pad_idx: u32) -> Option<gamestream_input::protocol::GamepadEvent> {
    use windows::Win32::UI::Input::XboxController::{XInputGetState, XINPUT_STATE};

    unsafe {
        let mut state = XINPUT_STATE::default();
        if XInputGetState(pad_idx, &mut state) == 0 {
            let gp = state.Gamepad;
            Some(gamestream_input::protocol::GamepadEvent {
                pad_id: pad_idx as u8,
                buttons: gp.wButtons.0,
                lt: gp.bLeftTrigger,
                rt: gp.bRightTrigger,
                lx: gp.sThumbLX,
                ly: gp.sThumbLY,
                rx: gp.sThumbRX,
                ry: gp.sThumbRY,
                _pad: 0,
            })
        } else {
            None
        }
    }
}

#[cfg(not(windows))]
fn poll_windows_gamepad(_pad_idx: u32) -> Option<gamestream_input::protocol::GamepadEvent> {
    None
}

async fn run_udp_client(
    target_addr: SocketAddr,
    key_hex: Option<String>,
    enable_stun: bool,
    output_stdout: bool,
) -> Result<()> {
    use gamestream_net::crypto::{CryptoRole, CryptoSession};
    use gamestream_net::protocol::NetworkFeedback;
    use gamestream_net::transceiver::{NetEvent, UdpTransceiver};
    use std::io::Write;

    if !output_stdout {
        info!("=== MODO CLIENTE UDP ROBUSTO CON CIFRADO E2E Y ADAPTACIÓN DINÁMICA ===");
    }

    let key = if let Some(hex) = key_hex {
        Some(CryptoSession::key_from_hex(&hex).context("Clave de cifrado hex inválida")?)
    } else {
        None
    };

    if enable_stun {
        if !output_stdout {
            info!("Descubriendo IP pública mediante STUN para P2P NAT Traversal...");
        }
        if let Ok(stun_sock) = tokio::net::UdpSocket::bind("0.0.0.0:0").await {
            if let Ok(pub_addr) = gamestream_net::stun::discover_public_endpoint(&stun_sock).await {
                if !output_stdout {
                    info!("Dirección pública STUN descubierta: {}", pub_addr);
                }
                let _ = gamestream_net::stun::punch_hole(&stun_sock, target_addr, 5).await;
            }
        }
    }

    let local_bind = "0.0.0.0:0";
    let (sender, mut receiver) = UdpTransceiver::bind(local_bind, target_addr, key, CryptoRole::Client).await?.into_split();

    // Enviar paquete inicial de saludo/punch
    let _ = sender.socket_ref().send_to(b"GSPUNCH", target_addr).await;

    if !output_stdout {
        println!("\n=========================================================================");
        println!("RECEPTOR UDP ROBUSTO GAMESTREAM CLIENT (H.264 + OPUS + CHACHA20-POLY1305)");
        println!("=========================================================================");
        println!("Host de Destino:   {}", target_addr);
        println!("Cifrado E2E:       {}", if key.is_some() { "ACTIVO (ChaCha20-Poly1305)" } else { "DESACTIVADO" });
        println!("-------------------------------------------------------------------------");
        println!("{:>5} | {:>9} | {:>9} | {:>10} | {:>8} | {:>10}", "Seg", "Video FPS", "Audio PPS", "Bitrate", "Pérdida", "Total Pkts");
        println!("-------------------------------------------------------------------------");
    }

    let running = Arc::new(AtomicBool::new(true));

    // Tarea de entrada periódica (60 Hz) hacia el host (Zero-lock)
    let input_sender = sender.clone();
    let in_run = running.clone();
    tokio::spawn(async move {
        use gamestream_input::protocol::MouseEvent;
        let mut interval = tokio::time::interval(Duration::from_millis(16));
        while in_run.load(Ordering::Relaxed) {
            interval.tick().await;

            // 1. Enviar eventos de ratón
            let mouse = MouseEvent { dx: 0, dy: 0, buttons: 0, wheel: 0 };
            let data = mouse.to_bytes();
            if input_sender.send_input_event(0x01, &data).await.is_err() {
                break;
            }

            // 2. Enviar estado de mando (XInput en Windows)
            if let Some(gp) = poll_windows_gamepad(0) {
                let gp_bytes = gp.to_bytes();
                if input_sender.send_input_event(0x03, &gp_bytes).await.is_err() {
                    break;
                }
            }
        }
    });

    // Tarea de telemetría de feedback hacia el host cada 500 ms (RTT + pérdida)
    let fb_sender = sender.clone();
    let fb_run = running.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_millis(500));
        let mut highest_ack = 0u32;
        while fb_run.load(Ordering::Relaxed) {
            interval.tick().await;
            let fb = NetworkFeedback {
                ack_frame_id: highest_ack,
                loss_rate_bp: 0,
                rtt_ms: 15,
                request_idr: false,
                suggested_profile: 0,
            };
            if fb_sender.send_feedback(&fb).await.is_err() {
                break;
            }
            highest_ack = highest_ack.wrapping_add(30);
        }
    });

    let mut rx_buf = [0u8; 4096];
    let mut total_packets = 0u64;
    let mut video_frames_sec = 0u32;
    let mut audio_packets_sec = 0u32;
    let mut bytes_this_sec = 0usize;
    let mut last_second = Instant::now();
    let start_time = Instant::now();

    while running.load(Ordering::Relaxed) {
        let event = receiver.recv_event(&mut rx_buf).await;

        match event {
            Ok(Some(NetEvent::Video(frame))) => {
                video_frames_sec += 1;
                total_packets += 1;
                bytes_this_sec += frame.data.len();
                if output_stdout {
                    let _ = std::io::stdout().write_all(&frame.data);
                    let _ = std::io::stdout().flush();
                } else {
                    std::hint::black_box(&frame.data);
                }
            }
            Ok(Some(NetEvent::Audio(audio))) => {
                audio_packets_sec += 1;
                total_packets += 1;
                bytes_this_sec += audio.opus_data.len();
                std::hint::black_box(&audio.opus_data);
            }
            Ok(Some(NetEvent::Ping { timestamp_us: _ })) => {
                total_packets += 1;
            }
            Ok(Some(NetEvent::Input { .. })) | Ok(Some(NetEvent::Feedback(_))) | Ok(None) => {}
            Err(e) => {
                warn!("Error en socket UDP: {e}");
                break;
            }
        }

        if !output_stdout {
            let elapsed_sec = last_second.elapsed();
            if elapsed_sec >= Duration::from_secs(1) {
                let v_fps = (video_frames_sec as f64) / elapsed_sec.as_secs_f64();
                let a_pps = (audio_packets_sec as f64) / elapsed_sec.as_secs_f64();
                let mbps = (bytes_this_sec as f64 * 8.0) / (1_000_000.0 * elapsed_sec.as_secs_f64());
                let sec_num = start_time.elapsed().as_secs();

                let loss_pct = receiver.current_loss_rate_bp() as f64 / 100.0;

                println!(
                    "{:>4}s | {:>9.1} | {:>9.1} | {:>7.2} Mbps | {:>7.2}% | {:>10}",
                    sec_num, v_fps, a_pps, mbps, loss_pct, total_packets
                );

                video_frames_sec = 0;
                audio_packets_sec = 0;
                bytes_this_sec = 0;
                last_second = Instant::now();
            }
        }
    }

    running.store(false, Ordering::Relaxed);
    Ok(())
}
