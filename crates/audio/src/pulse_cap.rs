use std::ffi::{CStr, CString};
use std::ptr;

use tracing::info;

use crate::ffi::*;
use crate::AudioError;

pub struct PulseAudioCapture {
    simple: *mut PaSimple,
    buffer_bytes: usize,
}

unsafe impl Send for PulseAudioCapture {}

impl PulseAudioCapture {
    pub fn new() -> Result<Self, AudioError> {
        let ss = PaSampleSpec {
            format: PA_SAMPLE_S16LE,
            rate: 48000,
            channels: 2,
        };

        let app_name = CString::new("gamestream-audio").unwrap();
        let stream_name = CString::new("Audio Monitor").unwrap();

        let mut error = 0;
        let simple = unsafe {
            pa_simple_new(
                ptr::null(),
                app_name.as_ptr(),
                PA_STREAM_RECORD,
                ptr::null(), // default sink monitor
                stream_name.as_ptr(),
                &ss,
                ptr::null(),
                ptr::null(),
                &mut error,
            )
        };

        if simple.is_null() {
            let err_str = unsafe {
                let s = pa_strerror(error);
                CStr::from_ptr(s).to_string_lossy().into_owned()
            };
            return Err(AudioError::DeviceInit(format!("Fallo al conectar a PulseAudio/PipeWire: {err_str}")));
        }

        // 10 ms a 48 kHz estéreo 16-bit = 480 muestras * 2 canales * 2 bytes = 1920 bytes
        let buffer_bytes = 1920;

        info!("Captura de audio PulseAudio/PipeWire inicializada exitosamente (48 kHz estéreo S16LE, 10 ms por bloque)");

        Ok(Self {
            simple,
            buffer_bytes,
        })
    }

    /// Lee un bloque de 10 ms de audio PCM (480 muestras estéreo = 960 enteros i16)
    pub fn read_10ms_frame(&mut self) -> Result<Vec<i16>, AudioError> {
        let sample_count = self.buffer_bytes / 2;
        let mut samples = vec![0i16; sample_count];
        let mut error = 0;

        let ret = unsafe {
            pa_simple_read(
                self.simple,
                samples.as_mut_ptr() as *mut _,
                self.buffer_bytes,
                &mut error,
            )
        };

        if ret < 0 {
            let err_str = unsafe {
                let s = pa_strerror(error);
                CStr::from_ptr(s).to_string_lossy().into_owned()
            };
            return Err(AudioError::StreamError(format!("Error al leer stream de audio: {err_str}")));
        }

        Ok(samples)
    }
}

impl Drop for PulseAudioCapture {
    fn drop(&mut self) {
        if !self.simple.is_null() {
            unsafe {
                pa_simple_free(self.simple);
            }
            self.simple = ptr::null_mut();
        }
    }
}
