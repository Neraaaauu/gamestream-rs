use std::ffi::{c_char, c_int, c_void};

pub const AV_PIX_FMT_YUV420P: c_int = 0;
pub const AV_PIX_FMT_NV12: c_int = 23;
pub const SWS_FAST_BILINEAR: c_int = 1;
pub const SWS_BILINEAR: c_int = 2;

pub const AVERROR_EOF: c_int = -('E' as c_int | ('O' as c_int) << 8 | ('F' as c_int) << 16 | (' ' as c_int) << 24);
pub const AVERROR_EAGAIN: c_int = -libc::EAGAIN;

#[repr(C)]
pub struct AVCodec {
    _private: [u8; 0],
}

#[repr(C)]
pub struct AVDictionary {
    _private: [u8; 0],
}

#[repr(C)]
pub struct AVBufferRef {
    pub data: *mut u8,
    pub size: usize,
}

#[repr(C)]
pub struct AVHWFramesContext {
    pub av_class: *const c_void,
    pub internal: *mut c_void,
    pub device_ref: *mut AVBufferRef,
    pub device_ctx: *mut c_void,
    pub free: *mut c_void,
    pub user_opaque: *mut c_void,
    pub pool: *mut c_void,
    pub initial_pool_size: c_int,
    pub format: c_int,
    pub sw_format: c_int,
    pub width: c_int,
    pub height: c_int,
}

extern "C" {
    pub fn gs_init_vaapi_encoder(
        ctx: *mut AVCodecContext,
        width: c_int,
        height: c_int,
        out_device: *mut *mut AVBufferRef,
        out_frames: *mut *mut AVBufferRef,
    ) -> c_int;

    pub fn gs_alloc_vaapi_frame(
        hw_frames_ref: *mut AVBufferRef,
        out_frame: *mut *mut AVFrame,
    ) -> c_int;

    pub fn av_hwframe_transfer_data(
        dst: *mut AVFrame,
        src: *const AVFrame,
        flags: c_int,
    ) -> c_int;
}

#[repr(C)]
pub struct SwsContext {
    _private: [u8; 0],
}

#[repr(C)]
pub struct AVRational {
    pub num: c_int,
    pub den: c_int,
}

#[repr(C)]
pub struct AVPacket {
    pub buf: *mut AVBufferRef,
    pub pts: i64,
    pub dts: i64,
    pub data: *mut u8,
    pub size: c_int,
    pub stream_index: c_int,
    pub flags: c_int,
    pub side_data: *mut c_void,
    pub side_data_elems: c_int,
    pub duration: i64,
    pub pos: i64,
    pub opaque: *mut c_void,
    pub opaque_ref: *mut AVBufferRef,
    pub time_base: AVRational,
}

#[repr(C)]
pub struct AVFrame {
    pub data: [*mut u8; 8],
    pub linesize: [c_int; 8],
    pub extended_data: *mut *mut u8,
    pub width: c_int,
    pub height: c_int,
    pub nb_samples: c_int,
    pub format: c_int,
    pub key_frame: c_int,
    pub pict_type: c_int,
    pub sample_aspect_ratio: AVRational,
    pub pts: i64,
    pub pkt_dts: i64,
    pub time_base: AVRational,
    pub quality: c_int,
    pub opaque: *mut c_void,
    // Other fields can follow
    _padding: [u8; 512],
}

type CUint = u32;

#[repr(C)]
pub struct AVCodecContext {
    pub av_class: *const c_void,
    pub log_level_offset: c_int,
    pub codec_type: c_int,
    pub codec: *const AVCodec,
    pub codec_id: c_int,
    pub codec_tag: CUint,
    pub priv_data: *mut c_void,
    pub internal: *mut c_void,
    pub opaque: *mut c_void,
    pub bit_rate: i64,
    pub bit_rate_tolerance: c_int,
    pub global_quality: c_int,
    pub compression_level: c_int,
    pub flags: c_int,
    pub flags2: c_int,
    pub extradata: *mut u8,
    pub extradata_size: c_int,
    pub time_base: AVRational,
    pub ticks_per_frame: c_int,
    pub delay: c_int,
    pub width: c_int,
    pub height: c_int,
    pub coded_width: c_int,
    pub coded_height: c_int,
    pub gop_size: c_int,
    pub pix_fmt: c_int,
    _padding: [u8; 1024],
}

impl AVCodecContext {
    /// # Safety
    /// Caller must ensure that `frames_ctx` is a valid pointer to `AVBufferRef`.
    pub unsafe fn set_hw_frames_ctx(&mut self, frames_ctx: *mut AVBufferRef) {
        let ptr = (self as *mut Self as *mut u8).add(840) as *mut *mut AVBufferRef;
        *ptr = frames_ctx;
    }

    /// # Safety
    /// Caller must ensure that `device_ctx` is a valid pointer to `AVBufferRef`.
    pub unsafe fn set_hw_device_ctx(&mut self, device_ctx: *mut AVBufferRef) {
        let ptr = (self as *mut Self as *mut u8).add(864) as *mut *mut AVBufferRef;
        *ptr = device_ctx;
    }
}

extern "C" {
    pub fn avcodec_find_encoder_by_name(name: *const c_char) -> *mut AVCodec;
    pub fn avcodec_alloc_context3(codec: *const AVCodec) -> *mut AVCodecContext;
    pub fn avcodec_free_context(avctx: *mut *mut AVCodecContext);
    pub fn avcodec_open2(avctx: *mut AVCodecContext, codec: *const AVCodec, options: *mut *mut AVDictionary) -> c_int;
    pub fn avcodec_send_frame(avctx: *mut AVCodecContext, frame: *const AVFrame) -> c_int;
    pub fn avcodec_receive_packet(avctx: *mut AVCodecContext, avpkt: *mut AVPacket) -> c_int;

    pub fn av_frame_alloc() -> *mut AVFrame;
    pub fn av_frame_free(frame: *mut *mut AVFrame);
    pub fn av_frame_get_buffer(frame: *mut AVFrame, align: c_int) -> c_int;
    pub fn av_frame_make_writable(frame: *mut AVFrame) -> c_int;

    pub fn av_packet_alloc() -> *mut AVPacket;
    pub fn av_packet_free(pkt: *mut *mut AVPacket);
    pub fn av_packet_unref(pkt: *mut AVPacket);

    pub fn av_opt_set(obj: *mut c_void, name: *const c_char, val: *const c_char, search_flags: c_int) -> c_int;
    pub fn av_opt_set_int(obj: *mut c_void, name: *const c_char, val: i64, search_flags: c_int) -> c_int;

    pub fn av_hwdevice_ctx_create(device_ctx: *mut *mut AVBufferRef, device_type: c_int, device: *const c_char, opts: *mut AVDictionary, flags: c_int) -> c_int;
    pub fn av_hwdevice_find_type_by_name(name: *const c_char) -> c_int;
    pub fn av_buffer_unref(buf: *mut *mut AVBufferRef);

    pub fn av_get_pix_fmt(name: *const c_char) -> c_int;
    pub fn av_strerror(errnum: c_int, errbuf: *mut c_char, errbuf_size: usize) -> c_int;

    pub fn sws_getContext(
        srcW: c_int,
        srcH: c_int,
        srcFormat: c_int,
        dstW: c_int,
        dstH: c_int,
        dstFormat: c_int,
        flags: c_int,
        srcFilter: *mut c_void,
        dstFilter: *mut c_void,
        param: *const f64,
    ) -> *mut SwsContext;

    pub fn sws_scale(
        c: *mut SwsContext,
        srcSlice: *const *const u8,
        srcStride: *const c_int,
        srcSliceY: c_int,
        srcSliceH: c_int,
        dst: *const *mut u8,
        dstStride: *const c_int,
    ) -> c_int;

    pub fn sws_freeContext(swsContext: *mut SwsContext);
}

pub fn get_error_string(errnum: c_int) -> String {
    let mut buf = [0u8; 128];
    unsafe {
        av_strerror(errnum, buf.as_mut_ptr() as *mut c_char, buf.len());
    }
    let cstr = unsafe { std::ffi::CStr::from_ptr(buf.as_ptr() as *const c_char) };
    cstr.to_string_lossy().into_owned()
}
