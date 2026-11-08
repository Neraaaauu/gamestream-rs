mod metrics;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use clap::Parser;
use gamestream_capture::ScreenCapture;
use gamestream_encode::h264::H264Encoder;
use gamestream_encode::VideoEncoder;
use metrics::ProcessMetrics;
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
#[command(name = "gamestream-host")]
#[command(about = "Host nativo de ultra baja latencia para Linux en Rust", long_about = None)]
struct Cli {
    /// Activar modo benchmark para medir FPS reales y latencia de captura
    #[arg(short, long, default_value_t = false)]
    bench: bool,

    /// Transmitir video en vivo a clientes por la red LAN (Fase 4)
    #[arg(short, long, default_value_t = false)]
    stream: bool,

    /// Puerto TCP o UDP para streaming
    #[arg(short, long, default_value_t = 8554)]
    port: u16,

    /// Activar streaming UDP robusto con cifrado E2E ChaCha20-Poly1305 (Fase 7)
    #[arg(long, default_value_t = false)]
    udp: bool,

    /// Clave de cifrado hex de 32 bytes (64 chars). Si se omite, se genera una aleatoria automáticamente.
    #[arg(long)]
    key: Option<String>,

    /// Dirección IP:puerto del peer cliente para UDP (ej. 192.168.1.50:8555)
    #[arg(long)]
    peer: Option<String>,

    /// Descubrir IP pública y puerto reflexivo mediante STUN (para jugar por Internet)
    #[arg(long, default_value_t = false)]
    stun: bool,

    /// Activar codificación H.264 (Fase 3: captura + reescalado + compresión)
    #[arg(short, long, default_value_t = false)]
    encode: bool,

    /// Ancho de resolución de salida/transmisión (ej. 1280 para 720p, 1024 para sub-HD)
    #[arg(long, default_value_t = 1280)]
    width: u32,

    /// Alto de resolución de salida/transmisión (ej. 720 para 720p, 576 para sub-HD)
    #[arg(long, default_value_t = 720)]
    height: u32,

    /// Bitrate objetivo en kbps
    #[arg(short = 'B', long, default_value_t = 4500)]
    bitrate: u32,

    /// Duración del benchmark en segundos
    #[arg(short, long, default_value_t = 5)]
    duration: u64,

    /// Límite de FPS objetivo (0 para sin límite / máximo rendimiento posible)
    #[arg(short, long, default_value_t = 60)]
    target_fps: u32,

    /// Activar logs detallados de depuración
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

    tracing::info!("=== Gamestream Host (Linux) ===");
    tracing::info!("Detectando entorno gráfico e inicializando subsistema de captura...");

    let mut capture = gamestream_capture::create_default_capture()
        .context("No se pudo inicializar la captura de pantalla (X11 / Wayland)")?;

    let (src_width, src_height) = capture.resolution();
    tracing::info!(
        "Captura inicializada exitosamente: resolución nativa {}x{}",
        src_width,
        src_height
    );

    let duration = cli.duration;
    let target_fps = cli.target_fps;
    let encode_enabled = cli.encode || cli.stream || cli.udp;
    let out_w = cli.width;
    let out_h = cli.height;
    let bitrate_bps = (cli.bitrate as i64) * 1000;
    let port = cli.port;
    let stream_mode = cli.stream;
    let udp_mode = cli.udp;
    let peer_arg = cli.peer;
    let key_arg = cli.key;
    let stun_arg = cli.stun;

    if udp_mode {
        tracing::info!("Modo Streaming UDP Robusto activado en puerto {}", port);
        run_udp_stream(
            capture,
            out_w,
            out_h,
            target_fps,
            bitrate_bps,
            port,
            peer_arg,
            key_arg,
            stun_arg,
        ).await?;
    } else if stream_mode {
        tracing::info!("Modo Streaming en LAN activado en puerto {}", port);
        run_lan_stream(capture, out_w, out_h, target_fps, bitrate_bps, port).await?;
    } else if cli.bench {
        tokio::task::spawn_blocking(move || -> Result<()> {
            let encoder: Option<Box<dyn VideoEncoder>> = if encode_enabled {
                let enc = H264Encoder::new(src_width, src_height, out_w, out_h, target_fps.max(1), bitrate_bps)
                    .context("Fallo al inicializar el encoder H.264")?;
                Some(Box::new(enc))
            } else {
                None
            };

            run_benchmark(&mut capture, encoder, duration, target_fps)
        })
        .await??;
    } else {
        tracing::info!("Modo normal: ejecutando captura de prueba (5 segundos a 60 FPS)");
        tracing::info!("Tip: Usa `gamestream-host --stream --port 8554` para transmitir en LAN al cliente Windows.");
        tokio::task::spawn_blocking(move || -> Result<()> {
            run_benchmark(&mut capture, None, 5, target_fps)
        })
        .await??;
    }

    tracing::info!("Ejecución finalizada con éxito.");
    Ok(())
}

async fn run_lan_stream(
    mut capture: impl ScreenCapture + 'static,
    out_w: u32,
    out_h: u32,
    target_fps: u32,
    bitrate_bps: i64,
    port: u16,
) -> Result<()> {
    use tokio::io::AsyncWriteExt;
    use tokio::net::TcpListener;
    use gamestream_audio::opus_enc::OpusAudioEncoder;
    use gamestream_audio::pulse_cap::PulseAudioCapture;

    let addr = format!("0.0.0.0:{}", port);
    let listener = TcpListener::bind(&addr).await
        .context(format!("No se pudo enlazar socket TCP en {}", addr))?;

    tracing::info!("============================================================");
    tracing::info!("SERVIDOR LAN A/V INICIADO: Escuchando en {}", addr);
    tracing::info!("Transmitiendo Video H.264 + Audio Opus sincronizado a 10 ms");
    tracing::info!("============================================================");

    let (src_w, src_h) = capture.resolution();
    let frame_interval = Duration::from_secs_f64(1.0 / (target_fps.max(1) as f64));

    loop {
        tracing::info!("Esperando conexión de un cliente...");
        let (socket, client_addr) = listener.accept().await?;
        tracing::info!("Cliente conectado desde: {}", client_addr);

        let (mut socket_rx, mut socket_tx) = socket.into_split();

        let mut encoder = H264Encoder::new(src_w, src_h, out_w, out_h, target_fps.max(1), bitrate_bps)
            .context("Error al inicializar encoder H.264 para cliente")?;

        // Tarea asíncrona para recibir e inyectar Input desde el cliente (Mouse, Teclado, Mando)
        let input_running = Arc::new(AtomicBool::new(true));
        let in_run = input_running.clone();
        tokio::spawn(async move {
            use gamestream_input::injector::LinuxInputInjector;
            use gamestream_input::protocol::{GamepadEvent, KeyEvent, MouseEvent};
            use tokio::io::AsyncReadExt;

            let mut injector = match LinuxInputInjector::new() {
                Ok(inj) => inj,
                Err(e) => {
                    tracing::warn!("No se pudo inicializar inyector de entrada: {e}");
                    return;
                }
            };

            let mut header = [0u8; 5];
            while in_run.load(Ordering::Relaxed) {
                if socket_rx.read_exact(&mut header).await.is_err() {
                    break;
                }
                let p_type = header[0];
                let p_len = u32::from_be_bytes([header[1], header[2], header[3], header[4]]) as usize;
                let mut data = vec![0u8; p_len];
                if socket_rx.read_exact(&mut data).await.is_err() {
                    break;
                }

                match p_type {
                    0x03 => {
                        if let Some(m_ev) = MouseEvent::from_bytes(&data) {
                            let _ = injector.inject_mouse(&m_ev);
                        }
                    }
                    0x04 => {
                        if let Some(k_ev) = KeyEvent::from_bytes(&data) {
                            let _ = injector.inject_keyboard(&k_ev);
                        }
                    }
                    0x05 => {
                        if let Some(g_ev) = GamepadEvent::from_bytes(&data) {
                            let _ = injector.inject_gamepad(&g_ev);
                        }
                    }
                    _ => {}
                }
            }
        });

        // Canal MPSC para audio en tiempo real desde el hilo de captura
        let (audio_tx, mut audio_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(100);
        let audio_running = Arc::new(AtomicBool::new(true));
        let a_run = audio_running.clone();

        // Iniciar hilo de captura y codificación Opus (10 ms / 100 pps)
        std::thread::spawn(move || {
            let mut pulse = match PulseAudioCapture::new() {
                Ok(p) => p,
                Err(e) => {
                    tracing::warn!("No se pudo iniciar captura de audio PulseAudio: {e}");
                    return;
                }
            };
            let mut opus = match OpusAudioEncoder::new(48000, 2, 96000) {
                Ok(o) => o,
                Err(e) => {
                    tracing::warn!("No se pudo iniciar encoder Opus: {e}");
                    return;
                }
            };

            while a_run.load(Ordering::Relaxed) {
                if let Ok(pcm) = pulse.read_10ms_frame() {
                    if let Ok(opus_packet) = opus.encode(&pcm) {
                        if audio_tx.blocking_send(opus_packet).is_err() {
                            break;
                        }
                    }
                }
            }
        });

        loop {
            let start = Instant::now();

            // 1. Drenar y enviar paquetes de audio Opus pendientes (Tipo 0x02)
            while let Ok(audio_data) = audio_rx.try_recv() {
                let mut header = [0u8; 5];
                header[0] = 0x02; // Tipo: Audio Opus
                let len_bytes = (audio_data.len() as u32).to_be_bytes();
                header[1..5].copy_from_slice(&len_bytes);

                if socket_tx.write_all(&header).await.is_err() || socket_tx.write_all(&audio_data).await.is_err() {
                    break;
                }
            }

            // 2. Capturar y codificar cuadro de video H.264 (Tipo 0x01)
            let frame = match capture.capture_frame() {
                Ok(f) => f,
                Err(e) => {
                    tracing::error!("Error de captura: {e}");
                    break;
                }
            };

            let encoded = match encoder.encode(&frame) {
                Ok(data) => data,
                Err(e) => {
                    tracing::error!("Error de codificación: {e}");
                    break;
                }
            };

            if !encoded.is_empty() {
                let mut header = [0u8; 5];
                header[0] = 0x01; // Tipo: Video H.264
                let len_bytes = (encoded.len() as u32).to_be_bytes();
                header[1..5].copy_from_slice(&len_bytes);

                if socket_tx.write_all(&header).await.is_err() || socket_tx.write_all(&encoded).await.is_err() {
                    tracing::warn!("Cliente {} desconectado.", client_addr);
                    break;
                }
            }

            let elapsed = start.elapsed();
            if elapsed < frame_interval {
                tokio::time::sleep(frame_interval - elapsed).await;
            }
        }

        audio_running.store(false, Ordering::Relaxed);
        input_running.store(false, Ordering::Relaxed);
    }
}

fn run_benchmark(
    capture: &mut impl ScreenCapture,
    mut encoder: Option<Box<dyn VideoEncoder>>,
    duration_secs: u64,
    target_fps: u32,
) -> Result<()> {
    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();

    // Capturar Ctrl+C para salida ordenada
    ctrlc_handler(r);

    let (src_w, src_h) = capture.resolution();
    let frame_interval = if target_fps > 0 {
        Duration::from_secs_f64(1.0 / (target_fps as f64))
    } else {
        Duration::ZERO
    };

    println!("\n===============================================================================");
    println!("INICIANDO BENCHMARK DE PIPELINE: {}", if encoder.is_some() { "CAPTURA + REESCALADO + ENCODE H.264" } else { "CAPTURA X11 SHM PURA" });
    println!("===============================================================================");
    println!("Resolución Escritorio:  {}x{}", src_w, src_h);
    if let Some(enc) = encoder.as_deref() {
        println!("Encoder H.264:          {} (Acelerado por HW: {})", enc.name(), enc.is_hardware_accelerated());
    }
    println!("Duración:               {} segundos", duration_secs);
    if target_fps > 0 {
        println!("FPS objetivo:           {} FPS (intervalo: {:.2} ms)", target_fps, frame_interval.as_secs_f64() * 1000.0);
    } else {
        println!("FPS objetivo:           SIN LÍMITE (throughput máximo)");
    }
    println!("-------------------------------------------------------------------------------");
    if encoder.is_some() {
        println!("{:>4} | {:>6} | {:>10} | {:>10} | {:>10} | {:>9} | {:>6} | {:>7}", "Seg", "FPS", "Cap Lat ms", "Enc Lat ms", "Tot Lat ms", "Bitrate", "CPU %", "RAM MB");
    } else {
        println!("{:>4} | {:>6} | {:>10} | {:>10} | {:>6} | {:>7}", "Seg", "FPS", "Cap Lat ms", "P99 Lat ms", "CPU %", "RAM MB");
    }
    println!("-------------------------------------------------------------------------------");

    let mut process_metrics = ProcessMetrics::new();
    let start_total = Instant::now();
    let mut last_second_mark = Instant::now();
    let mut total_frames: u64 = 0;
    let mut frames_this_second: u32 = 0;
    let mut bytes_this_second: usize = 0;
    let mut total_encoded_bytes: usize = 0;

    let mut cap_latencies_us: Vec<u64> = Vec::with_capacity(10_000);
    let mut enc_latencies_us: Vec<u64> = Vec::with_capacity(10_000);
    let mut tot_latencies_us: Vec<u64> = Vec::with_capacity(10_000);

    let max_duration = Duration::from_secs(duration_secs);

    while running.load(Ordering::Relaxed) {
        let frame_start = Instant::now();

        // 1. Captura del cuadro
        let frame = capture.capture_frame()
            .context("Error al capturar cuadro de pantalla")?;
        let cap_elapsed_us = frame_start.elapsed().as_micros() as u64;

        // 2. Codificación H.264 si está habilitada
        let (enc_elapsed_us, pkt_size) = if let Some(enc) = encoder.as_deref_mut() {
            let enc_start = Instant::now();
            let encoded = enc.encode(&frame)
                .context("Error al codificar frame H.264")?;
            let el = enc_start.elapsed().as_micros() as u64;
            let sz = encoded.len();
            std::hint::black_box(&encoded);
            (el, sz)
        } else {
            (0, 0)
        };

        let total_work_us = frame_start.elapsed().as_micros() as u64;

        cap_latencies_us.push(cap_elapsed_us);
        if encoder.is_some() {
            enc_latencies_us.push(enc_elapsed_us);
            tot_latencies_us.push(total_work_us);
        }

        total_frames += 1;
        frames_this_second += 1;
        bytes_this_second += pkt_size;
        total_encoded_bytes += pkt_size;

        // Medir si ha pasado 1 segundo para reporte en vivo
        let elapsed_second = last_second_mark.elapsed();
        if elapsed_second >= Duration::from_secs(1) {
            let (cpu_pct, rss_mb) = process_metrics.sample();
            let current_fps = (frames_this_second as f64) / elapsed_second.as_secs_f64();
            let sec_num = start_total.elapsed().as_secs();

            if encoder.is_some() {
                let mbps = (bytes_this_second as f64 * 8.0) / (1_000_000.0 * elapsed_second.as_secs_f64());
                let avg_cap = cap_latencies_us.iter().rev().take(frames_this_second as usize).copied().sum::<u64>() as f64 / (frames_this_second.max(1) as f64 * 1000.0);
                let avg_enc = enc_latencies_us.iter().rev().take(frames_this_second as usize).copied().sum::<u64>() as f64 / (frames_this_second.max(1) as f64 * 1000.0);
                let avg_tot = tot_latencies_us.iter().rev().take(frames_this_second as usize).copied().sum::<u64>() as f64 / (frames_this_second.max(1) as f64 * 1000.0);

                println!(
                    "{:>3}s | {:>6.1} | {:>10.3} | {:>10.3} | {:>10.3} | {:>6.2} Mbps | {:>5.1}% | {:>6.1}",
                    sec_num, current_fps, avg_cap, avg_enc, avg_tot, mbps, cpu_pct, rss_mb
                );
            } else {
                let avg_cap = cap_latencies_us.iter().rev().take(frames_this_second as usize).copied().sum::<u64>() as f64 / (frames_this_second.max(1) as f64 * 1000.0);
                println!(
                    "{:>3}s | {:>6.1} | {:>10.3} | {:>10.3} | {:>5.1}% | {:>6.1}",
                    sec_num, current_fps, avg_cap, avg_cap, cpu_pct, rss_mb
                );
            }

            frames_this_second = 0;
            bytes_this_second = 0;
            last_second_mark = Instant::now();
        }

        // Finalizar si se cumplió la duración
        if start_total.elapsed() >= max_duration {
            break;
        }

        // Pacing: limitar a FPS objetivo si corresponde
        if target_fps > 0 {
            let total_frame_work = frame_start.elapsed();
            if total_frame_work < frame_interval {
                std::thread::sleep(frame_interval - total_frame_work);
            }
        }

        std::hint::black_box(&frame.data);
    }

    let total_elapsed = start_total.elapsed();
    let total_elapsed_secs = total_elapsed.as_secs_f64();
    let overall_fps = (total_frames as f64) / total_elapsed_secs;

    println!("-------------------------------------------------------------------------------");
    println!("RESUMEN FINAL DEL BENCHMARK");
    println!("===============================================================================");
    println!("Tiempo total:         {:.3} segundos", total_elapsed_secs);
    println!("Cuadros procesados:   {}", total_frames);
    println!("FPS promedio real:    {:.2} FPS", overall_fps);

    if !cap_latencies_us.is_empty() {
        let avg_cap = (cap_latencies_us.iter().sum::<u64>() as f64) / (cap_latencies_us.len() as f64 * 1000.0);
        println!("Latencia Media Captura: {:.3} ms", avg_cap);

        if encoder.is_some() && !enc_latencies_us.is_empty() {
            enc_latencies_us.sort_unstable();
            tot_latencies_us.sort_unstable();

            let avg_enc = (enc_latencies_us.iter().sum::<u64>() as f64) / (enc_latencies_us.len() as f64 * 1000.0);
            let med_enc = enc_latencies_us[enc_latencies_us.len() / 2] as f64 / 1000.0;
            let p95_enc = enc_latencies_us[(enc_latencies_us.len() as f64 * 0.95) as usize] as f64 / 1000.0;

            let avg_tot = (tot_latencies_us.iter().sum::<u64>() as f64) / (tot_latencies_us.len() as f64 * 1000.0);
            let med_tot = tot_latencies_us[tot_latencies_us.len() / 2] as f64 / 1000.0;
            let p95_tot = tot_latencies_us[(tot_latencies_us.len() as f64 * 0.95) as usize] as f64 / 1000.0;

            let avg_mbps = (total_encoded_bytes as f64 * 8.0) / (1_000_000.0 * total_elapsed_secs);

            println!("Latencia Media Encode:  {:.3} ms (Mediana: {:.3} ms, P95: {:.3} ms)", avg_enc, med_enc, p95_enc);
            println!("Latencia TOTAL Host:    {:.3} ms (Mediana: {:.3} ms, P95: {:.3} ms)", avg_tot, med_tot, p95_tot);
            println!("Bitrate H.264 Real:     {:.2} Mbps", avg_mbps);

            let budget_consumed = (avg_tot / 16.666) * 100.0;
            println!("Presupuesto 60 FPS:     {:.1}% del presupuesto consumido en Host (Captura+Escalado+Encode)", budget_consumed);
        }
    }

    let (_, final_rss_mb) = process_metrics.sample();
    println!("Memoria RSS Final:      {:.2} MB", final_rss_mb);
    println!("===============================================================================\n");

    Ok(())
}

fn ctrlc_handler(running: Arc<AtomicBool>) {
    // Si se presiona Ctrl+C, salir limpiamente
    let r = running;
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            println!("\nRecibida señal de interrupción, finalizando...");
            r.store(false, Ordering::Relaxed);
        }
    });
}

#[allow(clippy::too_many_arguments)]
async fn run_udp_stream(
    mut capture: impl ScreenCapture + 'static,
    _initial_w: u32,
    _initial_h: u32,
    target_fps: u32,
    bitrate_bps: i64,
    port: u16,
    peer_str: Option<String>,
    key_hex: Option<String>,
    enable_stun: bool,
) -> Result<()> {
    use std::net::SocketAddr;
    use gamestream_audio::opus_enc::OpusAudioEncoder;
    use gamestream_audio::pulse_cap::PulseAudioCapture;
    use gamestream_net::adaptive::AdaptiveController;
    use gamestream_net::crypto::{CryptoRole, CryptoSession};
    use gamestream_net::protocol::NetworkFeedback;
    use gamestream_net::transceiver::{NetEvent, UdpTransceiver};

    // 1. Clave de cifrado extremo a extremo
    let key = if let Some(hex) = key_hex {
        CryptoSession::key_from_hex(&hex)
            .context("Clave hexadecimal de cifrado inválida")?
    } else {
        CryptoSession::generate_key()
    };
    let hex_display = CryptoSession::key_to_hex(&key);

    println!("\n============================================================");
    println!("GAMESTREAM HOST - SERVIDOR UDP DE ULTRA BAJA LATENCIA (FASE 7)");
    println!("============================================================");
    println!("Clave de Cifrado E2E (ChaCha20-Poly1305):");
    println!("  {}", hex_display);
    println!("Proporciona esta clave a tu cliente para descifrado en tiempo real.");
    println!("------------------------------------------------------------");

    // 2. Consulta STUN si está habilitada
    let local_bind = format!("0.0.0.0:{}", port);
    if enable_stun {
        println!("Consultando servidor STUN (RFC 5389) para NAT Traversal...");
        match tokio::net::UdpSocket::bind("0.0.0.0:0").await {
            Ok(stun_sock) => {
                match gamestream_net::stun::discover_public_endpoint(&stun_sock).await {
                    Ok(public_endpoint) => {
                        println!(">>> DIRECCIÓN PÚBLICA STUN DESCUBIERTA: {} <<<", public_endpoint);
                        println!("Tus amigos pueden conectarse a esta IP:puerto por Internet sin abrir puertos.");
                    }
                    Err(e) => {
                        tracing::warn!("STUN discovery no tuvo respuesta: {e}");
                    }
                }
            }
            Err(e) => tracing::warn!("No se pudo enlazar socket temporal STUN: {e}"),
        }
    }

    // 3. Resolver dirección del Peer
    let peer_addr = if let Some(ref p) = peer_str {
        p.parse::<SocketAddr>()
            .context(format!("Dirección de peer inválida: {}", p))?
    } else {
        println!("Escuchando en socket UDP {}...", local_bind);
        println!("Esperando paquete de saludo o handshake del cliente...");
        let socket = tokio::net::UdpSocket::bind(&local_bind)
            .await
            .context(format!("No se pudo enlazar socket UDP en {}", local_bind))?;

        let mut buf = [0u8; 512];
        let (_len, from_addr) = socket.recv_from(&mut buf).await
            .context("Error recibiendo paquete inicial UDP")?;

        println!("¡Cliente detectado desde {}! Estableciendo flujo P2P...", from_addr);
        // Responder con punch
        let _ = socket.send_to(b"GSPUNCH", from_addr).await;
        // Soltar el socket para que UdpTransceiver lo enlace en el puerto
        drop(socket);
        from_addr
    };

    println!("Conexión P2P establecida con {}", peer_addr);
    println!("============================================================\n");

    let (sender, mut receiver) = UdpTransceiver::bind(&local_bind, peer_addr, Some(key), CryptoRole::Host).await?.into_split();

    let (src_width, src_height) = capture.resolution();
    let mut adaptive = AdaptiveController::new(0); // 720p60 inicial
    let mut current_profile = adaptive.current_profile();
    let mut current_bitrate = bitrate_bps.max(current_profile.rec_bitrate_bps);

    let mut encoder = H264Encoder::new(
        src_width,
        src_height,
        current_profile.width,
        current_profile.height,
        target_fps.max(1),
        current_bitrate,
    )?;

    // Canal para recibir feedback de red desde la tarea asíncrona de recepción
    let (feedback_tx, mut feedback_rx) = tokio::sync::mpsc::channel::<NetworkFeedback>(32);

    // Inyector de entrada virtual en Linux
    #[cfg(target_os = "linux")]
    let mut input_injector = gamestream_input::injector::LinuxInputInjector::new().ok();

    // Tarea receptora UDP de entrada y feedback de red (Zero locks)
    let rx_running = Arc::new(AtomicBool::new(true));
    let r_run = rx_running.clone();

    tokio::spawn(async move {
        let mut rx_buf = [0u8; 4096];
        while r_run.load(Ordering::Relaxed) {
            let event = receiver.recv_event(&mut rx_buf).await;

            match event {
                Ok(Some(NetEvent::Feedback(fb))) => {
                    let _ = feedback_tx.send(fb).await;
                }
                Ok(Some(NetEvent::Input { kind, data })) => {
                    #[cfg(target_os = "linux")]
                    if let Some(ref mut inj) = input_injector {
                        match kind {
                            0x01 if data.len() >= 8 => {
                                if let Some(m) = gamestream_input::protocol::MouseEvent::from_bytes(&data[..8]) {
                                    let _ = inj.inject_mouse(&m);
                                }
                            }
                            0x02 if data.len() >= 4 => {
                                if let Some(k) = gamestream_input::protocol::KeyEvent::from_bytes(&data[..4]) {
                                    let _ = inj.inject_keyboard(&k);
                                }
                            }
                            0x03 if data.len() >= 16 => {
                                if let Some(g) = gamestream_input::protocol::GamepadEvent::from_bytes(&data[..16]) {
                                    let _ = inj.inject_gamepad(&g);
                                }
                            }
                            _ => {}
                        }
                    }
                }
                Ok(Some(NetEvent::Ping { timestamp_us: _ })) => {
                    // Ping recibido, conexión viva
                }
                _ => {}
            }
        }
    });

    // Iniciar hilo de captura y codificación Opus (10 ms / 100 pps)
    let (audio_tx, mut audio_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(100);
    let audio_running = Arc::new(AtomicBool::new(true));
    let a_run = audio_running.clone();

    std::thread::spawn(move || {
        let mut pulse = match PulseAudioCapture::new() {
            Ok(p) => p,
            Err(e) => {
                tracing::warn!("No se pudo iniciar captura PulseAudio: {e}");
                return;
            }
        };
        let mut opus = match OpusAudioEncoder::new(48000, 2, 96000) {
            Ok(o) => o,
            Err(e) => {
                tracing::warn!("No se pudo iniciar encoder Opus: {e}");
                return;
            }
        };

        while a_run.load(Ordering::Relaxed) {
            if let Ok(pcm) = pulse.read_10ms_frame() {
                if let Ok(opus_pkt) = opus.encode(&pcm) {
                    if audio_tx.blocking_send(opus_pkt).is_err() {
                        break;
                    }
                }
            }
        }
    });

    let running = Arc::new(AtomicBool::new(true));
    ctrlc_handler(running.clone());

    let frame_interval = Duration::from_micros(1_000_000 / (target_fps.max(1) as u64));
    let mut frame_id = 1u32;
    let mut audio_seq = 1u32;

    tracing::info!("Bucle de streaming UDP activo transmitiendo a 60 FPS fijos...");

    while running.load(Ordering::Relaxed) {
        let frame_start = Instant::now();

        // 1. Revisar si hay feedback de red del cliente para adaptación dinámica
        while let Ok(fb) = feedback_rx.try_recv() {
            if let Some(action) = adaptive.on_network_feedback(fb.loss_rate_bp, fb.rtt_ms) {
                if action.profile_changed {
                    tracing::info!(
                        "Adaptación de Resolución: Reescalando dinámicamente a {} ({}x{}) @ 60 FPS",
                        action.new_profile.name,
                        action.new_profile.width,
                        action.new_profile.height
                    );
                    current_profile = action.new_profile;
                    current_bitrate = action.new_bitrate_bps;
                    // Reconfigurar encoder
                    if let Ok(new_enc) = H264Encoder::new(
                        src_width,
                        src_height,
                        current_profile.width,
                        current_profile.height,
                        target_fps.max(1),
                        current_bitrate,
                    ) {
                        encoder = new_enc;
                    }
                } else if action.new_bitrate_bps != current_bitrate {
                    current_bitrate = action.new_bitrate_bps;
                    tracing::info!("Adaptación de Bitrate: Ajustado a {:.2} Mbps", current_bitrate as f64 / 1_000_000.0);
                }
            }
        }

        // 2. Drenar y transmitir paquetes de audio Opus
        while let Ok(opus_data) = audio_rx.try_recv() {
            let _ = sender.send_audio_packet(audio_seq, &opus_data).await;
            audio_seq = audio_seq.wrapping_add(1);
        }

        // 3. Capturar cuadro de video X11 SHM
        let raw_frame = match capture.capture_frame() {
            Ok(f) => f,
            Err(e) => {
                tracing::error!("Error de captura: {e}");
                break;
            }
        };

        // 4. Codificar frame con VA-API / software
        let encoded_bytes = match encoder.encode(&raw_frame) {
            Ok(data) => data,
            Err(e) => {
                tracing::error!("Error codificando cuadro: {e}");
                break;
            }
        };

        if !encoded_bytes.is_empty() {
            // Comprobar si es keyframe
            let is_keyframe = encoded_bytes.windows(4).any(|w| w == [0, 0, 0, 1]);

            if let Err(e) = sender.send_video_frame(frame_id, is_keyframe, &encoded_bytes).await {
                tracing::warn!("Error transmitiendo video UDP: {e}");
            }
            frame_id = frame_id.wrapping_add(1);
        }

        // 5. Mantener ritmo de 60 FPS estricto
        let elapsed = frame_start.elapsed();
        if elapsed < frame_interval {
            tokio::time::sleep(frame_interval - elapsed).await;
        }
    }

    rx_running.store(false, Ordering::Relaxed);
    audio_running.store(false, Ordering::Relaxed);
    println!("Streaming UDP detenido limpiamente.");
    Ok(())
}
