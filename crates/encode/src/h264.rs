use std::ffi::CString;
use std::ptr;

use gamestream_common::VideoFrame;
use tracing::{info, warn};

use crate::ffi::*;
use crate::{EncodeError, VideoEncoder};

pub struct H264Encoder {
    codec_ctx: *mut AVCodecContext,
    encoder_name: String,
    is_hw: bool,
    sws_ctx: *mut SwsContext,
    sw_frame: *mut AVFrame,
    hw_frame: *mut AVFrame,
    av_packet: *mut AVPacket,
    hw_device_ref: *mut AVBufferRef,
    hw_frames_ref: *mut AVBufferRef,
    src_width: u32,
    src_height: u32,
    dst_width: u32,
    dst_height: u32,
    pts_counter: i64,
    force_idr_next: bool,
}

unsafe impl Send for H264Encoder {}

impl H264Encoder {
    pub fn new(
        src_width: u32,
        src_height: u32,
        dst_width: u32,
        dst_height: u32,
        fps: u32,
        bitrate_bps: i64,
    ) -> Result<Self, EncodeError> {
        info!(
            "Inicializando pipeline H.264 (Entrada: {}x{}, Salida: {}x{} @ {} FPS, Bitrate: {} kbps)",
            src_width, src_height, dst_width, dst_height, fps, bitrate_bps / 1000
        );

        let bgra_fmt = unsafe { av_get_pix_fmt(c"bgra".as_ptr()) };
        let yuv420p_fmt = AV_PIX_FMT_YUV420P;
        let nv12_fmt = AV_PIX_FMT_NV12;

        let mut active_codec_ctx = ptr::null_mut();
        let mut chosen_name = String::new();
        let mut is_hw = false;
        let mut active_hw_device = ptr::null_mut();
        let mut active_hw_frames = ptr::null_mut();

        // 1. Intentar VA-API por hardware en Linux
        if std::path::Path::new("/dev/dri/renderD128").exists() {
            let vaapi_codec_name = CString::new("h264_vaapi").unwrap();
            let codec = unsafe { avcodec_find_encoder_by_name(vaapi_codec_name.as_ptr()) };
            if !codec.is_null() {
                let codec_ctx = unsafe { avcodec_alloc_context3(codec) };
                if !codec_ctx.is_null() {
                    let vaapi_pix_fmt = unsafe { av_get_pix_fmt(c"vaapi".as_ptr()) };
                    unsafe {
                        (*codec_ctx).width = dst_width as i32;
                        (*codec_ctx).height = dst_height as i32;
                        (*codec_ctx).time_base = AVRational { num: 1, den: fps as i32 };
                        (*codec_ctx).pix_fmt = vaapi_pix_fmt;
                        (*codec_ctx).bit_rate = bitrate_bps;
                        (*codec_ctx).gop_size = 600;

                        let opt_ctx = codec_ctx as *mut std::ffi::c_void;
                        let b_key = CString::new("bf").unwrap();
                        av_opt_set_int(opt_ctx, b_key.as_ptr(), 0, 0); // Cero B-frames

                        let mut hw_device = ptr::null_mut();
                        let mut hw_frames = ptr::null_mut();
                        let ret_hw = gs_init_vaapi_encoder(
                            codec_ctx,
                            dst_width as i32,
                            dst_height as i32,
                            &mut hw_device,
                            &mut hw_frames,
                        );

                        if ret_hw == 0 {
                            let ret_open = avcodec_open2(codec_ctx, codec, ptr::null_mut());
                            if ret_open == 0 {
                                info!("Encoder de HARDWARE VA-API (h264_vaapi) inicializado y verificado con éxito.");
                                active_codec_ctx = codec_ctx;
                                chosen_name = "h264_vaapi".to_string();
                                is_hw = true;
                                active_hw_device = hw_device;
                                active_hw_frames = hw_frames;
                            } else {
                                warn!("avcodec_open2 falló para h264_vaapi: {}", get_error_string(ret_open));
                                avcodec_free_context(&mut (codec_ctx as *mut _));
                                let mut f_ptr = hw_frames;
                                av_buffer_unref(&mut f_ptr);
                                let mut d_ptr = hw_device;
                                av_buffer_unref(&mut d_ptr);
                            }
                        } else {
                            warn!("gs_init_vaapi_encoder falló ({}): probando fallback...", get_error_string(ret_hw));
                            avcodec_free_context(&mut (codec_ctx as *mut _));
                        }
                    }
                }
            }
        }

        // 2. Fallback a libx264 (Software universal optimizado)
        if active_codec_ctx.is_null() {
            active_hw_device = ptr::null_mut();
            active_hw_frames = ptr::null_mut();

            info!("Iniciando encoder universal de software 'libx264' (preset=ultrafast, tune=zerolatency)");
            let c_name = CString::new("libx264").unwrap();
            let codec = unsafe { avcodec_find_encoder_by_name(c_name.as_ptr()) };
            if codec.is_null() {
                return Err(EncodeError::NoEncoderAvailable);
            }

            let codec_ctx = unsafe { avcodec_alloc_context3(codec) };
            if codec_ctx.is_null() {
                return Err(EncodeError::InitFailed("avcodec_alloc_context3 falló".into()));
            }

            unsafe {
                (*codec_ctx).width = dst_width as i32;
                (*codec_ctx).height = dst_height as i32;
                (*codec_ctx).time_base = AVRational { num: 1, den: fps as i32 };
                (*codec_ctx).pix_fmt = yuv420p_fmt;
                (*codec_ctx).bit_rate = bitrate_bps;
                (*codec_ctx).gop_size = 600;

                let opt_ctx = codec_ctx as *mut std::ffi::c_void;
                let b_key = CString::new("bf").unwrap();
                av_opt_set_int(opt_ctx, b_key.as_ptr(), 0, 0);

                let preset_key = CString::new("preset").unwrap();
                let preset_val = CString::new("ultrafast").unwrap();
                av_opt_set((*codec_ctx).priv_data, preset_key.as_ptr(), preset_val.as_ptr(), 0);

                let tune_key = CString::new("tune").unwrap();
                let tune_val = CString::new("zerolatency").unwrap();
                av_opt_set((*codec_ctx).priv_data, tune_key.as_ptr(), tune_val.as_ptr(), 0);

                let profile_key = CString::new("profile").unwrap();
                let profile_val = CString::new("baseline").unwrap();
                av_opt_set((*codec_ctx).priv_data, profile_key.as_ptr(), profile_val.as_ptr(), 0);

                let vbv_buf = bitrate_bps / (fps as i64);
                let bufsize_key = CString::new("bufsize").unwrap();
                av_opt_set_int(opt_ctx, bufsize_key.as_ptr(), vbv_buf, 0);

                let maxrate_key = CString::new("maxrate").unwrap();
                av_opt_set_int(opt_ctx, maxrate_key.as_ptr(), bitrate_bps, 0);

                let ret = avcodec_open2(codec_ctx, codec, ptr::null_mut());
                if ret < 0 {
                    let err = get_error_string(ret);
                    avcodec_free_context(&mut (codec_ctx as *mut _));
                    return Err(EncodeError::InitFailed(format!("avcodec_open2 falló para libx264: {err}")));
                }

                active_codec_ctx = codec_ctx;
                chosen_name = "libx264".to_string();
                is_hw = false;
            }
        }

        let codec_ctx = active_codec_ctx;

        // 3. Crear SwsContext para escalado y conversión de color
        let target_sw_fmt = if is_hw { nv12_fmt } else { yuv420p_fmt };
        let sws_ctx = unsafe {
            sws_getContext(
                src_width as i32,
                src_height as i32,
                bgra_fmt,
                dst_width as i32,
                dst_height as i32,
                target_sw_fmt,
                SWS_FAST_BILINEAR,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null(),
            )
        };
        if sws_ctx.is_null() {
            unsafe { avcodec_free_context(&mut (codec_ctx as *mut _)) };
            return Err(EncodeError::ScalerFailed("sws_getContext falló".into()));
        }

        // 4. Alojar AVFrame para datos SW
        let sw_frame = unsafe { av_frame_alloc() };
        if sw_frame.is_null() {
            unsafe {
                sws_freeContext(sws_ctx);
                avcodec_free_context(&mut (codec_ctx as *mut _));
            }
            return Err(EncodeError::InitFailed("av_frame_alloc falló".into()));
        }

        unsafe {
            (*sw_frame).width = dst_width as i32;
            (*sw_frame).height = dst_height as i32;
            (*sw_frame).format = target_sw_fmt;
            let ret = av_frame_get_buffer(sw_frame, 32);
            if ret < 0 {
                let err_str = get_error_string(ret);
                av_frame_free(&mut (sw_frame as *mut _));
                sws_freeContext(sws_ctx);
                avcodec_free_context(&mut (codec_ctx as *mut _));
                return Err(EncodeError::InitFailed(format!("av_frame_get_buffer falló: {err_str}")));
            }
        }

        // 5. Si es HW, alojar frame en GPU desde el pool de hardware frames
        let mut hw_frame = ptr::null_mut();
        if is_hw && !active_hw_frames.is_null() {
            unsafe {
                let ret = gs_alloc_vaapi_frame(active_hw_frames, &mut hw_frame);
                if ret < 0 {
                    warn!("gs_alloc_vaapi_frame falló: {}", get_error_string(ret));
                }
            }
        }

        // 6. Alojar AVPacket
        let av_packet = unsafe { av_packet_alloc() };
        if av_packet.is_null() {
            unsafe {
                av_frame_free(&mut (sw_frame as *mut _));
                if !hw_frame.is_null() {
                    av_frame_free(&mut (hw_frame as *mut _));
                }
                sws_freeContext(sws_ctx);
                avcodec_free_context(&mut (codec_ctx as *mut _));
            }
            return Err(EncodeError::InitFailed("av_packet_alloc falló".into()));
        }

        info!("Encoder H.264 '{}' listo para transmisión.", chosen_name);

        Ok(Self {
            codec_ctx,
            encoder_name: chosen_name,
            is_hw,
            sws_ctx,
            sw_frame,
            hw_frame,
            av_packet,
            hw_device_ref: active_hw_device,
            hw_frames_ref: active_hw_frames,
            src_width,
            src_height,
            dst_width,
            dst_height,
            pts_counter: 0,
            force_idr_next: true,
        })
    }

    pub fn input_resolution(&self) -> (u32, u32) {
        (self.src_width, self.src_height)
    }

    pub fn output_resolution(&self) -> (u32, u32) {
        (self.dst_width, self.dst_height)
    }
}

impl VideoEncoder for H264Encoder {
    fn name(&self) -> &str {
        &self.encoder_name
    }

    fn is_hardware_accelerated(&self) -> bool {
        self.is_hw
    }

    fn force_idr(&mut self) {
        self.force_idr_next = true;
    }

    fn encode(&mut self, frame: &VideoFrame) -> Result<Vec<u8>, EncodeError> {
        let mut encoded_data = Vec::with_capacity(32 * 1024);

        unsafe {
            let ret = av_frame_make_writable(self.sw_frame);
            if ret < 0 {
                return Err(EncodeError::SendFrameFailed(format!(
                    "av_frame_make_writable falló: {}",
                    get_error_string(ret)
                )));
            }

            // Convertir y reescalar BGRA8 a YUV420P / NV12
            let src_slice: [*const u8; 1] = [frame.data.as_ptr()];
            let src_stride: [i32; 1] = [frame.info.stride as i32];

            sws_scale(
                self.sws_ctx,
                src_slice.as_ptr(),
                src_stride.as_ptr(),
                0,
                self.src_height as i32,
                (*self.sw_frame).data.as_ptr(),
                (*self.sw_frame).linesize.as_ptr(),
            );

            (*self.sw_frame).pts = self.pts_counter;

            if self.force_idr_next {
                (*self.sw_frame).pict_type = 1; // AV_PICTURE_TYPE_I
                self.force_idr_next = false;
            } else {
                (*self.sw_frame).pict_type = 0;
            }

            // Si es hardware, transferir datos al surface de GPU
            let frame_to_send = if self.is_hw && !self.hw_frame.is_null() {
                let ret_tr = av_hwframe_transfer_data(self.hw_frame, self.sw_frame, 0);
                if ret_tr < 0 {
                    return Err(EncodeError::SendFrameFailed(format!(
                        "av_hwframe_transfer_data falló: {}",
                        get_error_string(ret_tr)
                    )));
                }
                (*self.hw_frame).pts = self.pts_counter;
                self.hw_frame
            } else {
                self.sw_frame
            };

            self.pts_counter += 1;

            let ret = avcodec_send_frame(self.codec_ctx, frame_to_send);
            if ret < 0 {
                return Err(EncodeError::SendFrameFailed(format!(
                    "avcodec_send_frame falló: {}",
                    get_error_string(ret)
                )));
            }

            loop {
                let ret = avcodec_receive_packet(self.codec_ctx, self.av_packet);
                if ret == AVERROR_EAGAIN || ret == AVERROR_EOF {
                    break;
                }
                if ret < 0 {
                    return Err(EncodeError::ReceivePacketFailed(format!(
                        "avcodec_receive_packet falló: {}",
                        get_error_string(ret)
                    )));
                }

                let pkt_size = (*self.av_packet).size as usize;
                let pkt_data = (*self.av_packet).data;
                if !pkt_data.is_null() && pkt_size > 0 {
                    let slice = std::slice::from_raw_parts(pkt_data, pkt_size);
                    encoded_data.extend_from_slice(slice);
                }

                av_packet_unref(self.av_packet);
            }
        }

        Ok(encoded_data)
    }
}

impl Drop for H264Encoder {
    fn drop(&mut self) {
        unsafe {
            if !self.av_packet.is_null() {
                let mut pkt = self.av_packet;
                av_packet_free(&mut pkt);
                self.av_packet = ptr::null_mut();
            }
            if !self.hw_frame.is_null() {
                let mut f = self.hw_frame;
                av_frame_free(&mut f);
                self.hw_frame = ptr::null_mut();
            }
            if !self.sw_frame.is_null() {
                let mut f = self.sw_frame;
                av_frame_free(&mut f);
                self.sw_frame = ptr::null_mut();
            }
            if !self.sws_ctx.is_null() {
                sws_freeContext(self.sws_ctx);
                self.sws_ctx = ptr::null_mut();
            }
            if !self.hw_frames_ref.is_null() {
                let mut r = self.hw_frames_ref;
                av_buffer_unref(&mut r);
                self.hw_frames_ref = ptr::null_mut();
            }
            if !self.hw_device_ref.is_null() {
                let mut r = self.hw_device_ref;
                av_buffer_unref(&mut r);
                self.hw_device_ref = ptr::null_mut();
            }
            if !self.codec_ctx.is_null() {
                let mut c = self.codec_ctx;
                avcodec_free_context(&mut c);
                self.codec_ctx = ptr::null_mut();
            }
        }
    }
}
