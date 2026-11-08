use thiserror::Error;

#[derive(Error, Debug)]
pub enum NetError {
    #[error("Connection failed: {0}")]
    Connection(String),
    #[error("Data transmission failed: {0}")]
    Transmission(String),
}
