pub mod ffi;
pub mod h264;

use gamestream_common::VideoFrame;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum EncodeError {
    #[error("No valid H.264 encoder found")]
    NoEncoderAvailable,
    #[error("Encoder initialization failed: {0}")]
    InitFailed(String),
    #[error("Failed to send frame to encoder: {0}")]
    SendFrameFailed(String),
    #[error("Failed to receive packet from encoder: {0}")]
    ReceivePacketFailed(String),
    #[error("Scaler initialization failed: {0}")]
    ScalerFailed(String),
}

pub trait VideoEncoder: Send {
    /// Nombre del encoder activo (ej. "h264_vaapi", "libx264")
    fn name(&self) -> &str;

    /// Es acelerado por hardware
    fn is_hardware_accelerated(&self) -> bool;

    /// Codifica un frame de video y retorna los bytes H.264 (NAL units)
    fn encode(&mut self, frame: &VideoFrame) -> Result<Vec<u8>, EncodeError>;

    /// Fuerza la generación de un cuadro IDR (Keyframe) en el siguiente frame
    fn force_idr(&mut self);
}
