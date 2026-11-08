use thiserror::Error;

#[derive(Error, Debug)]
pub enum AudioError {
    #[error("Audio device initialization failed: {0}")]
    DeviceInit(String),
    #[error("Audio capture stream failed: {0}")]
    StreamError(String),
}

pub trait AudioCapture: Send {
    fn start(&mut self) -> Result<(), AudioError>;
    fn stop(&mut self) -> Result<(), AudioError>;
}
