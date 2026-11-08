use std::time::Instant;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum CommonError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Invalid configuration: {0}")]
    Config(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelFormat {
    Bgra8,
    Rgba8,
    Nv12,
    I420,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Resolution {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
}

impl Resolution {
    pub const fn new(width: u32, height: u32, fps: u32) -> Self {
        Self { width, height, fps }
    }

    pub fn pixel_count(&self) -> u64 {
        self.width as u64 * self.height as u64
    }
}

#[derive(Debug, Clone)]
pub struct FrameInfo {
    pub sequence: u64,
    pub timestamp: Instant,
    pub width: u32,
    pub height: u32,
    pub stride: usize,
    pub format: PixelFormat,
}

pub struct VideoFrame {
    pub info: FrameInfo,
    pub data: Vec<u8>,
}

impl VideoFrame {
    pub fn new(info: FrameInfo, data: Vec<u8>) -> Self {
        Self { info, data }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolution_pixel_count() {
        let res = Resolution::new(1280, 720, 60);
        assert_eq!(res.pixel_count(), 921_600);
        assert_eq!(res.fps, 60);

        let sub_hd = Resolution::new(1024, 576, 60);
        assert_eq!(sub_hd.pixel_count(), 589_824);
    }

    #[test]
    fn test_video_frame_creation() {
        let info = FrameInfo {
            sequence: 42,
            timestamp: Instant::now(),
            width: 1280,
            height: 720,
            stride: 1280 * 4,
            format: PixelFormat::Bgra8,
        };
        let frame = VideoFrame::new(info, vec![0u8; 100]);
        assert_eq!(frame.info.sequence, 42);
        assert_eq!(frame.info.format, PixelFormat::Bgra8);
        assert_eq!(frame.data.len(), 100);
    }
}
