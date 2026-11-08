mod metrics;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use clap::Parser;
use gamestream_capture::x11_shm::X11ShmCapture;
use gamestream_capture::ScreenCapture;
use metrics::ProcessMetrics;
use tracing_subscriber::EnvFilter;

#[derive(Parser, Debug)]
#[command(name = "gamestream-host")]
#[command(about = "Host nativo de ultra baja latencia para Linux en Rust", long_about = None)]
struct Cli {
    /// Activar modo benchmark para medir FPS reales y latencia de captura
    #[arg(short, long, default_value_t = false)]
    bench: bool,

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

    let (width, height) = capture.resolution();
    tracing::info!(
        "Captura inicializada exitosamente: resolución nativa {}x{}",
        width,
        height
    );

    let duration = cli.duration;
    let target_fps = cli.target_fps;

    if cli.bench {
        tokio::task::spawn_blocking(move || {
            run_benchmark(&mut capture, duration, target_fps)
        })
        .await??;
    } else {
        tracing::info!("Modo normal: ejecutando captura de prueba (5 segundos a 60 FPS)");
        tracing::info!("Tip: Para benchmarking exhaustivo, usa `gamestream-host --bench --duration 10`");
        tokio::task::spawn_blocking(move || {
            run_benchmark(&mut capture, 5, target_fps)
        })
        .await??;
    }

    tracing::info!("Ejecución finalizada con éxito.");
    Ok(())
}

fn run_benchmark(
    capture: &mut impl ScreenCapture,
    duration_secs: u64,
    target_fps: u32,
) -> Result<()> {
    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();

    // Capturar Ctrl+C para salida ordenada
    ctrlc_handler(r);

    let (width, height) = capture.resolution();
    let frame_interval = if target_fps > 0 {
        Duration::from_secs_f64(1.0 / (target_fps as f64))
    } else {
        Duration::ZERO
    };

    println!("\n========================================================");
    println!("INICIANDO BENCHMARK DE CAPTURA X11 SHM");
    println!("========================================================");
    println!("Resolución:        {}x{}", width, height);
    println!("Duración objetivo: {} segundos", duration_secs);
    if target_fps > 0 {
        println!("FPS objetivo:      {} FPS (intervalo: {:.2} ms)", target_fps, frame_interval.as_secs_f64() * 1000.0);
    } else {
        println!("FPS objetivo:      SIN LÍMITE (throughput máximo)");
    }
    println!("--------------------------------------------------------");
    println!("{:>6} | {:>7} | {:>10} | {:>10} | {:>8} | {:>7}", "Seg", "FPS", "Avg Lat (ms)", "P99 Lat (ms)", "CPU %", "RAM (MB)");
    println!("--------------------------------------------------------");

    let mut process_metrics = ProcessMetrics::new();
    let start_total = Instant::now();
    let mut last_second_mark = Instant::now();
    let mut total_frames: u64 = 0;
    let mut frames_this_second: u32 = 0;
    let mut latencies_us: Vec<u64> = Vec::with_capacity(10_000);
    let mut current_sec_latencies: Vec<u64> = Vec::with_capacity(1000);

    let max_duration = Duration::from_secs(duration_secs);

    while running.load(Ordering::Relaxed) {
        let frame_start = Instant::now();

        // Captura del cuadro
        let frame = capture.capture_frame()
            .context("Error al capturar cuadro de pantalla")?;

        let capture_elapsed = frame_start.elapsed();
        let capture_us = capture_elapsed.as_micros() as u64;

        latencies_us.push(capture_us);
        current_sec_latencies.push(capture_us);
        total_frames += 1;
        frames_this_second += 1;

        // Medir si ha pasado 1 segundo para reporte en vivo
        let elapsed_second = last_second_mark.elapsed();
        if elapsed_second >= Duration::from_secs(1) {
            let (cpu_pct, rss_mb) = process_metrics.sample();
            let current_fps = (frames_this_second as f64) / elapsed_second.as_secs_f64();

            current_sec_latencies.sort_unstable();
            let avg_lat = if !current_sec_latencies.is_empty() {
                let sum: u64 = current_sec_latencies.iter().sum();
                (sum as f64) / (current_sec_latencies.len() as f64) / 1000.0
            } else {
                0.0
            };
            let p99_idx = (current_sec_latencies.len() as f64 * 0.99) as usize;
            let p99_lat = current_sec_latencies
                .get(p99_idx)
                .copied()
                .unwrap_or(0) as f64 / 1000.0;

            let sec_num = start_total.elapsed().as_secs();
            println!(
                "{:>5}s | {:>7.1} | {:>10.3} | {:>10.3} | {:>7.1}% | {:>6.1}",
                sec_num, current_fps, avg_lat, p99_lat, cpu_pct, rss_mb
            );

            frames_this_second = 0;
            current_sec_latencies.clear();
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

        // Prevenir optimización no deseada
        std::hint::black_box(&frame.data);
    }

    let total_elapsed = start_total.elapsed();
    let total_elapsed_secs = total_elapsed.as_secs_f64();
    let overall_fps = (total_frames as f64) / total_elapsed_secs;

    println!("--------------------------------------------------------");
    println!("RESUMEN FINAL DEL BENCHMARK");
    println!("========================================================");
    println!("Tiempo total:         {:.3} segundos", total_elapsed_secs);
    println!("Cuadros capturados:   {}", total_frames);
    println!("FPS promedio real:    {:.2} FPS", overall_fps);

    if !latencies_us.is_empty() {
        latencies_us.sort_unstable();
        let min_lat = latencies_us[0] as f64 / 1000.0;
        let max_lat = latencies_us[latencies_us.len() - 1] as f64 / 1000.0;
        let sum: u64 = latencies_us.iter().sum();
        let avg_lat = (sum as f64) / (latencies_us.len() as f64) / 1000.0;
        let median_idx = latencies_us.len() / 2;
        let median_lat = latencies_us[median_idx] as f64 / 1000.0;
        let p95_idx = (latencies_us.len() as f64 * 0.95) as usize;
        let p95_lat = latencies_us[p95_idx.min(latencies_us.len() - 1)] as f64 / 1000.0;
        let p99_idx = (latencies_us.len() as f64 * 0.99) as usize;
        let p99_lat = latencies_us[p99_idx.min(latencies_us.len() - 1)] as f64 / 1000.0;

        let bytes_per_frame = (width as u64) * (height as u64) * 4;
        let throughput_mb_s = (bytes_per_frame as f64 * overall_fps) / (1024.0 * 1024.0);

        println!("Latencia Mínima:      {:.3} ms", min_lat);
        println!("Latencia Media (Avg): {:.3} ms", avg_lat);
        println!("Latencia Mediana:     {:.3} ms", median_lat);
        println!("Latencia P95:         {:.3} ms", p95_lat);
        println!("Latencia P99:         {:.3} ms", p99_lat);
        println!("Latencia Máxima:      {:.3} ms", max_lat);
        println!("Throughput Memoria:   {:.2} MB/s", throughput_mb_s);

        let target_frame_budget_ms = 16.666;
        let budget_consumed_pct = (avg_lat / target_frame_budget_ms) * 100.0;
        println!("Presupuesto 60 FPS:   {:.1}% del presupuesto de 16.6 ms consumido en captura", budget_consumed_pct);
    }

    let (_, final_rss_mb) = process_metrics.sample();
    println!("Memoria RSS Final:    {:.2} MB", final_rss_mb);
    println!("========================================================\n");

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
