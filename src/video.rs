//! RIIR: https://github.com/FFmpeg/FFmpeg/blob/master/doc/examples/hw_decode.c
//! HW-accelerated decoding example using rsmpeg
use crate::{
    app::{
        AppRunner, AudioFrame, CudaFrame, DecodedFrame, GluResource, GluResourceCtrl,
        GluResourceCtrlSync, GluResourceStat, GluResourceStatInfo, GluResourceState, UserEvent,
    },
    cuda::{
        CUDA_GRAPHIC_REGISTER_FLAG_SURFACE_LOAD_STORE, CUDA_MEMCPY_DEVICE_TO_DEVICE,
        NppStreamContext, NppiSize, cudaFree, cudaGraphicsD3D11RegisterResource,
        cudaGraphicsGLRegisterImage, cudaGraphicsMapResources,
        cudaGraphicsSubResourceGetMappedArray, cudaGraphicsUnmapResources,
        cudaGraphicsUnregisterResource, cudaMalloc, cudaMemcpy2D, cudaMemcpy2DToArray,
        cudaMemset2D, cudaStreamSynchronize, nppGetStreamContext, nppiNV12ToRGB_8u_P2C3R_Ctx,
    },
    cuda_check, cuda_error, elogger, npp_error,
    utils::{self, AUDIO_DEIVCE, AUDIO_SAMPLES, AudioCtrl, CHANNEL_SIZE},
};
use anyhow::{Context, Result, anyhow};
use bytes::{Bytes, BytesMut};
use rsmpeg::{
    UnsafeDerefMut,
    avcodec::AVCodecContext,
    avformat::AVFormatContextInput,
    avutil::{
        AVHWDeviceContext, AVHWDeviceType, AVPixelFormat, AVSamples, get_bytes_per_sample,
        hwdevice_find_type_by_name, hwdevice_get_type_name, hwdevice_iterate_types,
    },
    build_array,
    ffi::{
        self, AV_SAMPLE_FMT_DBLP, AV_SAMPLE_FMT_FLTP, AV_SAMPLE_FMT_NB, AV_SAMPLE_FMT_S16P,
        AV_SAMPLE_FMT_S32P, AV_SAMPLE_FMT_S64P, AV_SAMPLE_FMT_U8P, AVRational, AVSampleFormat,
    },
    swresample::SwrContext,
};
use std::{
    collections::HashMap,
    path::PathBuf,
    ptr::{null, null_mut},
    slice,
    sync::{Arc, Barrier, Mutex, RwLock, Weak, atomic::Ordering},
    time::Duration,
};
use std::{str::FromStr, sync::atomic::AtomicBool};
use winit::event_loop::EventLoop;

// static HW_PIX_FMT: OnceCell<ffi::AVPixelFormat> = OnceCell::new();

// Only macOS has videotoolbox in Github Actions
// #[test]
// #[cfg_attr(not(target_os = "macos"), ignore)]
// fn test_hw_decode() {
//     let device_type = hwdevice_iterate_types().next().unwrap();
//     // ffplay -f rawvideo -video_size 320x180 tests/output/transcode/bear.frames
//     hw_decode(
//         hwdevice_get_type_name(device_type).expect("Failed to get device type name"),
//         c"tests/assets/vids/bear.mp4",
//         c"tests/output/transcode/bear.frames",
//     )
//     .unwrap();
// }
// pub fn set_ffmpeg_log() {
//     let mut log_lvl = ffi::AV_LOG_VERBOSE;
//     if let Ok(level) = std::env::var("RUST_LOG") {
//         if level == "debug" {
//             log_lvl = ffi::AV_LOG_DEBUG;
//         }
//     }
//     unsafe {
//         ffi::av_log_set_level(log_lvl as i32);
//         ffi::av_log_set_callback(Some(codec_log));
//         // ffi::av_log_set_callback(Some(codec_log));
//     }
// }

// #[unsafe(no_mangle)]
// pub unsafe extern "C" fn codec_log(
//     _opaque: *mut std::os::raw::c_void,
//     level: i32,
//     fmt: *const std::os::raw::c_char,
//     vl: ffi::va_list,
// ) {
//     if level as u32 <= ffi::AV_LOG_VERBOSE {
//         let mut buf = vec![0; 1024];
//         let len = unsafe { ffi::vsprintf(buf.as_mut_ptr(), fmt, vl) as usize };
//         if len <= 1024 {
//             let data = buf.iter().map(|v| *v as u8).collect::<Vec<u8>>();
//             let data_trim = data[..len]
//                 .iter()
//                 .filter(|v| **v != 10u8)
//                 .map(|v| *v)
//                 .collect::<Vec<u8>>();
//             let content = String::from_utf8_lossy(data_trim.as_slice());
//             log::info!("{content}",);
//         }
//     }
// }

#[derive(Clone)]
pub struct GluResourceVideo {
    id: usize,
    pub path: String,
    pub info: GluResourceStatInfo,
    /// 关键，绘制和退出是2个线程，必须用互斥锁管理
    tex: Option<Arc<Mutex<GluResourceVideoTextures>>>,
    ctrl: GluResourceCtrl,
}
impl GluResourceVideo {
    pub fn new(
        idx: usize,
        input: &str,
        state: &Arc<RwLock<GluResourceState>>,
        master: &Arc<Mutex<Option<AudioCtrl<f32>>>>,
        decode_sndr: crossbeam::channel::Sender<DecodedFrame>,
        nppi_ctx: NppStreamContext,
        barrier: &Arc<Barrier>,
    ) -> Result<Self> {
        // 用于接收视频信息和任何错误
        let path = input.to_owned();
        let path1 = path.clone();
        // let (decode_sndr, decode_rcvr) = crossbeam::channel::bounded(CHANNEL_SIZE);
        let (ready_sndr, ready_rcvr) = crossbeam::channel::bounded(CHANNEL_SIZE);
        let (stat_sndr, stat_rcvr) = crossbeam::channel::bounded(1);

        let state1 = Arc::downgrade(state);
        let master = master.clone();
        let barrier = barrier.clone();
        std::thread::spawn(move || {
            // let mut frames = 0;
            match run_worker(
                idx,
                &path,
                &state1,
                decode_sndr,
                &ready_rcvr,
                &stat_sndr,
                &master,
                nppi_ctx,
                barrier,
            ) {
                Ok(_) => {
                    let _ = stat_sndr.send(GluResourceStat::Code(0));
                }
                Err(e) => {
                    log::error!("error {e:?}");
                    let _ = stat_sndr.send(GluResourceStat::Err(e));
                }
            }
            // if stat_sndr_opt.is_some() {
            //     stat_sndr_opt.take();
            // }
            log::info!("Quitting GluResourceVideo for {path}");
        });
        let GluResourceStat::Info(info) = stat_rcvr.recv_timeout(Duration::from_secs(60))? else {
            return Err(anyhow!("received stat error"));
        };
        let ctrl = GluResourceCtrl::new(state, ready_sndr, stat_rcvr);

        let width = info.width;
        let height = info.height;
        let chan_size = width * height;

        let mut cache_rgba = null_mut();
        cuda_error!(cudaMalloc(&mut cache_rgba, chan_size * 4))?;
        cuda_error!(cudaMemset2D(cache_rgba, width * 4, 1, width * 4, height))?;

        // let mut nppi_ctx = unsafe { std::mem::zeroed() };
        // npp_error!(nppGetStreamContext(&mut nppi_ctx))?;

        Ok(Self {
            id: rand::random::<usize>(),
            path: path1,
            info,
            ctrl,
            tex: None,
        })
    }

    fn register_resource(&mut self, tex_rgba_id: *mut std::ffi::c_void) -> Result<()> {
        //  tex_rgba_id: u32
        self.unregister_resource()?;
        let res_rgba = {
            let mut res = null_mut();
            // cuda_error!(cudaGraphicsGLRegisterImage(
            //     &mut res,
            //     tex_rgba_id,
            //     glow::TEXTURE_2D,
            //     0
            // ))?;

            cuda_error!(cudaGraphicsD3D11RegisterResource(
                &mut res,
                tex_rgba_id,
                CUDA_GRAPHIC_REGISTER_FLAG_SURFACE_LOAD_STORE,
            ))?;
            res
        };
        self.tex
            .replace(Arc::new(Mutex::new(GluResourceVideoTextures::new(
                tex_rgba_id,
                res_rgba,
            ))));
        Ok(())
    }

    fn unregister_resource(&mut self) -> Result<()> {
        if let Some(tex) = self.tex.take() {
            if let Ok(lock) = tex.lock() {
                if !lock.res_rgba.is_null() {
                    cuda_error!(cudaGraphicsUnregisterResource(lock.res_rgba))?;
                }
            }
        }
        Ok(())
    }
}

impl GluResource for GluResourceVideo {
    fn get_info(&self) -> Option<GluResourceStatInfo> {
        Some(self.info.clone())
    }
    fn get_id(&self) -> usize {
        self.id
    }
    fn set_id(&mut self, id: usize) {
        self.id = id;
    }
    fn get_ctrl(&self) -> Option<&GluResourceCtrl> {
        Some(&self.ctrl)
    }
    fn stop(&self) -> Result<()> {
        let state = self.ctrl.state.upgrade().ok_or(anyhow!("state dropped"))?;
        let mut state1 = state
            .write()
            .map_err(|e| anyhow!("state write failed {e:?}"))?;
        *state1 = GluResourceState::Stop;
        Ok(())
        // let _ = self.ctrl.command.send(GluResourceCommand::stop());
    }
    fn start(&mut self) -> Result<()> {
        let state = self.ctrl.state.upgrade().ok_or(anyhow!("state dropped"))?;
        let mut state1 = state
            .write()
            .map_err(|e| anyhow!("state write failed {e:?}"))?;
        *state1 = GluResourceState::Start;
        Ok(())
    }
    fn pause(&mut self) -> Result<()> {
        let state = self.ctrl.state.upgrade().ok_or(anyhow!("state dropped"))?;
        let mut state1 = state
            .write()
            .map_err(|e| anyhow!("state write failed {e:?}"))?;
        *state1 = GluResourceState::Start;
        Ok(())
    }
    fn seek(&self, _timestap: i64) {
        // let _ = self.ctrl.command.send(GluResourceCommand::seek(timestap));
    }
    fn register(&mut self, tex_rgba_id: *mut std::ffi::c_void) -> Result<()> {
        self.register_resource(tex_rgba_id)
    }
    fn unregister(&mut self) -> Result<()> {
        self.unregister_resource()
    }
    fn draw(&self, frame: Option<&CudaFrame>) -> Result<()> {
        if let Some(cuda_frame) = frame {
            // log::debug!(
            //     "[Draw]painting {:?} for {}",
            //     cuda_frame.idx.as_ref().map(|v| v.index),
            //     self.id
            // );
            let idx = cuda_frame.idx;
            let pts = cuda_frame.pts;
            if let Some(tex) = &self.tex {
                if let Ok(lock) = tex.try_lock() {
                    let res_rgba = lock.res_rgba;
                    let mut resources = [res_rgba];
                    cuda_check!(
                        cudaGraphicsMapResources(1, resources.as_mut_ptr(), std::ptr::null_mut()),
                        "Cuda Map Resource"
                    );
                    let mut array_rgba = std::ptr::null_mut();
                    cuda_check!(
                        cudaGraphicsSubResourceGetMappedArray(&mut array_rgba, res_rgba, 0, 0),
                        "Cuda Mapped Resource RGBA Array fetching"
                    );
                    cuda_check!(
                        cudaMemcpy2DToArray(
                            array_rgba,
                            0,
                            0,
                            cuda_frame.cache_rgba as *const std::ffi::c_void,
                            cuda_frame.width as usize * 4,
                            cuda_frame.width as usize * 4,
                            cuda_frame.height as usize,
                            CUDA_MEMCPY_DEVICE_TO_DEVICE
                        ),
                        "Cuda cudaMemcpy2DToArray RGBA Array"
                    );
                    cuda_check!(
                        cudaGraphicsUnmapResources(1, resources.as_mut_ptr(), std::ptr::null_mut()),
                        "Cuda cudaGraphicsUnmapResources"
                    );
                }
            }
            log::debug!("[Draw]painted <{pts}> for {idx}");
        }

        Ok(())
    }
}

#[derive(Clone)]
struct GluResourceVideoTextures {
    // _tex_rgba_id: u32,
    _tex_rgba_id: *mut std::ffi::c_void,
    res_rgba: *mut std::ffi::c_void,
}
impl GluResourceVideoTextures {
    pub fn new(tex_rgba_id: *mut std::ffi::c_void, res_rgba: *mut std::ffi::c_void) -> Self {
        Self {
            _tex_rgba_id: tex_rgba_id,
            res_rgba,
        }
    }
}

unsafe impl Send for GluResourceVideoTextures {}
unsafe impl Sync for GluResourceVideoTextures {}

fn run_worker(
    idx: usize,
    path: &str,
    state: &Weak<RwLock<GluResourceState>>,
    // cmd_rcvr: tokio::sync::broadcast::Receiver<GluResourceCommand>,
    decode_sndr: crossbeam::channel::Sender<DecodedFrame>,
    ready_rcvr: &crossbeam::channel::Receiver<CudaFrame>,
    stat_sndr: &crossbeam::channel::Sender<GluResourceStat>,
    master: &Arc<Mutex<Option<AudioCtrl<f32>>>>,
    nppi_ctx: NppStreamContext,
    barrier: Arc<Barrier>,
) -> Result<()> {
    let mut worker = GluResourceVideoWorker::new(
        idx,
        path,
        &state,
        decode_sndr,
        ready_rcvr,
        &stat_sndr,
        master,
        nppi_ctx,
        barrier,
    )?;
    let result = worker.work();
    let _ = elogger!(worker.flush());
    // TODO 关键，自动清理cuda context，这时不能drop
    // let input_file = worker.input_file.clone();
    // log::debug!("clearing up {}", input_file);
    // std::thread::sleep(Duration::from_secs(1));
    drop(worker);
    log::debug!("[worker] cleared up {path}",);
    result
    // Ok(())
}

// #[derive(Clone)]
struct GluResourceVideoWorker {
    idx: usize,
    path: String,
    hw_pix_fmt: ffi::AVPixelFormat,
    video_stream_idx: usize,
    audio_stream_idx: Option<usize>,
    // 视频流的时间基
    video_time_base: AVRational,
    audio_time_base: Option<AVRational>,
    video_dec_ctx: AVCodecContext,
    audio_dec_ctx: Option<AVCodecContext>,
    audio_swr: Option<SwrContext>,
    audio_samples: Option<AVSamples>,
    audio_pts: Option<i64>,
    audio_cache: BytesMut,
    input_ctx: AVFormatContextInput,

    cache_rgb: *mut std::ffi::c_void,
    nppi_ctx: NppStreamContext,

    // audio_sndr: Option<crossbeam::channel::Sender<GluAudioData>>,
    decode_sndr: crossbeam::channel::Sender<DecodedFrame>,
    ready_rcvr: crossbeam::channel::Receiver<CudaFrame>,
    // cmd_rcvr: crossbeam::channel::Receiver<GluResourceCommand>,
    state: Weak<RwLock<GluResourceState>>,
    local_state: GluResourceState,
    local_frames: usize,

    barrier: Arc<Barrier>,

    is_master: bool,
}

impl GluResourceVideoWorker {
    pub fn new(
        idx: usize,
        path: &str,
        state: &Weak<RwLock<GluResourceState>>,
        decode_sndr: crossbeam::channel::Sender<DecodedFrame>,
        ready_rcvr: &crossbeam::channel::Receiver<CudaFrame>,
        stat_sndr: &crossbeam::channel::Sender<GluResourceStat>,
        master: &Arc<Mutex<Option<AudioCtrl<f32>>>>,
        nppi_ctx: NppStreamContext,
        barrier: Arc<Barrier>,
    ) -> Result<Self> {
        // let input_file = (*path
        //     .upgrade()
        //     .ok_or(anyhow!("path is dropped when GluResourceVideoInner::new"))?)
        // .clone();
        let device_type_raw = hwdevice_iterate_types()
            .next()
            .ok_or(anyhow!("hw device not exist"))?;
        let device_name = hwdevice_get_type_name(device_type_raw)
            .ok_or(anyhow!("hw device id {device_type_raw} has not name"))?;
        let device_type = hwdevice_find_type_by_name(device_name);
        if device_type == ffi::AV_HWDEVICE_TYPE_NONE {
            return Err(anyhow!(
                "Device type {} is not supported.",
                device_name.to_string_lossy()
            ));
        }
        let input_file_cstr = utils::str_to_c_str(path);
        let mut input_ctx = AVFormatContextInput::open(&input_file_cstr)?;
        let _ = input_ctx.dump(0, &input_file_cstr);

        // 视频部分
        let framerate;
        let width;
        let height;
        let raw_pix_fmt;
        let video_time_base;
        let video_duration;
        let total_frames;
        let video_stream_idx;
        let hw_pix_fmt;
        let video_dec_ctx = {
            let (stream_idx, decoder) = input_ctx
                .find_best_stream(ffi::AVMEDIA_TYPE_VIDEO)?
                .context("No video stream")?;
            video_stream_idx = stream_idx;
            let input_ctx_ptr = input_ctx.as_mut_ptr();
            // unsafe {
            //     ffi::av_dump_format(input_ctx_ptr, 0, input_file_cstr.as_ptr(), 0);
            // }
            // let stream = &input_ctx.streams()[video_stream_idx];
            video_time_base = input_ctx.streams()[stream_idx].time_base;
            video_duration = input_ctx.streams()[stream_idx].duration;
            total_frames = input_ctx.streams()[stream_idx].nb_frames;
            // let total_frame = stream.nb_frames as usize;
            let mut video_dec_ctx = AVCodecContext::new(&decoder);
            video_dec_ctx.apply_codecpar(&input_ctx.streams()[stream_idx].codecpar())?;
            unsafe {
                let stream_ptr = *(*input_ctx.as_mut_ptr())
                    .streams
                    .offset(stream_idx as isize);
                let dec_ctx_deref = video_dec_ctx.deref_mut();
                (*dec_ctx_deref).framerate =
                    ffi::av_guess_frame_rate(input_ctx_ptr, stream_ptr, null_mut());
                (*dec_ctx_deref).time_base = ffi::av_inv_q((*dec_ctx_deref).framerate);
            }
            framerate = video_dec_ctx.framerate;
            width = video_dec_ctx.width as usize;
            height = video_dec_ctx.height as usize;
            raw_pix_fmt = video_dec_ctx.pix_fmt;
            let _hw_ctx = hw_decoder_init(&mut video_dec_ctx, device_type)?;
            let mut hw_pix_fmt_opt = None;
            // Find supported hw_pix_fmt
            for i in 0.. {
                let Some(config) = decoder.hw_config(i) else {
                    break;
                };
                if config.methods & ffi::AV_CODEC_HW_CONFIG_METHOD_HW_DEVICE_CTX as i32 != 0
                    && config.device_type == device_type
                {
                    // HW_PIX_FMT.set(config.pix_fmt).unwrap();
                    hw_pix_fmt_opt.replace(config.pix_fmt);
                    break;
                }
            }
            if hw_pix_fmt_opt.is_none() {
                return Err(anyhow!(
                    "Decoder {} does not support device type {}",
                    decoder.name().to_string_lossy(),
                    hwdevice_get_type_name(device_type)
                        .map(|x| x.to_string_lossy())
                        .unwrap_or_default()
                ));
            }
            hw_pix_fmt = hw_pix_fmt_opt.unwrap();
            video_dec_ctx.set_get_format(Some(get_hw_format));
            video_dec_ctx.open(None)?;
            video_dec_ctx
        };

        let mut is_master = false;
        // 音频部分
        let mut audio_dec_ctx = None;
        let mut audio_samples = None;
        let mut audio_swr = None;
        let mut audio_duration = None;
        let mut audio_stream_idx = None;
        let mut audio_time_base = None;
        let mut info = GluResourceStatInfo::new(
            width,
            height,
            Some(framerate),
            Some(raw_pix_fmt),
            Some(video_duration),
            audio_duration,
            Some(total_frames as usize),
            None,
            None,
            None,
        );

        if let Some((stream_index, decoder)) = input_ctx
            .find_best_stream(ffi::AVMEDIA_TYPE_AUDIO)
            .context("Find best stream failed.")?
        {
            if let Ok(mut lock) = master.lock() {
                if lock.is_none() {
                    let mut decode_context = AVCodecContext::new(&decoder);
                    decode_context
                        .apply_codecpar(&input_ctx.streams()[stream_index].codecpar())
                        .context("Apply codecpar failed.")?;
                    let duration = input_ctx.streams()[stream_index].duration;
                    decode_context.open(None).context("Could not open codec")?;

                    // 如果播放设备支持格式部匹配，则初始化重采样
                    if let Some((device_channels, device_samplerate, device_sampleformat)) =
                        AUDIO_DEIVCE
                            .as_ref()
                            .map(|v| {
                                let Ok(device_sampleformat) = elogger!(
                                    utils::cpal_splfmt_to_ffmpeg_splfmt(v.config.sample_format())
                                ) else {
                                    return None;
                                };
                                let device_channels = v.config.channels();
                                let device_samplerate = v.config.sample_rate();
                                if device_channels != decode_context.ch_layout.nb_channels as u16
                                    || device_samplerate as i32 != decode_context.sample_rate
                                    || device_sampleformat != decode_context.sample_fmt
                                {
                                    Some((device_channels, device_samplerate, device_sampleformat))
                                } else {
                                    None
                                }
                            })
                            .unwrap_or(None)
                    {
                        // 初始化swr
                        let mut swr = SwrContext::new(
                            &utils::get_channel_layout_for_channels(device_channels as i32),
                            device_sampleformat as i32,
                            device_samplerate as i32,
                            &decode_context.ch_layout,
                            decode_context.sample_fmt,
                            decode_context.sample_rate,
                        )?;
                        swr.init()?;
                        audio_swr.replace(swr);

                        // 目标样本缓冲器
                        let max_dst_nb_samples = unsafe {
                            ffi::av_rescale_rnd(
                                AUDIO_SAMPLES as i64,
                                device_sampleformat as i64,
                                decode_context.sample_rate as i64,
                                ffi::AV_ROUND_UP,
                            ) as i32
                        };
                        audio_samples.replace(
                            AVSamples::new(
                                device_channels as i32,
                                max_dst_nb_samples,
                                device_sampleformat as i32,
                                0,
                            )
                            .ok_or(anyhow!("AVSamples alloc failed for {idx}",))?,
                        );
                    }
                    audio_stream_idx.replace(stream_index);
                    audio_duration.replace(duration);
                    audio_time_base.replace(input_ctx.streams()[stream_index].time_base);
                    info.samplerate.replace(decode_context.sample_rate);
                    info.channels.replace(decode_context.ch_layout.nb_channels);
                    info.sample_fmt.replace(decode_context.sample_fmt);
                    audio_dec_ctx.replace(decode_context);
                    is_master = true;

                    let audio_ctrl: AudioCtrl<f32> = AudioCtrl::new(idx)?;
                    // audio_sndr.replace(audio_ctrl.sndr.clone());
                    lock.replace(audio_ctrl);
                }
            }
        };

        let _ = stat_sndr.send(GluResourceStat::Info(info));
        // cuda_error!(cudaMalloc(&mut cache_uv, chan_size / 2))?;

        let chan_size = width * height;
        let mut cache_rgb = null_mut();
        cuda_error!(cudaMalloc(&mut cache_rgb, chan_size * 3))?;

        // let mut nppi_ctx = unsafe { std::mem::zeroed() };
        // npp_error!(nppGetStreamContext(&mut nppi_ctx))?;

        return Ok(Self {
            // input_file,
            idx,
            path: path.to_owned(),
            video_stream_idx,
            audio_stream_idx,
            hw_pix_fmt,
            video_time_base,
            audio_time_base,
            video_dec_ctx,
            audio_dec_ctx,
            audio_samples,
            audio_cache: BytesMut::new(),
            audio_swr,
            audio_pts: None,
            input_ctx,
            decode_sndr: decode_sndr.clone(),
            ready_rcvr: ready_rcvr.clone(),
            // audio_sndr,
            state: state.clone(),
            local_frames: 0,
            nppi_ctx,
            cache_rgb,
            local_state: GluResourceState::Pause,
            barrier,
            is_master,
        });
    }

    pub fn work(&mut self) -> Result<()> {
        'outer: loop {
            if self.peek_state() {
                break;
            }
            if self.local_state == GluResourceState::Pause {
                std::thread::sleep(Duration::from_millis(100));
                continue 'outer;
            }
            while let Some(mut pkt) = self.input_ctx.read_packet()? {
                if self.peek_state() {
                    break 'outer;
                }
                if pkt.stream_index as usize == self.video_stream_idx {
                    pkt.rescale_ts(self.video_time_base, self.video_dec_ctx.time_base);
                    self.decode_queue_video(Some(&pkt))?;
                } else if self
                    .audio_stream_idx
                    .as_ref()
                    .map(|v| *v == pkt.stream_index as usize)
                    .unwrap_or(false)
                {
                    // && let Some(audio_dec_ctx) = self.audio_dec_ctx.as_ref()
                    // && let Some(audio_sndr) = self.audio_sndr.as_ref()
                    if let Some(audio_dec_ctx) = self.audio_dec_ctx.as_ref()
                        && let Some(audio_time_base) = self.audio_time_base
                    {
                        pkt.rescale_ts(audio_time_base, audio_dec_ctx.time_base);
                    }
                    self.decode_queue_audio(Some(&pkt))?;
                }
                if self.local_state != GluResourceState::Start {
                    continue 'outer;
                }
            }
            // flush
            log::debug!("[worker] flushing {}", self.idx);
            self.decode_queue_video(None)?;
            self.decode_queue_audio(None)?;
            if let Some(audio_stream_idx) = self.audio_stream_idx {
                self.input_ctx.seek(
                    audio_stream_idx as i32,
                    0,
                    (rsmpeg::ffi::AVSEEK_FLAG_BACKWARD | rsmpeg::ffi::AVSEEK_FLAG_FRAME) as i32,
                )?;
            } else {
                self.input_ctx.seek(
                    self.video_stream_idx as i32,
                    0,
                    rsmpeg::ffi::AVSEEK_FLAG_BACKWARD as i32,
                )?;
            }
            self.video_dec_ctx.flush_buffers();
            if let Some(audio_dec_ctx) = &mut self.audio_dec_ctx {
                audio_dec_ctx.flush_buffers();
            }
            self.local_frames = 0;
            self.audio_pts.take();
            log::debug!("[debug] sending eof for {}", self.idx);
            let _ = self.decode_sndr.send(DecodedFrame::EOF(self.idx));
            let result = self.barrier.wait();
            log::debug!("[worker] flushed {} result {result:?}", self.idx);
            // cuda_error!(cudaStreamSynchronize(self.nppi_ctx.hStream))?;
            // cuda_error!(cudaDeviceSynchronize())?;
        }
        Ok(())
    }

    /// 返回是否退出read loop
    fn peek_state(&mut self) -> bool {
        let Some(state) = self.state.upgrade() else {
            return true;
        };
        if let Ok(v) = state.try_read() {
            self.local_state = *v;
        }
        let watch_stat = match self.local_state {
            GluResourceState::Stop => true,
            GluResourceState::Start => false,
            GluResourceState::Pause => false,
        };
        watch_stat
    }

    fn decode_queue_video(
        &mut self,
        packet: Option<&rsmpeg::avcodec::AVPacket>,
        // frames: &mut usize,
    ) -> Result<()> {
        self.video_dec_ctx.send_packet(packet)?;
        loop {
            let frame = match self.video_dec_ctx.receive_frame() {
                Ok(f) => f,
                Err(rsmpeg::error::RsmpegError::DecoderDrainError)
                | Err(rsmpeg::error::RsmpegError::DecoderFlushedError) => break,
                Err(e) => {
                    return Err(anyhow::Error::from(e));
                }
            };
            if frame.format == self.hw_pix_fmt {
                let width = frame.width;
                let height = frame.height;

                let Ok(mut cuda_frame) = self.ready_rcvr.recv_timeout(Duration::from_secs(10))
                else {
                    return Ok(());
                };
                let pts =
                    (frame.pts as f64 * 1000f64 * ffi::av_q2d(self.video_dec_ctx.time_base)) as i64;
                cuda_frame.pts = pts;
                cuda_frame.idx = self.idx;
                // if let Some(idx) = cuda_frame.idx.as_mut() {
                //     idx.index = self.local_frames;
                //     idx.pts = pts;
                // } else {
                //     cuda_frame
                //         .idx
                //         .replace(GluVideoIndex::new(self.local_frames, pts));
                // }
                // cuda_frame.master.replace(self.is_master);
                let chan_size = width * height;
                // 重要，首帧cuda有延迟，会导致绿屏
                if self.local_frames == 0 {
                    // cuda_error!(cudaDeviceSynchronize())?;
                    // cuda_error!(cudaStreamSynchronize(self.nppi_ctx.hStream))?;
                    std::thread::sleep(Duration::from_millis(1));
                }
                // if self.local_frames == 0 {
                //     use std::io::Write;
                //     let mut rgb_data = vec![0; chan_size as usize * 3 / 2];
                //     cuda_check!(
                //         cudaMemcpy2D(
                //             rgb_data.as_mut_ptr() as *mut std::ffi::c_void,
                //             width as usize,
                //             frame.data[0] as *const std::ffi::c_void,
                //             frame.linesize[0] as usize,
                //             width as usize,
                //             height as usize,
                //             CUDA_MEMCPY_DEVICE_TO_HOST
                //         ),
                //         "H"
                //     );
                //     cuda_check!(
                //         cudaMemcpy2D(
                //             rgb_data.as_mut_ptr().offset(chan_size as isize)
                //                 as *mut std::ffi::c_void,
                //             width as usize,
                //             frame.data[1] as *const std::ffi::c_void,
                //             frame.linesize[1] as usize,
                //             width as usize,
                //             height as usize / 2,
                //             CUDA_MEMCPY_DEVICE_TO_HOST
                //         ),
                //         "I"
                //     );
                //     let mut fs = std::fs::File::create(format!("test{}.nv12", self.idx)).unwrap();
                //     let _ = fs.write_all(&rgb_data);
                // }
                npp_error!(nppiNV12ToRGB_8u_P2C3R_Ctx(
                    frame.data.as_ptr() as *const *const std::ffi::c_uchar,
                    frame.linesize[0],
                    self.cache_rgb as *mut std::ffi::c_uchar,
                    width * 3,
                    NppiSize { width, height },
                    self.nppi_ctx,
                ))?;
                cuda_error!(cudaMemcpy2D(
                    cuda_frame.cache_rgba,
                    4,
                    self.cache_rgb,
                    3,
                    3,
                    chan_size as usize,
                    CUDA_MEMCPY_DEVICE_TO_DEVICE
                ))?;
                // {
                //     use std::io::Write;
                //     let mut rgb_data = vec![0; chan_size as usize * 4];
                //     cuda_check!(
                //         cudaMemcpy(
                //             rgb_data.as_mut_ptr() as *mut std::ffi::c_void,
                //             cuda_frame.cache_rgba,
                //             chan_size as usize * 3,
                //             CUDA_MEMCPY_DEVICE_TO_HOST
                //         ),
                //         "H"
                //     );
                //     let mut fs =
                //         std::fs::File::create(format!("test{}.rgba", self.local_frames)).unwrap();
                //     let _ = fs.write_all(&rgb_data);
                // }

                cuda_error!(cudaStreamSynchronize(self.nppi_ctx.hStream))?;
                // TODO! move to align thread

                // let cuda_data = [cuda_data_y, cuda_data_uv];
                // log::debug!(
                //     "[worker] sending frame {} for {}",
                //     self.local_frames,
                //     self.idx,
                // );
                let _ = self.decode_sndr.send(DecodedFrame::Video(cuda_frame));
                log::debug!("[worker] sent frame {pts} for {} ", self.idx,);
                // *frames += 1;
                self.local_frames += 1;
            }
        }
        Ok(())
    }

    fn decode_queue_audio(
        &mut self,
        packet: Option<&rsmpeg::avcodec::AVPacket>,
        // audio_dec_ctx: &mut AVCodecContext,
        // audio_sndr: &crossbeam::channel::Sender<Bytes>,
    ) -> Result<()> {
        let Some(audio_dec_ctx) = self.audio_dec_ctx.as_mut() else {
            return Ok(());
        };
        let Some(audio_device) = AUDIO_DEIVCE.as_ref() else {
            return Ok(());
        };
        let device_channels = audio_device.config.channels();
        let device_samplerate = audio_device.config.sample_rate();
        // let cpal::SupportedBufferSize::Range {
        //     min: device_bufsize_min,
        //     max: device_bufsize_max,
        // } = audio_device.config.buffer_size()
        // else {
        //     return Ok(());
        // };
        let device_sampleformat =
            utils::cpal_splfmt_to_ffmpeg_splfmt(audio_device.config.sample_format())?;
        let device_bytes_per_sample = get_bytes_per_sample(device_sampleformat).ok_or(anyhow!(
            "decode_queue_audio::get_bytes_per_sample failed for {}",
            self.idx
        ))? as usize;
        let device_buf_size = device_bytes_per_sample * device_channels as usize * AUDIO_SAMPLES;

        audio_dec_ctx.send_packet(packet)?;
        loop {
            let frame = match audio_dec_ctx.receive_frame() {
                Ok(f) => f,
                Err(rsmpeg::error::RsmpegError::DecoderDrainError)
                | Err(rsmpeg::error::RsmpegError::DecoderFlushedError) => break,
                Err(e) => {
                    return Err(anyhow::Error::from(e));
                }
            };
            let sample_fmt = audio_dec_ctx.sample_fmt;
            let nb_samples = frame.nb_samples as usize;
            let channels = frame.ch_layout.nb_channels as usize;
            let pts: i64 =
                (frame.pts as f64 * 1000f64 * ffi::av_q2d(audio_dec_ctx.time_base)) as i64;
            let duration: i64 =
                (frame.duration as f64 * 1000f64 * ffi::av_q2d(audio_dec_ctx.time_base)) as i64;
            if let Some(audio_swr) = self.audio_swr.as_mut()
                && let Some(audio_samples) = self.audio_samples.as_mut()
            {
                if self.audio_pts.is_none() {
                    self.audio_pts.replace(pts);
                }
                let audio_pts = self.audio_pts.as_mut().unwrap();
                // 确保目标容器足够大
                let dst_nb_samples = unsafe {
                    ffi::av_rescale_rnd(
                        audio_swr.get_delay(audio_dec_ctx.sample_rate as usize) as i64
                            + nb_samples as i64,
                        audio_device.config.sample_rate() as i64,
                        audio_dec_ctx.sample_rate as i64,
                        ffi::AV_ROUND_UP,
                    ) as i32
                };
                if dst_nb_samples > audio_samples.nb_samples {
                    *audio_samples = AVSamples::new(
                        device_channels as i32,
                        dst_nb_samples,
                        device_sampleformat,
                        1,
                    )
                    .ok_or(anyhow!("AVSamples alloc failed for {}", self.idx))?;
                }

                let ret = unsafe {
                    audio_swr.convert(
                        audio_samples.audio_data.as_mut_ptr(),
                        dst_nb_samples,
                        frame.data.as_ptr() as *const *const u8,
                        nb_samples as i32,
                    )
                }?;
                let dst_bufsize = device_bytes_per_sample * ret as usize * device_channels as usize;
                // cpal数据一般是交错的，所以必须有至少一个通道
                let buf =
                    unsafe { std::slice::from_raw_parts(audio_samples.audio_data[0], dst_bufsize) };
                self.audio_cache.extend_from_slice(buf);
                // let mut output_samples = ret as i64;
                // let buf_duration = output_samples * 1000 / device_samplerate as i64;
                // *audio_pts += buf_duration;

                // 为保持pts准确而清空swr缓存
                if packet.is_none() {
                    let ret1 = unsafe {
                        audio_swr.convert(
                            audio_samples.audio_data.as_mut_ptr(),
                            dst_nb_samples,
                            null(),
                            0 as i32,
                        )?
                    };
                    let dst_bufsize =
                        device_bytes_per_sample * ret1 as usize * device_channels as usize;
                    // cpal数据一般是交错的，所以必须有至少一个通道
                    let buf = unsafe {
                        std::slice::from_raw_parts(audio_samples.audio_data[0], dst_bufsize)
                    };
                    self.audio_cache.extend_from_slice(buf);
                    // output_samples += ret1 as i64;
                }
                while self.audio_cache.len() >= device_buf_size {
                    let data = self.audio_cache.split_to(device_buf_size);
                    let data_samples =
                        data.len() / device_channels as usize / device_bytes_per_sample as usize;
                    let data_duration = data_samples as i64 * 1000 / device_samplerate as i64;
                    // log::debug!(
                    //     "[debug][worker] Audio Sending {} pts {} data_samples {data_samples} packet_none {}",
                    //     data.len(),
                    //     *audio_pts,
                    //     packet.is_none()
                    // );
                    // {
                    //     use std::io::Seek;
                    //     use std::io::Write;
                    //     let mut f = std::fs::File::options()
                    //         .create(true)
                    //         .append(true)
                    //         .open("./test.pcm")?;
                    //     f.seek(std::io::SeekFrom::End(0));
                    //     f.write_all(&data);
                    // }
                    let _ = self.decode_sndr.send(DecodedFrame::Audio(AudioFrame::new(
                        self.idx,
                        *audio_pts,
                        data_duration,
                        &data,
                    )));
                    *audio_pts += data_duration;
                }
                if packet.is_none() {
                    self.audio_pts.take();
                }
            } else {
                let bytes_per_sample =
                    get_bytes_per_sample(sample_fmt).context("Unknown sample fmt")?;
                let total_samples = channels * nb_samples;
                let data = if is_planar_format(sample_fmt) {
                    let mut interleaved = vec![0u8; nb_samples * channels * bytes_per_sample];
                    for sample_idx in 0..nb_samples {
                        for ch in 0..channels {
                            let src_offset = (ch * nb_samples + sample_idx) * bytes_per_sample;
                            let dst_offset = (sample_idx * channels + ch) * bytes_per_sample;
                            unsafe {
                                std::ptr::copy_nonoverlapping(
                                    frame.data[ch].add(src_offset),
                                    interleaved.as_mut_ptr().add(dst_offset),
                                    bytes_per_sample,
                                );
                            }
                        }
                    }
                    Bytes::from(interleaved)
                } else {
                    let interleaved_data = unsafe {
                        slice::from_raw_parts(frame.data[0], total_samples * bytes_per_sample)
                    };
                    Bytes::copy_from_slice(interleaved_data)
                };
                let _ = self.decode_sndr.send(DecodedFrame::Audio(AudioFrame::new(
                    self.idx, pts, duration, &data,
                )));
            }
            // {
            //     use std::io::Seek;
            //     use std::io::Write;
            //     let mut f = std::fs::File::options()
            //         .create(true)
            //         .append(true)
            //         .open("./test.pcm")?;
            //     f.seek(std::io::SeekFrom::End(0));
            //     f.write_all(&data);
            // }

            // // 设备的适配
            // let cache_len = data.len();
            // let device_bufsize_min1 = *device_bufsize_min as usize;
            // let device_bufsize_max1 = if *device_bufsize_max as usize % device_bytes_per_sample == 0
            // {
            //     (*device_bufsize_max as usize / device_bytes_per_sample) * device_bytes_per_sample
            // } else {
            //     *device_bufsize_max as usize
            // };

            // if cache_len > device_bufsize_max1 {
            //     loop {
            //         let cache_len = data.len();
            //         if cache_len < device_bufsize_min1 {
            //             break;
            //         } else if cache_len > device_bufsize_max1 {
            //             let piece = data.split_to(device_bufsize_max1);
            //             let pts_shift = device_bufsize_max1
            //                 / device_bytes_per_sample
            //                 / device_channels as usize;
            //             let _ = audio_sndr.send(GluAudioData::new(piece.freeze(), pts));
            //             pts += pts_shift as i64;
            //         } else {
            //             let _ = audio_sndr.send(GluAudioData::new(data.freeze(), pts));
            //             break;
            //         }
            //     }
            // } else if cache_len >= *device_bufsize_min as usize {
            //     let _ = audio_sndr.send(GluAudioData::new(data.freeze(), pts));
            // }
        }
        Ok(())
    }

    fn flush(&mut self) -> Result<()> {
        self.video_dec_ctx.send_packet(None)?;
        while let Ok(_) = self.video_dec_ctx.receive_frame() {
            self.local_frames += 1;
        }
        self.video_dec_ctx.flush_buffers();

        if let Some(audio_dec_ctx) = &mut self.audio_dec_ctx {
            audio_dec_ctx.send_packet(None)?;
            while let Ok(_) = audio_dec_ctx.receive_frame() {}
            audio_dec_ctx.flush_buffers();
        }
        Ok(())
    }
}

impl Drop for GluResourceVideoWorker {
    fn drop(&mut self) {
        if !self.cache_rgb.is_null() {
            cuda_check!(
                cudaFree(self.cache_rgb),
                "CudaFree rgba Err for GluResourceVideoWorker"
            );
        }
    }
}

fn hw_decoder_init(
    ctx: &mut AVCodecContext,
    device_type: AVHWDeviceType,
) -> Result<AVHWDeviceContext> {
    let hw_device_ctx = AVHWDeviceContext::create(device_type, None, None, 0)
        .context("Failed to create specified HW device.")?;
    ctx.set_hw_device_ctx(hw_device_ctx.clone());
    Ok(hw_device_ctx)
}

unsafe extern "C" fn get_hw_format<'a>(
    _ctx: *mut ffi::AVCodecContext,
    pix_fmts: *const AVPixelFormat,
) -> AVPixelFormat {
    let pix_fmts = unsafe { build_array(pix_fmts, ffi::AV_PIX_FMT_NONE) }.unwrap_or_default();
    for &pix_fmt in pix_fmts {
        if pix_fmt == ffi::AV_PIX_FMT_CUDA {
            return pix_fmt;
        }
    }
    log::warn!("Failed to get HW surface format.");
    ffi::AV_PIX_FMT_NONE
}

pub struct GluPlayer {
    videos: Vec<Box<dyn GluResource>>,
    sync: Option<GluResourceCtrlSync>,
    cache: Vec<HashMap<usize, CudaFrame>>,
    // master: Arc<Mutex<GluSystemResourceStatInfo>>,
    // logger: String,
    // running: Arc<AtomicBool>,
    // r_sndr: std::sync::mpsc::Sender<i32>,
}
impl GluPlayer {
    pub fn new(video_paths: &Vec<String>) -> Result<Self> {
        if !cfg!(target_has_atomic = "64") {
            panic!("Platform will need 64bit atomic operation");
        }
        // let logger = logger.unwrap_or("./log".to_owned());
        // let _ = utils::init_logger(&logger)?;
        let (decode_sndr, decode_rcvr) = crossbeam::channel::unbounded();
        let state = Arc::new(RwLock::new(GluResourceState::Pause));
        // let (master_inner, audio_rcvr) = GluSystemResourceStatInfo::new(None);
        let master = Arc::new(Mutex::new(None));
        let mut nppi_ctx = unsafe { std::mem::zeroed() };
        npp_error!(nppGetStreamContext(&mut nppi_ctx))?;
        let mut videos = vec![];
        let barrier = Arc::new(Barrier::new(video_paths.len()));
        for (i, video_path) in video_paths.iter().enumerate() {
            let mut glu_image = elogger!(GluResourceVideo::new(
                i,
                &video_path,
                &state,
                &master,
                decode_sndr.clone(),
                nppi_ctx,
                &barrier
            ))?;
            glu_image.set_id(i);
            videos.push(Box::new(glu_image) as Box<dyn GluResource>);
        }
        let sync_ctrl = GluResourceCtrlSync::new(state, &videos, decode_rcvr, master)?;
        // let running = Arc::new(AtomicBool::new(true));
        // let r = running.clone();
        // let (r_sndr, r_rcvr) = std::sync::mpsc::channel::<i32>();
        // ctrlc::set_handler(move || {
        //     log::warn!("Ctrl-C received, gracefully clearing up cuda");
        //     r.store(false, Ordering::SeqCst);
        //     let _ = r_rcvr.recv();
        //     log::warn!("Gracefully clearing up done");
        // })?;

        Ok(Self {
            videos,
            sync: Some(sync_ctrl),
            cache: vec![],
            // master,
            // logger,
            // running,
            // r_sndr,
        })
    }
    pub fn get_ratios(&self) -> Vec<(usize, usize)> {
        self.videos
            .iter()
            .map(|v| {
                let info = v.get_info().unwrap();
                (info.width, info.height)
            })
            .collect()
    }

    pub fn register_textures(&mut self, texture_rgba_ids: &[*mut std::ffi::c_void]) -> Result<()> {
        if self.videos.len() < texture_rgba_ids.len() {
            return Err(anyhow!("texture id len mismatch"));
        }
        for (i, video) in self.videos.iter_mut().enumerate() {
            video.register(texture_rgba_ids[i])?;
        }
        Ok(())
    }
    pub fn unregister_textures(&mut self) -> Result<()> {
        for video in self.videos.iter_mut() {
            video.unregister()?;
        }
        Ok(())
    }

    pub fn start(&mut self) -> Result<()> {
        if let Some(sync_ctrl) = &mut self.sync {
            let _ = sync_ctrl.start()?;
        }
        Ok(())
    }
    pub fn load_frames(&mut self) -> Result<i32> {
        let Some(sync_ctrl) = &mut self.sync else {
            return Err(anyhow!("sync not set yet"));
        };
        if let Some(data) = sync_ctrl.load_frames() {
            self.cache.push(data);
        }
        Ok(self.cache.len() as i32)
    }
    // pub fn draw_frames(&mut self, frames: &HashMap<usize, CudaFrame>) -> Result<()> {
    //     if self.videos.len() < frames.len() {
    //         return Err(anyhow!("texture id len mismatch"));
    //     }
    //     for (i, video) in self.videos.iter_mut().enumerate() {
    //         if let Some(frame) = frames.get(&i) {
    //             let _ = video.draw(Some(frame))?;
    //         }
    //     }
    //     Ok(())
    // }
    pub fn draw_frames(&mut self) -> Result<()> {
        if self.cache.len() == 0 {
            return Err(anyhow!("cache is 0"));
        }
        let Some(sync_ctrl) = &mut self.sync else {
            return Err(anyhow!("sync not set yet"));
        };
        let item = self.cache.remove(0);
        if self.videos.len() != item.len() {
            sync_ctrl.recycle(item)?;
            return Err(anyhow!("texture id len mismatch"));
        }
        for (k, v) in &item {
            let _ = self.videos[*k].draw(Some(v))?;
        }
        // sync_ctrl.recycle(self.cache.drain().collect())?;
        sync_ctrl.recycle(item)?;
        Ok(())
    }
    // pub fn recycle_frames(&mut self) -> Result<()> {
    //     let Some(sync_ctrl) = &mut self.sync else {
    //         return Err(anyhow!("sync not set yet"));
    //     };
    //     sync_ctrl.recycle(self.cache.drain().collect())?;
    //     Ok(())
    // }
    pub fn pause(&mut self) -> Result<()> {
        let Some(sync_ctrl) = &mut self.sync else {
            return Err(anyhow!("sync not set yet"));
        };
        sync_ctrl.pause()
    }
    pub fn stop(&mut self) -> Result<()> {
        let Some(sync_ctrl) = &mut self.sync else {
            return Err(anyhow!("sync not set yet"));
        };
        sync_ctrl.stop()
    }
    pub fn terminate(mut self) -> Result<()> {
        if self.sync.is_none() {
            return Err(anyhow!("sync not set yet"));
        };
        let sync = self.sync.take();
        drop(self);
        if let Some(sync_ctrl) = sync {
            let _ = sync_ctrl.terminate()?;
        }
        // let _ = self.r_sndr.send(0);
        Ok(())
    }
}

pub fn run_video_window(type_: i32, view_ports: usize) -> Result<()> {
    // unsafe {
    //     let _ = std::env::set_var("RUST_LOG", "info");
    // }
    // {
    //     let env = env_logger::Env::default()
    //         .filter("RUST_LOG")
    //         .write_style("RUST_LOG_STYLE");
    //     env_logger::Builder::from_env(env)
    //         // .format_level(false)
    //         .format_timestamp_micros()
    //         .init();
    // }
    let _ = utils::cleaner("./log", false);
    let _ = utils::init_logger("./log");
    // env_logger::init();
    // let event_loop_builder = EventLoop::builder();
    let mut builder = EventLoop::<UserEvent>::with_user_event();
    let event_loop = builder.build()?;
    let media_path = PathBuf::from_str("./media")?;
    let mut resources = vec![];

    let video_name = match type_ {
        0 => "video",
        1 => "video_hevc",
        2 => "sample",
        3 => "sample_idx",
        // 4 => "test1/screen_",
        4 => "test2/screen_",
        5 => "test03/screen_",
        6 => "test04_nvenc_hevc_long/fh_screen_",
        7 => "test05_nvenc_hevc_scale/fh_screen_",
        8 => "video_short",
        _ => "",
    };
    // if type_ == 0 { "video" } else if type { "video_hevc" };
    // let view_ports = 3;
    let (decode_sndr, decode_rcvr) = crossbeam::channel::unbounded();
    let state = Arc::new(RwLock::new(GluResourceState::Pause));
    // let (master_inner, audio_rcvr) = GluSystemResourceStatInfo::new(None);
    let master = Arc::new(Mutex::new(None));
    let mut nppi_ctx = unsafe { std::mem::zeroed() };
    npp_error!(nppGetStreamContext(&mut nppi_ctx))?;
    let barrier = Arc::new(Barrier::new(view_ports));
    for i in 0..view_ports {
        let video_path = media_path
            .join(format!("{video_name}{i}.mp4"))
            .to_string_lossy()
            .to_string();
        // let glu_image = GluResourceImage::new(&img_path, 0, None, None,).expect("23");
        // images.push(glu_image);
        if let Ok(mut glu_image) = elogger!(GluResourceVideo::new(
            i,
            &video_path,
            &state,
            &master,
            decode_sndr.clone(),
            nppi_ctx,
            &barrier
        )) {
            glu_image.set_id(i);
            resources.push(Box::new(glu_image) as Box<dyn GluResource>);
        };
    }
    // let proxy = event_loop.create_proxy();
    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();
    // let mut app = App::new(running, vec![], None);
    // let onload = move |data: HashMap<usize, CudaFrame>| -> Result<()> {
    //     let _ = proxy.send_event(UserEvent::Frames(data));
    //     Ok(())
    // };
    let sync_ctrl = GluResourceCtrlSync::new(state, &resources, decode_rcvr, master)?;
    // app.resources = resources;
    // app.sync_ctrl.replace(sync_ctrl);
    let mut app = AppRunner::new(running, resources, Some(sync_ctrl));
    let (sndr, rcvr) = std::sync::mpsc::channel::<i32>();
    ctrlc::set_handler(move || {
        log::warn!("Ctrl-C received, gracefully clearing up cuda");
        r.store(false, Ordering::SeqCst);
        let _ = rcvr.recv();
        // std::thread::sleep(Duration::from_secs(1));
        // std::thread::sleep(Duration::from_millis(60));
        log::warn!("Gracefully clearing up done");
    })?;
    let sync_ctrl = {
        event_loop.run_app(&mut app)?;
        let sync_ctrl = app.sync_ctrl.take().map(|mut v| {
            let _ = elogger!(v.stop());
            v
        });
        drop(app);
        sync_ctrl
    };
    if let Some(sync_ctrl) = sync_ctrl {
        let _ = sync_ctrl.terminate();
    }
    // log::debug!("app dropped");
    let _ = sndr.send(0);
    // std::thread::sleep(Duration::from_millis(60));
    Ok(())
}

pub fn is_planar_format(sample_fmt: AVSampleFormat) -> bool {
    match sample_fmt {
        AV_SAMPLE_FMT_U8P | AV_SAMPLE_FMT_S16P | AV_SAMPLE_FMT_S32P | AV_SAMPLE_FMT_FLTP
        | AV_SAMPLE_FMT_DBLP | AV_SAMPLE_FMT_S64P | AV_SAMPLE_FMT_NB => true,
        _ => false,
    }
}

#[derive(Clone)]
pub struct GluSystemResourceStatInfo {
    pub audio_sndr: crossbeam::channel::Sender<GluAudioData>,
    pub info: Option<GluResourceStatInfo>,
}
impl GluSystemResourceStatInfo {
    pub fn new(
        info: Option<GluResourceStatInfo>,
    ) -> (Self, crossbeam::channel::Receiver<GluAudioData>) {
        let (audio_sndr, audio_rcvr) = crossbeam::channel::unbounded();
        (Self { audio_sndr, info }, audio_rcvr)
    }
}
#[derive(Clone)]
pub struct GluAudioData {
    pub data: Bytes, // 如果data为空则表示eof，需要重置时间轴
    pub pts: i64,
}
impl GluAudioData {
    pub fn new(data: Bytes, pts: i64) -> Self {
        Self { data, pts }
    }
}

#[test]
pub fn test_nv12_rgb() {
    use crate::{
        cuda::{
            CUDA_MEMCPY_DEVICE_TO_HOST, CUDA_MEMCPY_HOST_TO_DEVICE, NppiSize, cudaFree, cudaMalloc,
            cudaMemcpy, nppGetStreamContext, nppiNV12ToRGB_8u_P2C3R_Ctx,
        },
        cuda_check,
    };
    use std::io::{Read, Write};
    // unsafe {
    //     let _ = std::env::set_var("RUST_LOG", "debug");
    // }
    // {
    //     let env = env_logger::Env::default()
    //         .filter("RUST_LOG")
    //         .write_style("RUST_LOG_STYLE");
    //     env_logger::Builder::from_env(env)
    //         // .format_level(false)
    //         .format_timestamp_micros()
    //         .init();
    // }
    let _ = utils::init_logger("./log");
    let mut data = vec![];
    let mut fs =
        std::fs::File::open("/data/workspace/boe/2026_heterogeneous_space_aigc/media/mock0.nv12")
            .unwrap();
    let _ = fs.read_to_end(&mut data);

    let w = 1920;
    let h = 1080;
    let size = w * h;
    let mut cuda_data_y = null_mut();
    cuda_check!(cudaMalloc(&mut cuda_data_y, size), "A");
    cuda_check!(
        cudaMemcpy(
            cuda_data_y,
            data.as_ptr() as *const std::ffi::c_void,
            size,
            CUDA_MEMCPY_HOST_TO_DEVICE
        ),
        "B"
    );
    let mut cuda_data_uv = null_mut();
    cuda_check!(cudaMalloc(&mut cuda_data_uv, size / 2), "C");
    cuda_check!(
        cudaMemcpy(
            cuda_data_uv,
            data.as_ptr().offset(size as isize) as *const std::ffi::c_void,
            size / 2,
            CUDA_MEMCPY_HOST_TO_DEVICE
        ),
        "D"
    );
    let mut cuda_data_rgb = null_mut();
    cuda_check!(cudaMalloc(&mut cuda_data_rgb, size * 3), "E");
    let cuda_data = [cuda_data_y, cuda_data_uv];

    // let nppi_ctx = null_mut();
    unsafe {
        let mut nppi_ctx = std::mem::zeroed();
        let code0 = nppGetStreamContext(&mut nppi_ctx);
        println!("code0 {code0}");
        let code = nppiNV12ToRGB_8u_P2C3R_Ctx(
            cuda_data.as_ptr() as *const *const std::ffi::c_uchar,
            w as i32,
            cuda_data_rgb as *mut std::ffi::c_uchar,
            w as i32 * 3,
            NppiSize {
                width: w as i32,
                height: h as i32,
            },
            nppi_ctx, // rSrcStep: ::std::os::raw::c_int,
                      // pDst: *mut std::ffi::c_uchar,
                      // nDstStep: ::std::os::raw::c_int,
                      // oSizeROI: NppiSize,
        );
        println!("code {code}");
    }
    cuda_check!(cudaFree(cuda_data_y), "F");
    cuda_check!(cudaFree(cuda_data_uv), "G");
    let mut rgb_data = vec![0; size * 3];
    cuda_check!(
        cudaMemcpy(
            rgb_data.as_mut_ptr() as *mut std::ffi::c_void,
            cuda_data_rgb,
            size * 3,
            CUDA_MEMCPY_DEVICE_TO_HOST
        ),
        "H"
    );
    cuda_check!(cudaFree(cuda_data_rgb), "G");
    let mut fs = std::fs::File::create("test.rgb").unwrap();
    let _ = fs.write_all(&rgb_data);
}
