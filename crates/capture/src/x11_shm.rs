use std::ptr;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use gamestream_common::{FrameInfo, PixelFormat, VideoFrame};
use x11rb::connection::Connection;
use x11rb::protocol::shm::ConnectionExt as ShmConnectionExt;
use x11rb::protocol::xproto::ImageFormat;
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as WrapperConnectionExt;

use crate::{CaptureError, ScreenCapture};

pub struct X11ShmCapture {
    conn: RustConnection,
    root: u32,
    width: u16,
    height: u16,
    _depth: u8,
    shm_seg: u32,
    shm_id: i32,
    shm_addr: *mut u8,
    shm_size: usize,
    sequence: AtomicU64,
}

// Implement Send safely because RustConnection and the shm pointer are managed exclusively by this struct
unsafe impl Send for X11ShmCapture {}

impl X11ShmCapture {
    pub fn new() -> Result<Self, CaptureError> {
        let (conn, screen_num) = x11rb::connect(None)
            .map_err(|e| CaptureError::X11(format!("Failed to connect to X11 display: {e}")))?;

        // Verify SHM extension
        let shm_version = conn
            .shm_query_version()
            .map_err(|e| CaptureError::X11(format!("Failed to query SHM extension: {e}")))?
            .reply()
            .map_err(|_| CaptureError::ShmNotSupported)?;

        tracing::info!(
            "X11 SHM extension available (version {}.{})",
            shm_version.major_version,
            shm_version.minor_version
        );

        let setup = conn.setup();
        let screen = setup
            .roots
            .get(screen_num)
            .ok_or_else(|| CaptureError::X11(format!("Screen index {screen_num} not found")))?;

        let root = screen.root;
        let width = screen.width_in_pixels;
        let height = screen.height_in_pixels;
        let depth = screen.root_depth;

        // Calculate buffer size (32 bits / 4 bytes per pixel for typical 24/32 depth)
        let bytes_per_pixel = 4usize;
        let shm_size = (width as usize) * (height as usize) * bytes_per_pixel;

        tracing::info!(
            "X11 Display detected: {}x{}, depth {}, buffer size {} bytes ({:.2} MB)",
            width,
            height,
            depth,
            shm_size,
            shm_size as f64 / (1024.0 * 1024.0)
        );

        // Allocate SysV shared memory
        let shm_id = unsafe {
            libc::shmget(
                libc::IPC_PRIVATE,
                shm_size,
                libc::IPC_CREAT | 0o777,
            )
        };
        if shm_id < 0 {
            let err = std::io::Error::last_os_error();
            return Err(CaptureError::SystemMemory(format!("shmget failed: {err}")));
        }

        let shm_addr = unsafe { libc::shmat(shm_id, ptr::null(), 0) as *mut u8 };
        if shm_addr == usize::MAX as *mut u8 || shm_addr.is_null() {
            let err = std::io::Error::last_os_error();
            unsafe {
                libc::shmctl(shm_id, libc::IPC_RMID, ptr::null_mut());
            }
            return Err(CaptureError::SystemMemory(format!("shmat failed: {err}")));
        }

        // Generate XID for SHM segment
        let shm_seg = conn
            .generate_id()
            .map_err(|e| CaptureError::X11(format!("generate_id failed: {e}")))?;

        // Attach to X Server
        conn.shm_attach(shm_seg, shm_id as u32, false)
            .map_err(|e| CaptureError::X11(format!("shm_attach failed: {e}")))?;

        // Sync connection to make sure the attach request was processed
        conn.sync()
            .map_err(|e| CaptureError::X11(format!("X11 sync failed after shm_attach: {e}")))?;

        // Now mark shm for deletion so it gets reclaimed automatically when process exits
        unsafe {
            libc::shmctl(shm_id, libc::IPC_RMID, ptr::null_mut());
        }

        Ok(Self {
            conn,
            root,
            width,
            height,
            _depth: depth,
            shm_seg,
            shm_id,
            shm_addr,
            shm_size,
            sequence: AtomicU64::new(0),
        })
    }
}

impl ScreenCapture for X11ShmCapture {
    fn resolution(&self) -> (u32, u32) {
        (self.width as u32, self.height as u32)
    }

    fn capture_frame(&mut self) -> Result<VideoFrame, CaptureError> {
        let start_time = Instant::now();

        // shm_get_image: captures the screen directly into our attached shared memory buffer
        let reply = self
            .conn
            .shm_get_image(
                self.root,
                0,
                0,
                self.width,
                self.height,
                !0, // all planes
                ImageFormat::Z_PIXMAP.into(),
                self.shm_seg,
                0, // offset
            )
            .map_err(|e| CaptureError::CaptureFailed(format!("shm_get_image failed: {e}")))?;

        // Wait for X server to finish writing into the SHM segment
        reply
            .reply()
            .map_err(|e| CaptureError::CaptureFailed(format!("shm_get_image reply failed: {e}")))?;

        let seq = self.sequence.fetch_add(1, Ordering::Relaxed);
        let stride = (self.width as usize) * 4;

        // Copy captured bytes from the shared memory page to a frame buffer
        // Note: in a zero-copy pipeline this pointer will be wrapped directly or passed to VA-API
        let mut data = vec![0u8; self.shm_size];
        unsafe {
            ptr::copy_nonoverlapping(self.shm_addr, data.as_mut_ptr(), self.shm_size);
        }

        let info = FrameInfo {
            sequence: seq,
            timestamp: start_time,
            width: self.width as u32,
            height: self.height as u32,
            stride,
            format: PixelFormat::Bgra8,
        };

        Ok(VideoFrame::new(info, data))
    }
}

impl Drop for X11ShmCapture {
    fn drop(&mut self) {
        tracing::debug!("Cleaning up X11 SHM resources (seg: {}, id: {})", self.shm_seg, self.shm_id);
        let _ = self.conn.shm_detach(self.shm_seg);
        let _ = self.conn.sync();
        if !self.shm_addr.is_null() && self.shm_addr != usize::MAX as *mut u8 {
            unsafe {
                libc::shmdt(self.shm_addr as *mut libc::c_void);
            }
        }
    }
}
