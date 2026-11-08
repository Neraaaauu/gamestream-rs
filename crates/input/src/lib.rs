pub mod protocol;

#[cfg(target_os = "linux")]
pub mod injector;

use thiserror::Error;

#[derive(Error, Debug)]
pub enum InputError {
    #[error("Failed to open uinput device: {0}")]
    UinputInit(String),
    #[error("Failed to inject input event: {0}")]
    EmitFailed(String),
}
