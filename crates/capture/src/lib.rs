pub mod x11_shm;

use gamestream_common::VideoFrame;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum CaptureError {
    #[error("X11 error: {0}")]
    X11(String),
    #[error("X11 SHM extension not supported")]
    ShmNotSupported,
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
