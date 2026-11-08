pub mod adaptive;
pub mod crypto;
pub mod fragment;
pub mod protocol;
pub mod stun;
pub mod transceiver;

use thiserror::Error;

#[derive(Error, Debug)]
pub enum NetError {
    #[error("Connection failed: {0}")]
    Connection(String),
    #[error("Data transmission failed: {0}")]
    Transmission(String),
    #[error("Decryption failed: {0}")]
    Decryption(String),
}
