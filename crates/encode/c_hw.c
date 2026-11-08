#include <libavcodec/avcodec.h>
#include <libavutil/hwcontext.h>
#include <libavutil/opt.h>
#include <libavutil/pixdesc.h>

int gs_init_vaapi_encoder(
    AVCodecContext *ctx,
    int width,
    int height,
    AVBufferRef **out_device,
    AVBufferRef **out_frames
) {
    AVBufferRef *hw_device_ctx = NULL;
    int ret = av_hwdevice_ctx_create(&hw_device_ctx, AV_HWDEVICE_TYPE_VAAPI, "/dev/dri/renderD128", NULL, 0);
    if (ret < 0) {
        return ret;
    }

    AVBufferRef *hw_frames_ref = av_hwframe_ctx_alloc(hw_device_ctx);
    if (!hw_frames_ref) {
        av_buffer_unref(&hw_device_ctx);
        return -1;
    }

    AVHWFramesContext *frames_ctx = (AVHWFramesContext *)(hw_frames_ref->data);
    frames_ctx->format = AV_PIX_FMT_VAAPI;
    frames_ctx->sw_format = AV_PIX_FMT_NV12;
    frames_ctx->width = width;
    frames_ctx->height = height;
    frames_ctx->initial_pool_size = 20;

    ret = av_hwframe_ctx_init(hw_frames_ref);
    if (ret < 0) {
        av_buffer_unref(&hw_frames_ref);
        av_buffer_unref(&hw_device_ctx);
        return ret;
    }

    ctx->hw_frames_ctx = av_buffer_ref(hw_frames_ref);
    *out_device = hw_device_ctx;
    *out_frames = hw_frames_ref;
    return 0;
}

int gs_alloc_vaapi_frame(AVBufferRef *hw_frames_ref, AVFrame **out_frame) {
    AVFrame *frame = av_frame_alloc();
    if (!frame) return -1;
    int ret = av_hwframe_get_buffer(hw_frames_ref, frame, 0);
    if (ret < 0) {
        av_frame_free(&frame);
        return ret;
    }
    *out_frame = frame;
    return 0;
}
