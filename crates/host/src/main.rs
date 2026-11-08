mod metrics;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use clap::Parser;
use gamestream_capture::x11_shm::X11ShmCapture;
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

    /// Puerto TCP para streaming en red LAN
    #[arg(short, long, default_value_t = 8554)]
    port: u16,

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
    #[arg(short, long, default_value_t = 4500)]
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

    tracing::info!("=== Gamestream Host (Linux) - Fase 2 ===");
    tracing::info!("Inicializando subsistema de captura X11 SHM...");

    let mut capture = X11ShmCapture::new()
        .context("No se pudo inicializar la captura X11 SHM")?;

    let (src_width, src_height) = capture.resolution();
    tracing::info!(
        "Captura inicializada exitosamente: resolución nativa {}x{}",
        src_width,
        src_height
    );

    let duration = cli.duration;
    let target_fps = cli.target_fps;
    let encode_enabled = cli.encode || cli.stream;
    let out_w = cli.width;
    let out_h = cli.height;
    let bitrate_bps = (cli.bitrate as i64) * 1000;
    let port = cli.port;
    let stream_mode = cli.stream;

    if stream_mode {
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
        let (mut socket, client_addr) = listener.accept().await?;
        tracing::info!("Cliente conectado desde: {}", client_addr);

        let mut encoder = H264Encoder::new(src_w, src_h, out_w, out_h, target_fps.max(1), bitrate_bps)
            .context("Error al inicializar encoder H.264 para cliente")?;

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

                if socket.write_all(&header).await.is_err() || socket.write_all(&audio_data).await.is_err() {
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

                if socket.write_all(&header).await.is_err() || socket.write_all(&encoded).await.is_err() {
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
