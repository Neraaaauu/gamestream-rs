use std::ffi::{c_char, c_int, c_uchar, c_uint, c_void};

pub const PA_SAMPLE_S16LE: c_int = 3;
pub const PA_STREAM_RECORD: c_int = 2;

pub const OPUS_APPLICATION_RESTRICTED_LOWDELAY: c_int = 2051;
pub const OPUS_SET_BITRATE_REQUEST: c_int = 4002;
pub const OPUS_SET_COMPLEXITY_REQUEST: c_int = 4010;
pub const OPUS_SET_INBAND_FEC_REQUEST: c_int = 4012;
pub const OPUS_SET_DTX_REQUEST: c_int = 4016;

#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct PaSampleSpec {
    pub format: c_int,
    pub rate: c_uint,
    pub channels: c_uchar,
}

#[repr(C)]
pub struct PaSimple {
    _private: [u8; 0],
}

#[repr(C)]
pub struct OpusEncoder {
    _private: [u8; 0],
}

extern "C" {
    // PulseAudio Simple API
    pub fn pa_simple_new(
        server: *const c_char,
        name: *const c_char,
        dir: c_int,
        dev: *const c_char,
        stream_name: *const c_char,
        ss: *const PaSampleSpec,
        map: *const c_void,
        attr: *const c_void,
        error: *mut c_int,
    ) -> *mut PaSimple;

    pub fn pa_simple_read(
        s: *mut PaSimple,
        data: *mut c_void,
        bytes: usize,
        error: *mut c_int,
    ) -> c_int;

    pub fn pa_simple_free(s: *mut PaSimple);
    pub fn pa_strerror(error: c_int) -> *const c_char;

    // Opus API
    pub fn opus_encoder_create(
        fs: c_int,
        channels: c_int,
        application: c_int,
        error: *mut c_int,
    ) -> *mut OpusEncoder;

    pub fn opus_encoder_destroy(st: *mut OpusEncoder);

    pub fn opus_encode(
        st: *mut OpusEncoder,
        pcm: *const i16,
        frame_size: c_int,
        data: *mut u8,
        max_data_bytes: i32,
    ) -> i32;

    pub fn opus_encoder_ctl(st: *mut OpusEncoder, request: c_int, ...) -> c_int;
    pub fn opus_strerror(error: c_int) -> *const c_char;
}
