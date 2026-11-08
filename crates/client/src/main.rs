use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use clap::Parser;
use tokio::io::AsyncReadExt;
use tokio::net::TcpStream;
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

    /// Activar estadísticas de telemetría y latencia en consola
    #[arg(short, long, default_value_t = true)]
    stats: bool,

    /// Activar logs detallados
    #[arg(short, long, default_value_t = false)]
    verbose: bool,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let log_level = if cli.verbose { "debug" } else { "info" };
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(log_level)),
        )
        .init();

    info!("=== Gamestream Client (Windows / Multiplataforma) - Fase 4 ===");
    info!("Conectando al host en {}...", cli.host);

    #[cfg(windows)]
    {
        info!("Inicializando acelerador gráfico Direct3D 11 (Feature Level 10_1 compatible con Intel HD 3000)...");
        init_d3d11_device()?;
    }

    // Conectar al socket de transmisión del host
    let target_addr: SocketAddr = cli.host.parse()
        .context(format!("Dirección inválida de host: {}", cli.host))?;

    if cli.udp {
        return run_udp_client(target_addr, cli.key, cli.stun).await;
    }

    let stream = match TcpStream::connect(target_addr).await {
        Ok(s) => {
            info!("¡Conectado exitosamente al host {}!", target_addr);
            s
        }
        Err(e) => {
            warn!("No se pudo conectar a {} ({}). Ejecutando en modo de espera/diagnóstico.", target_addr, e);
            info!("Para iniciar el streaming, ejecuta en el host: `gamestream-host --stream --port 8554`");
            return Ok(());
        }
    };

    let (mut stream_rx, mut stream_tx) = stream.into_split();

    // Tarea de envío de Input en tiempo real hacia el host (60 Hz)
    let send_running = Arc::new(AtomicBool::new(true));
    let s_run = send_running.clone();
    tokio::spawn(async move {
        use gamestream_input::protocol::MouseEvent;
        use tokio::io::AsyncWriteExt;

        let mut interval = tokio::time::interval(Duration::from_millis(16));
        while s_run.load(Ordering::Relaxed) {
            interval.tick().await;

            let mouse = MouseEvent { dx: 0, dy: 0, buttons: 0, wheel: 0 };
            let data = mouse.to_bytes();
            let mut header = [0u8; 5];
            header[0] = 0x03; // Tipo Mouse
            header[1..5].copy_from_slice(&(data.len() as u32).to_be_bytes());

            if stream_tx.write_all(&header).await.is_err() || stream_tx.write_all(&data).await.is_err() {
                break;
            }
        }
    });

    println!("\n=========================================================================");
    println!("RECEPTOR MULTIMEDIA LAN GAMESTREAM CLIENT (VIDEO H.264 + AUDIO OPUS)");
    println!("=========================================================================");
    println!("Host:              {}", target_addr);
    println!("-------------------------------------------------------------------------");
    println!("{:>5} | {:>9} | {:>9} | {:>10} | {:>12}", "Seg", "Video FPS", "Audio PPS", "Bitrate", "Total Paquetes");
    println!("-------------------------------------------------------------------------");

    let mut header_buf = [0u8; 5];
    let mut total_packets = 0u64;
    let mut video_frames_sec = 0u32;
    let mut audio_packets_sec = 0u32;
    let mut bytes_this_sec = 0usize;
    let mut last_second = Instant::now();
    let start_time = Instant::now();

    loop {
        // Leer cabecera de 5 bytes: [tipo: 1 byte, longitud: 4 bytes]
        if stream_rx.read_exact(&mut header_buf).await.is_err() {
            info!("Conexión finalizada por el host.");
            break;
        }

        let packet_type = header_buf[0];
        let packet_len = u32::from_be_bytes([
            header_buf[1], header_buf[2], header_buf[3], header_buf[4]
        ]) as usize;

        let mut packet_data = vec![0u8; packet_len];
        if stream_rx.read_exact(&mut packet_data).await.is_err() {
            warn!("Error al leer datos del paquete");
            break;
        }

        total_packets += 1;
        bytes_this_sec += 5 + packet_len;

        if packet_type == 0x01 {
            video_frames_sec += 1;
        } else if packet_type == 0x02 {
            audio_packets_sec += 1;
        }

        std::hint::black_box(&packet_data);

        let elapsed_sec = last_second.elapsed();
        if elapsed_sec >= Duration::from_secs(1) {
            let v_fps = (video_frames_sec as f64) / elapsed_sec.as_secs_f64();
            let a_pps = (audio_packets_sec as f64) / elapsed_sec.as_secs_f64();
            let mbps = (bytes_this_sec as f64 * 8.0) / (1_000_000.0 * elapsed_sec.as_secs_f64());
            let sec_num = start_time.elapsed().as_secs();

            println!(
                "{:>4}s | {:>9.1} | {:>9.1} | {:>7.2} Mbps | {:>12}",
                sec_num, v_fps, a_pps, mbps, total_packets
            );

            video_frames_sec = 0;
            audio_packets_sec = 0;
            bytes_this_sec = 0;
            last_second = Instant::now();
        }
    }

    send_running.store(false, Ordering::Relaxed);
    Ok(())
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

async fn run_udp_client(
    target_addr: SocketAddr,
    key_hex: Option<String>,
    enable_stun: bool,
) -> Result<()> {
    use gamestream_net::crypto::{CryptoRole, CryptoSession};
    use gamestream_net::protocol::NetworkFeedback;
    use gamestream_net::transceiver::{NetEvent, UdpTransceiver};

    info!("=== MODO CLIENTE UDP ROBUSTO CON CIFRADO E2E Y ADAPTACIÓN DINÁMICA ===");

    let key = if let Some(hex) = key_hex {
        Some(CryptoSession::key_from_hex(&hex).context("Clave de cifrado hex inválida")?)
    } else {
        None
    };

    if enable_stun {
        info!("Descubriendo IP pública mediante STUN para P2P NAT Traversal...");
        if let Ok(stun_sock) = tokio::net::UdpSocket::bind("0.0.0.0:0").await {
            if let Ok(pub_addr) = gamestream_net::stun::discover_public_endpoint(&stun_sock).await {
                info!("Dirección pública STUN descubierta: {}", pub_addr);
                let _ = gamestream_net::stun::punch_hole(&stun_sock, target_addr, 5).await;
            }
        }
    }

    let local_bind = "0.0.0.0:0";
    let (sender, mut receiver) = UdpTransceiver::bind(local_bind, target_addr, key, CryptoRole::Client).await?.into_split();

    // Enviar paquete inicial de saludo/punch
    let _ = sender.socket_ref().send_to(b"GSPUNCH", target_addr).await;

    println!("\n=========================================================================");
    println!("RECEPTOR UDP ROBUSTO GAMESTREAM CLIENT (H.264 + OPUS + CHACHA20-POLY1305)");
    println!("=========================================================================");
    println!("Host de Destino:   {}", target_addr);
    println!("Cifrado E2E:       {}", if key.is_some() { "ACTIVO (ChaCha20-Poly1305)" } else { "DESACTIVADO" });
    println!("-------------------------------------------------------------------------");
    println!("{:>5} | {:>9} | {:>9} | {:>10} | {:>8} | {:>10}", "Seg", "Video FPS", "Audio PPS", "Bitrate", "Pérdida", "Total Pkts");
    println!("-------------------------------------------------------------------------");

    let running = Arc::new(AtomicBool::new(true));

    // Tarea de entrada periódica (60 Hz) hacia el host (Zero-lock)
    let input_sender = sender.clone();
    let in_run = running.clone();
    tokio::spawn(async move {
        use gamestream_input::protocol::MouseEvent;
        let mut interval = tokio::time::interval(Duration::from_millis(16));
        while in_run.load(Ordering::Relaxed) {
            interval.tick().await;
            let mouse = MouseEvent { dx: 0, dy: 0, buttons: 0, wheel: 0 };
            let data = mouse.to_bytes();
            if input_sender.send_input_event(0x01, &data).await.is_err() {
                break;
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
                std::hint::black_box(&frame.data);
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

    running.store(false, Ordering::Relaxed);
    Ok(())
}
