pub mod ffi;
pub mod opus_enc;
pub mod pulse_cap;

use std::time::Instant;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum AudioError {
    #[error("Audio device initialization failed: {0}")]
    DeviceInit(String),
    #[error("Audio capture stream failed: {0}")]
    StreamError(String),
}

#[derive(Debug, Clone)]
pub struct AudioPacket {
    pub sequence: u64,
    pub timestamp: Instant,
    pub data: Vec<u8>,
}
