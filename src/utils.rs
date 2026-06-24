use crate::elogger;
use anyhow::{Result, anyhow};
use cpal::traits::DeviceTrait;
use cpal::traits::StreamTrait;
use cpal::{FromSample, Sample, SizedSample, StreamConfig};
use lazy_static::lazy_static;
use rsmpeg::avutil::get_bytes_per_sample;
use rsmpeg::ffi::{
    self, AV_CHANNEL_LAYOUT_5POINT0, AV_CHANNEL_LAYOUT_5POINT1, AV_CHANNEL_LAYOUT_6POINT1,
    AV_CHANNEL_LAYOUT_7POINT1, AV_CHANNEL_LAYOUT_MONO, AV_CHANNEL_LAYOUT_QUAD,
    AV_CHANNEL_LAYOUT_STEREO, AV_CHANNEL_LAYOUT_SURROUND, AV_SAMPLE_FMT_DBL, AV_SAMPLE_FMT_FLT,
    AV_SAMPLE_FMT_S16, AV_SAMPLE_FMT_S32, AV_SAMPLE_FMT_U8, AVChannelLayout,
    av_channel_layout_default,
};
use std::sync::{Arc, Mutex};
use std::{
    ffi::{CStr, CString, c_char},
    path::Path,
};

pub const CHANNEL_SIZE: usize = 10;
pub const AUDIO_SAMPLES: usize = 1024;
pub const AUDIO_MAX_DELAY: usize = 50; //ms

lazy_static! {
    pub static ref AUDIO_DEIVCE: Option<GluSystemAudioDevice> = GluSystemAudioDevice::new().ok();
    // pub static ref AUDIO_PLAY_BUFSIZE: Option<usize> = {
    //     match AUDIO_DEIVCE {
    //         Some(audio_device)=>{
    //             let mut config: StreamConfig = audio_device.config.clone().into();
    //             config.buffer_size = cpal::BufferSize::Fixed(AUDIO_SAMPLES as u32);
    //             // let num_channels = config.channels as usize;
    //             let err_fn = |err| eprintln!("Error building output sound stream: {}", err);
    //             audio_device.device.build_output_stream(
    //                 &config,
    //                 move |output: &mut [T], _: &cpal::OutputCallbackInfo| {
    //                     output.len()
    //                 },
    //                 err_fn,
    //                 None,
    //             )?;
    //         },
    //         None=> None
    //     }
    // };
}

pub fn c_str_to_string_uncheck(c_str: *const c_char) -> String {
    unsafe { CStr::from_ptr(c_str) }
        .to_str()
        .unwrap()
        .to_string()
}

pub fn c_str_to_string(c_str: *const c_char) -> Result<String> {
    Ok(unsafe { CStr::from_ptr(c_str) }
        .to_str()
        .map(|v| v.to_owned())?)
}

pub fn str_to_c_str(str: &str) -> CString {
    CString::new(str).expect("could not alloc CString")
}

pub fn string_to_c_str(str: String) -> CString {
    CString::new(&str[..]).expect("could not alloc CString")
}

pub fn av_q2d(n: ffi::AVRational) -> f64 {
    if n.den == 0 {
        return 0f64;
    }
    n.num as f64 / n.den as f64
}

fn build_rolling_appender(
    name: &str,
    path: &Path,
) -> Result<log4rs::append::rolling_file::RollingFileAppender> {
    use log4rs::append::rolling_file::RollingFileAppender;
    use log4rs::append::rolling_file::policy::compound::CompoundPolicy;
    use log4rs::append::rolling_file::policy::compound::roll::fixed_window::FixedWindowRoller;
    use log4rs::append::rolling_file::policy::compound::trigger::size::SizeTrigger;
    use log4rs::encode::pattern::PatternEncoder;

    let trigger = Box::new(SizeTrigger::new(100 * 1024 * 1024));
    let roller_count = 30;
    let roller_base = 0;
    let roller = Box::new(
        FixedWindowRoller::builder()
            .base(roller_base)
            .build(
                &format!("log/compressed-{name}-log-{{}}-.log"),
                roller_count,
            )
            .unwrap(),
    );
    let compound_policy = Box::new(CompoundPolicy::new(trigger, roller));
    Ok(RollingFileAppender::builder()
        .encoder(Box::new(PatternEncoder::new(&format!(
            "[{}] {{d}} - {{l}} - {{t}} - {{m}}{{n}}",
            name.to_uppercase()
        ))))
        .build(path.join(format!("{name}.log")), compound_policy)?)
}

#[derive(Clone)]
pub struct GluSystemAudioDevice {
    pub device: cpal::Device,
    pub config: cpal::SupportedStreamConfig,
}

impl GluSystemAudioDevice {
    pub fn new() -> Result<Self> {
        let (_host, device, config) = host_device_setup()?;
        Ok(Self { device, config })
    }
}

pub fn host_device_setup() -> Result<(cpal::Host, cpal::Device, cpal::SupportedStreamConfig)> {
    use cpal::traits::DeviceTrait;
    use cpal::traits::HostTrait;
    let host = cpal::default_host();

    let device = host
        .default_output_device()
        .ok_or_else(|| anyhow::Error::msg("Default output device is not available"))?;
    log::info!("Output device : {}", device.name()?);
    let config = device.default_output_config()?;
    log::info!("Default output config : {:?}", config);
    Ok((host, device, config))
}
pub fn make_stream<T: SizedSample + Send + FromSample<f32> + 'static>(
    sample_rcvr: std::sync::mpsc::Receiver<Vec<T>>,
) -> Result<AudioStream<T>>
where
    T: SizedSample + Send + 'static,
{
    let Some(audio_device) = AUDIO_DEIVCE.as_ref() else {
        return Err(anyhow!("Audio Device not found"));
    };

    let config: StreamConfig = audio_device.config.clone().into();
    // config.buffer_size = cpal::BufferSize::Fixed(AUDIO_SAMPLES as u32);
    // let num_channels = config.channels as usize;
    let err_fn = |err| eprintln!("Error building output sound stream: {}", err);

    // let time_at_start = std::time::Instant::now();
    // println!(
    //     "num_channels {num_channels} Time at start: {:?} config {:?}",
    //     time_at_start, config
    // );

    // config.buffer_size =
    // let channels = config.channels;
    let cache: Arc<Mutex<Vec<T>>> = Arc::new(Mutex::new(vec![]));
    let cahce1 = cache.clone();
    let Some(audio_device) = AUDIO_DEIVCE.as_ref() else {
        return Err(anyhow!("Audio Device not set"));
    };
    let device_sampleformat = cpal_splfmt_to_ffmpeg_splfmt(audio_device.config.sample_format())?;
    let device_bytes_per_sample = get_bytes_per_sample(device_sampleformat).ok_or(anyhow!(
        "decode_queue_audio::get_bytes_per_sample failed for <make_stream>",
    ))? as usize;
    let threshold = AUDIO_MAX_DELAY
        * config.sample_rate as usize
        * device_bytes_per_sample
        * config.channels as usize
        / 1000;
    let stream = audio_device.device.build_output_stream(
        &config,
        move |output: &mut [T], _: &cpal::OutputCallbackInfo| {
            let output_len = output.len();
            let Ok(mut locker) = cahce1.try_lock() else {
                output.iter_mut().for_each(|d| {
                    *d = Sample::EQUILIBRIUM;
                });
                return;
            };
            if let Ok(data) = sample_rcvr.try_recv() {
                locker.extend_from_slice(&data);
                // if let Ok(mut locker) = cahce1.lock() {
                //     // let actual_len = data.len();
                //     // if actual_len > output_len {
                //     //     log::warn!("need {output_len} actual_len {actual_len}");
                //     // }
                // }
            }
            let cache_len = locker.len();
            if cache_len > threshold {
                let tmp = locker.split_off(threshold);
                *locker = tmp;
            }
            let actual_len = std::cmp::min(output_len, locker.len());
            let tmp = locker.split_off(actual_len);
            output.iter_mut().enumerate().for_each(|(i, d)| {
                if i >= actual_len {
                    // *d = Sample::EQUILIBRIUM;
                    *d = Sample::EQUILIBRIUM;
                } else {
                    *d = locker[i];
                }
            });
            *locker = tmp;
            // processed = true;

            // if !processed {
            //     output.iter_mut().for_each(|d| {
            //         *d = Sample::EQUILIBRIUM;
            //     });
            // }
        },
        err_fn,
        None,
    )?;
    Ok(AudioStream::new(stream, cache))
}

pub struct AudioStream<T: SizedSample + Send + 'static> {
    pub stream: cpal::Stream,
    _cache: Arc<Mutex<Vec<T>>>,
}
impl<T: SizedSample + Send + 'static> AudioStream<T> {
    pub fn new(stream: cpal::Stream, _cache: Arc<Mutex<Vec<T>>>) -> Self {
        Self { stream, _cache }
    }
    // pub fn new() -> Self {
    //     Self { stream, cache }
    // }
}

pub struct AudioCtrl<T: SizedSample + FromSample<f32> + Send + 'static> {
    pub idx: usize,
    pub sndr: std::sync::mpsc::Sender<Vec<T>>,
    pub streamer: AudioStream<T>,
}
impl<T: SizedSample + FromSample<f32> + Send + 'static> AudioCtrl<T> {
    pub fn new(idx: usize) -> Result<Self> {
        let (sample_sndr, sample_rcvr) = std::sync::mpsc::channel();
        let streamer = make_stream(sample_rcvr)?;
        streamer.stream.play()?;
        Ok(Self {
            idx,
            sndr: sample_sndr,
            streamer,
        })
    }
}

// fn process_frame(output: &mut [T], num_channels: usize, distributer: &mut Distributer<T>)
// where
//     T: Sample,
// {
//     let output_len = output.len();
//     if let Some(data) = distributer.try_rcv(output_len) {
//         let actual_len = data.len();
//         // println!("need {output_len} actual_len {actual_len}");
//         output.iter_mut().enumerate().for_each(|(i, d)| {
//             let j = i;
//             if j >= actual_len {
//                 *d = Sample::EQUILIBRIUM;
//                 // return;
//             } else {
//                 // *d = Sample::from_sample(data[j]);
//                 *d = data[j];
//             }
//         });
//     } else {
//         output.iter_mut().enumerate().for_each(|(i, d)| {
//             *d = Sample::EQUILIBRIUM;
//         });
//     }
// }

pub fn init_logger(logger: &str) -> Result<()> {
    use log::LevelFilter;
    use log4rs::Config;
    use log4rs::append::console::ConsoleAppender;
    use log4rs::config::Appender;
    use log4rs::config::Logger;
    use log4rs::config::Root;
    use log4rs::encode::pattern::PatternEncoder;

    let log_folder = Path::new(logger);
    if log_folder.is_file() {
        return Err(anyhow!("logger {logger} should be dir"));
    }
    if !log_folder.exists() {
        let _ = std::fs::create_dir_all(log_folder)?;
    }
    let stdout = ConsoleAppender::builder()
        .encoder(Box::new(PatternEncoder::new(
            "[CONSOLE] {d} - {l} - {t} - {m}{n}",
        )))
        .build();
    let app_appender = build_rolling_appender("app", log_folder)?;
    let debug_appender = build_rolling_appender("debug", log_folder)?;

    // .logger(Logger::builder().build("app::backend::db", LevelFilter::Info))
    let config = Config::builder()
        .appender(Appender::builder().build("stdout", Box::new(stdout)))
        .appender(Appender::builder().build("app", Box::new(app_appender)))
        .appender(Appender::builder().build("debug", Box::new(debug_appender)))
        .logger(
            Logger::builder()
                .appender("app")
                .additive(false)
                .build("app", LevelFilter::Info),
        )
        .logger(
            Logger::builder()
                .appender("debug")
                .additive(false)
                .build("debug", LevelFilter::Debug),
        )
        .build(Root::builder().appender("app").build(LevelFilter::Debug))?;
    log4rs::init_config(config)?;
    Ok(())
}

/// 扫描并清除log目录中超过7天的日志
pub fn cleaner(logger: &str, interval: bool) {
    if !interval {
        if let Ok(results) = std::fs::read_dir(logger) {
            for entry in results {
                if let Ok(de) = entry {
                    let _ = elogger!(std::fs::remove_file(de.path()));
                }
            }
        }
    } else {
        let logger = logger.to_owned();
        std::thread::spawn(move || {
            log::info!("clearing up log/storage folder");
            log::debug!(target:"debug","clearing up log/storage folder");
            if let Ok(results) = std::fs::read_dir(&logger) {
                for entry in results {
                    if let Ok(de) = entry {
                        if let Ok(meta) = de.metadata() {
                            let mut can_delete = false;
                            if let Ok(modified) = meta.modified() {
                                if let Ok(elisped) = modified.elapsed() {
                                    // 默认保留一周
                                    if elisped.as_secs() > 7 * 24 * 3600 {
                                        can_delete = true;
                                    }
                                }
                            }
                            if can_delete {
                                if meta.file_type().is_file() {
                                    let _ = std::fs::remove_file(de.path());
                                } else if meta.file_type().is_dir() {
                                    let _ = std::fs::remove_dir_all(de.path());
                                }
                            }
                        }
                    }
                }
            }
        });
    }

    // let logger1 = logger.to_owned();
    // std::thread::spawn(move || {

    // });
}

pub fn get_channel_layout_for_channels(channels: i32) -> AVChannelLayout {
    match channels {
        1 => AV_CHANNEL_LAYOUT_MONO,
        2 => AV_CHANNEL_LAYOUT_STEREO,
        3 => AV_CHANNEL_LAYOUT_SURROUND, // 3.0 (L, R, C)
        4 => AV_CHANNEL_LAYOUT_QUAD,     // 4.0 (FL, FR, BL, BR)
        5 => AV_CHANNEL_LAYOUT_5POINT0,  // 5.0 (FL, FR, C, SL, SR)
        6 => AV_CHANNEL_LAYOUT_5POINT1,  // 5.1 (FL, FR, C, LFE, SL, SR)
        7 => {
            // 7.0 或 6.1，选择常见的 6.1
            AV_CHANNEL_LAYOUT_6POINT1
        }
        8 => AV_CHANNEL_LAYOUT_7POINT1, // 7.1 (FL, FR, C, LFE, SL, SR, BL, BR)
        _ => {
            // 对于其他声道数，创建自定义布局
            create_default_channel_layout(channels)
        }
    }
}
fn create_default_channel_layout(channels: i32) -> AVChannelLayout {
    let mut channel_layout = std::mem::MaybeUninit::<AVChannelLayout>::uninit();
    unsafe {
        // 使用默认的声道顺序创建布局
        av_channel_layout_default(channel_layout.as_mut_ptr(), channels);
        channel_layout.assume_init()
    }
}

pub fn cpal_splfmt_to_ffmpeg_splfmt(fmt: cpal::SampleFormat) -> Result<ffi::AVSampleFormat> {
    let format = match fmt {
        cpal::SampleFormat::U8 => AV_SAMPLE_FMT_U8,
        cpal::SampleFormat::I16 => AV_SAMPLE_FMT_S16,
        cpal::SampleFormat::I32 => AV_SAMPLE_FMT_S32,
        cpal::SampleFormat::F32 => AV_SAMPLE_FMT_FLT,
        cpal::SampleFormat::F64 => AV_SAMPLE_FMT_DBL,
        _ => return Err(anyhow!("Not supported {fmt:?}")),
    };
    Ok(format)
}
// #[track_caller]
// pub fn log_error<E: std::fmt::Debug>(e: E) -> E {
//     let location = std::panic::Location::caller();

//     log::error!(
//         "module: {} -> file: {} -> line: {}",
//         module_path!(),
//         location.file(),
//         location.line()
//     );
//     log::error!("error: {:?}", e);

//     e
// }
