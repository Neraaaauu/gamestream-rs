pub mod wayland_pipewire;
pub mod x11_shm;

use gamestream_common::VideoFrame;
use thiserror::Error;
use tracing::info;

#[derive(Error, Debug)]
pub enum CaptureError {
    #[error("X11 error: {0}")]
    X11(String),
    #[error("X11 SHM extension not supported")]
    ShmNotSupported,
    #[error("Wayland / PipeWire error: {0}")]
    PipeWire(String),
    #[error("System memory allocation error: {0}")]
    SystemMemory(String),
    #[error("Failed to capture screen: {0}")]
    CaptureFailed(String),
}

pub trait ScreenCapture: Send {
    /// Dimensiones actuales del área de captura (width, height)
    fn resolution(&self) -> (u32, u32);

    /// Captura un frame del escritorio
    fn capture_frame(&mut self) -> Result<VideoFrame, CaptureError>;
}

impl ScreenCapture for Box<dyn ScreenCapture> {
    fn resolution(&self) -> (u32, u32) {
        (**self).resolution()
    }

    fn capture_frame(&mut self) -> Result<VideoFrame, CaptureError> {
        (**self).capture_frame()
    }
}

/// Crea automáticamente el capturador adecuado según el entorno de escritorio (X11 vs Wayland)
pub fn create_default_capture() -> Result<Box<dyn ScreenCapture>, CaptureError> {
    if wayland_pipewire::WaylandPipeWireCapture::is_wayland_session() {
        info!("Entorno Wayland detectado: activando capturador PipeWire...");
        let cap = wayland_pipewire::WaylandPipeWireCapture::new()?;
        Ok(Box::new(cap))
    } else {
        info!("Entorno X11 detectado: activando capturador X11 SHM de alta velocidad...");
        let cap = x11_shm::X11ShmCapture::new()?;
        Ok(Box::new(cap))
    }
}
