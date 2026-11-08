use gamestream_common::VideoFrame;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum EncodeError {
    #[error("Hardware encoder not available: {0}")]
    HardwareUnavailable(String),
    #[error("Encoding frame failed: {0}")]
    EncodeFailed(String),
}

pub trait VideoEncoder: Send {
    fn encode(&mut self, frame: &VideoFrame) -> Result<Vec<u8>, EncodeError>;
}
