use std::ffi::CStr;
use std::ptr;

use tracing::info;

use crate::ffi::*;
use crate::AudioError;

pub struct OpusAudioEncoder {
    encoder: *mut OpusEncoder,
    frame_size_samples: i32,
    channels: i32,
}

unsafe impl Send for OpusAudioEncoder {}

impl OpusAudioEncoder {
    pub fn new(sample_rate: i32, channels: i32, bitrate_bps: i32) -> Result<Self, AudioError> {
        let mut error = 0;
        let enc = unsafe {
            opus_encoder_create(
                sample_rate,
                channels,
                OPUS_APPLICATION_RESTRICTED_LOWDELAY,
                &mut error,
            )
        };

        if enc.is_null() || error != 0 {
            let err_str = unsafe {
                let s = opus_strerror(error);
                CStr::from_ptr(s).to_string_lossy().into_owned()
            };
            return Err(AudioError::DeviceInit(format!("Fallo al crear encoder Opus: {err_str}")));
        }

        unsafe {
            // Configurar Bitrate (96 kbps)
            opus_encoder_ctl(enc, OPUS_SET_BITRATE_REQUEST, bitrate_bps);
            // Activar In-band FEC para recuperar paquetes perdidos sin retardo
            opus_encoder_ctl(enc, OPUS_SET_INBAND_FEC_REQUEST, 1);
            // Desactivar DTX para evitar clicks y pausas en audio de juegos
            opus_encoder_ctl(enc, OPUS_SET_DTX_REQUEST, 0);
            // Complejidad equilibrada para CPUs modestas (escala 1-10)
            opus_encoder_ctl(enc, OPUS_SET_COMPLEXITY_REQUEST, 5);
        }

        // 10 ms a 48 kHz = 480 muestras por canal
        let frame_size_samples = sample_rate / 100;

        info!(
            "Encoder Opus inicializado (Rate: {} Hz, Canales: {}, Bitrate: {} kbps, Frame: 10 ms / {} samples)",
            sample_rate, channels, bitrate_bps / 1000, frame_size_samples
        );

        Ok(Self {
            encoder: enc,
            frame_size_samples,
            channels,
        })
    }

    /// Codifica 10 ms de audio PCM i16 interleaved a formato Opus
    pub fn encode(&mut self, pcm_samples: &[i16]) -> Result<Vec<u8>, AudioError> {
        let expected_samples = (self.frame_size_samples * self.channels) as usize;
        if pcm_samples.len() < expected_samples {
            return Err(AudioError::StreamError(format!(
                "Buffer de audio insuficiente: esperado {}, recibido {}",
                expected_samples,
                pcm_samples.len()
            )));
        }

        let mut output = vec![0u8; 1000];
        let bytes_written = unsafe {
            opus_encode(
                self.encoder,
                pcm_samples.as_ptr(),
                self.frame_size_samples,
                output.as_mut_ptr(),
                output.len() as i32,
            )
        };

        if bytes_written < 0 {
            let err_str = unsafe {
                let s = opus_strerror(bytes_written);
                CStr::from_ptr(s).to_string_lossy().into_owned()
            };
            return Err(AudioError::StreamError(format!("Error en opus_encode: {err_str}")));
        }

        output.truncate(bytes_written as usize);
        Ok(output)
    }
}

impl Drop for OpusAudioEncoder {
    fn drop(&mut self) {
        if !self.encoder.is_null() {
            unsafe {
                opus_encoder_destroy(self.encoder);
            }
            self.encoder = ptr::null_mut();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    #[test]
    fn test_opus_encode_10ms_sine() {
        let mut encoder = OpusAudioEncoder::new(48000, 2, 96000).expect("Fallo al crear encoder Opus");
        // 480 muestras * 2 canales = 960 enteros i16
        let mut pcm = vec![0i16; 960];
        for i in 0..480 {
            let sample = ((i as f64 * 440.0 * 2.0 * std::f64::consts::PI / 48000.0).sin() * 16000.0) as i16;
            pcm[i * 2] = sample;
            pcm[i * 2 + 1] = sample;
        }

        let start = Instant::now();
        let opus_data = encoder.encode(&pcm).expect("Fallo al codificar audio Opus");
        let elapsed = start.elapsed();

        assert!(!opus_data.is_empty());
        assert!(opus_data.len() < 300);
        println!("Opus 10ms frame encoded in {:.3} ms (size: {} bytes)", elapsed.as_secs_f64() * 1000.0, opus_data.len());
        assert!(elapsed.as_millis() < 5, "Encode de 10ms debe tomar menos de 5ms");
    }
}
