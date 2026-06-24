use crate::cuda::{cudaFree, cudaMalloc, cudaMemset2D};
use crate::utils::{AudioCtrl, CHANNEL_SIZE};
use crate::{cuda_check, cuda_error, elogger};
use anyhow::{Result, anyhow};
use bytes::Bytes;
use glow::HasContext;
use glutin::config::ConfigTemplateBuilder;
use glutin::context::{ContextAttributesBuilder, PossiblyCurrentContext};
use glutin::display::GetGlDisplay;
use glutin::prelude::*;
use glutin::surface::{Surface, SurfaceAttributesBuilder, SwapInterval, WindowSurface};
use glutin_winit::DisplayBuilder;
use std::collections::HashMap;
use std::ffi::CString;
use std::num::NonZeroU32;
use std::ptr::null_mut;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock, Weak};
use std::time::{Duration, Instant};
use std::{f64, usize};
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::{KeyCode, PhysicalKey};
#[allow(deprecated)]
use winit::raw_window_handle::HasRawWindowHandle;
use winit::window::{Window, WindowAttributes};

const WIDTH: i32 = 1920;
const HEIGHT: i32 = 1080;
// pub const VIEW_PORTS: usize = 5;
// macro_rules! cuda_check {
//     ($expr:expr, $msg:expr) => {{
//         let err = unsafe { $expr };
//         if err != 0 {
//             println!("❌ CUDA ERROR {} at {}", err, $msg);
//         } else {
//             println!("✅ CUDA OK: {}", $msg);
//         }
//     }};
// }

// #[derive(Default)]
pub struct App {
    pub running: Arc<AtomicBool>,
    pub window: Option<Window>,
    pub gl: Option<glow::Context>,
    pub surface: Option<Surface<WindowSurface>>,
    pub context: Option<PossiblyCurrentContext>,

    pub tex_rgb: Vec<glow::NativeTexture>,
    pub resources: Vec<Box<dyn GluResource>>,
    pub program: Option<glow::Program>,
    pub vao: Option<glow::NativeVertexArray>,

    pub last_op: Instant,
    pub sync_ctrl: Option<GluResourceCtrlSync>,
    // pub redraw: bool,
    // pub onload: Box<dyn FnMut(HashMap<usize, CudaFrame>) -> Result<()> + Send + 'static>,
}

impl App {
    pub fn new(
        running: Arc<AtomicBool>,
        resources: Vec<Box<dyn GluResource>>,
        sync_ctrl: Option<GluResourceCtrlSync>,
    ) -> Self {
        Self {
            running,
            window: None,
            gl: None,
            surface: None,
            context: None,
            tex_rgb: vec![],
            program: None,
            vao: None,
            // frames: 0,
            // start_time: Instant::now(),
            resources,
            sync_ctrl,
            last_op: Instant::now(),
            // redraw: false,
        }
    }
    fn handle_key_press(
        &mut self,
        key_code: KeyCode,
        event_loop: &winit::event_loop::ActiveEventLoop,
    ) {
        if self.last_op.elapsed().as_secs() < 1 {
            return;
        }
        match key_code {
            // Q 键退出
            KeyCode::KeyQ => {
                log::info!("Q pressed - exiting application");
                event_loop.exit();
            }
            // ESC 键退出
            KeyCode::Escape => {
                log::info!("ESC pressed - exiting application");
                event_loop.exit();
            }
            // space 暂停
            KeyCode::Space => {
                log::info!("Space pressed - pausing/unpausing");
                if let Some(sync_ctrl) = &mut self.sync_ctrl {
                    let _ = elogger!(sync_ctrl.pause());
                }
                self.last_op = Instant::now();
            }
            _ => {}
        }
    }
}
impl ApplicationHandler<UserEvent> for App {
    /// 在windows中gl drop和windows是绑定的，所以务必确保退出时即使反注册
    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        for resource in &mut self.resources {
            let _ = elogger!(resource.unregister());
        }
    }
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let window_attrs = WindowAttributes::default()
            .with_title("NV12 CUDA/GL Demo")
            .with_inner_size(PhysicalSize::new(WIDTH, HEIGHT));

        let template = ConfigTemplateBuilder::new();
        let display_builder = DisplayBuilder::new().with_window_attributes(Some(window_attrs));

        let (window, gl_config) = display_builder
            .build(event_loop, template, |mut configs| configs.next().unwrap())
            .unwrap();

        let window = window.unwrap();

        #[allow(deprecated)]
        let context_attributes = ContextAttributesBuilder::new()
            // .with_context_api(ContextApi::Gles(Some(Version::new(3, 0))))
            .build(window.raw_window_handle().ok());

        let not_current = unsafe {
            gl_config
                .display()
                .create_context(&gl_config, &context_attributes)
                .unwrap()
        };

        let attrs = SurfaceAttributesBuilder::<WindowSurface>::new().build(
            #[allow(deprecated)]
            window.raw_window_handle().expect("oops"),
            NonZeroU32::new(WIDTH as u32).unwrap(),
            NonZeroU32::new(HEIGHT as u32).unwrap(),
        );

        let surface = unsafe {
            gl_config
                .display()
                .create_window_surface(&gl_config, &attrs)
                .unwrap()
        };
        let context = not_current.make_current(&surface).unwrap();

        surface
            .set_swap_interval(
                &context,
                // SwapInterval::Wait(NonZeroU32::new(1).unwrap()), // VSync 开启
                SwapInterval::DontWait, // 或关闭 VSync
            )
            .unwrap();

        let gl = unsafe {
            glow::Context::from_loader_function(|s| {
                gl_config
                    .display()
                    .get_proc_address(&CString::new(s).unwrap())
            })
        };

        for i in 0..self.resources.len() {
            let Some(GluResourceStatInfo { width, height, .. }) = self.resources[i].get_info()
            else {
                continue;
            };
            // ---- textures ----
            let tex_rgb = unsafe {
                let t = gl.create_texture().unwrap();
                gl.bind_texture(glow::TEXTURE_2D, Some(t));
                gl.tex_image_2d(
                    glow::TEXTURE_2D,
                    0,
                    glow::RGBA8 as i32,
                    width as i32,
                    height as i32,
                    0,
                    glow::RGBA,
                    glow::UNSIGNED_BYTE,
                    None,
                );
                gl.tex_parameter_i32(
                    glow::TEXTURE_2D,
                    glow::TEXTURE_MIN_FILTER,
                    glow::LINEAR as i32,
                );
                gl.tex_parameter_i32(
                    glow::TEXTURE_2D,
                    glow::TEXTURE_MAG_FILTER,
                    glow::LINEAR as i32,
                );
                gl.tex_parameter_i32(
                    glow::TEXTURE_2D,
                    glow::TEXTURE_WRAP_S,
                    glow::CLAMP_TO_EDGE as i32,
                );
                gl.tex_parameter_i32(
                    glow::TEXTURE_2D,
                    glow::TEXTURE_WRAP_T,
                    glow::CLAMP_TO_EDGE as i32,
                );
                t
            };
            // self.resources[i].stop();
            // println!("app registering {} {}", tex_y.0.get(), tex_uv.0.get());
            let _ = elogger!(self.resources[i].register(tex_rgb.0.get()));
            // self.resources[i].start();
            self.tex_rgb.push(tex_rgb);
            // self.tex_uv.push(tex_uv);
        }

        let program = create_program(&gl);
        // let vao = unsafe { gl.create_vertex_array().unwrap() };

        let vao = unsafe {
            let vao = gl.create_vertex_array().unwrap();
            gl.bind_vertex_array(Some(vao));
            // Vertex data: positions (x, y) and texture coordinates (u, v)
            let vertices: [f32; 24] = [
                // First triangle
                -1.0, -1.0, 0.0, 1.0, // bottom-left
                1.0, -1.0, 1.0, 1.0, // bottom-right
                -1.0, 1.0, 0.0, 0.0, // top-left
                // Second triangle
                -1.0, 1.0, 0.0, 0.0, // top-left
                1.0, -1.0, 1.0, 1.0, // bottom-right
                1.0, 1.0, 1.0, 0.0, // top-right
            ];

            // Create and fill vertex buffer
            let vbo = gl.create_buffer().unwrap();
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
            gl.buffer_data_u8_slice(
                glow::ARRAY_BUFFER,
                &vertices.align_to::<u8>().1,
                glow::STATIC_DRAW,
            );

            // Set up vertex attributes
            let stride = (4 * std::mem::size_of::<f32>()) as i32; // 4 floats per vertex

            // Position attribute (location 0)
            gl.vertex_attrib_pointer_f32(0, 2, glow::FLOAT, false, stride, 0);
            gl.enable_vertex_attrib_array(0);

            // Texture coordinate attribute (location 1)
            gl.vertex_attrib_pointer_f32(1, 2, glow::FLOAT, false, stride, 8); // 2 floats offset
            gl.enable_vertex_attrib_array(1);

            // Cleanup
            gl.bind_vertex_array(None);
            gl.bind_buffer(glow::ARRAY_BUFFER, None);

            vao
        };

        self.window = Some(window);
        self.gl = Some(gl);
        self.surface = Some(surface);
        self.context = Some(context);
        self.program = Some(program);
        self.vao = Some(vao);
        // self.cuda_y = cuda_y;
        // self.cuda_uv = cuda_uv;

        if let Some(sync_ctrl) = &mut self.sync_ctrl {
            let _ = elogger!(sync_ctrl.start());
        }
    }

    // fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: UserEvent) {
    //     match event {
    //         UserEvent::Frames(data) => {
    //             // log::debug!(
    //             //     "[debug] user event {:?}",
    //             //     data.values().find(|_| true).unwrap().idx
    //             // );
    //             let Some(sync_ctrl) = &mut self.sync_ctrl else {
    //                 return;
    //             };
    //             for i in 0..self.resources.len() {
    //                 let _ = elogger!(self.resources[i].draw(data.get(&i)));
    //             }
    //             let _ = elogger!(sync_ctrl.recycle(data));
    //             // cuda_check!(
    //             //     cudaStreamSynchronize(sync_ctrl.nppi_ctx.hStream),
    //             //     "GluResourceCtrlSync cudaStreamSynchronize Err"
    //             // );
    //             if let Some(window) = &self.window {
    //                 // self.redraw = true;
    //                 window.request_redraw();
    //             }
    //         }
    //         _ => {}
    //     }
    // }
    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _id: winit::window::WindowId,
        event: WindowEvent,
    ) {
        if !self.running.load(Ordering::SeqCst) {
            event_loop.exit();
            return;
        }

        match event {
            WindowEvent::CloseRequested => {
                event_loop.exit();
                // std::process::exit(0)
            }
            WindowEvent::KeyboardInput {
                device_id: _,
                event,
                is_synthetic: _,
            } => {
                if let KeyEvent {
                    state: ElementState::Pressed,
                    physical_key: PhysicalKey::Code(key_code),
                    ..
                } = event
                {
                    self.handle_key_press(key_code, event_loop);
                }
            }
            WindowEvent::RedrawRequested => {
                let Some(sync_ctrl) = &mut self.sync_ctrl else {
                    return;
                };
                let Some(data) = sync_ctrl.load_frames() else {
                    std::thread::sleep(Duration::from_millis(10));
                    // self.surface
                    //     .as_ref()
                    //     .unwrap()
                    //     .swap_buffers(self.context.as_ref().unwrap())
                    //     .unwrap();
                    return;
                };
                for i in 0..self.resources.len() {
                    let _ = elogger!(self.resources[i].draw(data.get(&i)));
                }
                let pts = data.iter().find(|_| true).map(|(_, v)| v.pts).unwrap();
                let _ = elogger!(sync_ctrl.recycle(data));

                let gl = self.gl.as_ref().unwrap();
                unsafe {
                    gl.clear_color(0.1, 0.1, 0.1, 1.0);
                    gl.clear(glow::COLOR_BUFFER_BIT);

                    gl.use_program(self.program);

                    if let Some(program) = self.program {
                        let loc_rgb = gl.get_uniform_location(program, "tex_rgb");
                        // let loc_uv = gl.get_uniform_location(program, "tex_uv");

                        gl.uniform_1_i32(loc_rgb.as_ref(), 0);
                        // gl.uniform_1_i32(loc_uv.as_ref(), 1);
                    }

                    // 去掉toolbar影响
                    let window = self.window.as_ref().unwrap();
                    let size = window.inner_size();
                    let inner_width = size.width as i32;
                    let inner_height = size.height as i32;

                    let matrix = (self.resources.len() as f32).sqrt().ceil() as usize;

                    let grid_cols = matrix as i32;
                    let grid_rows = matrix as i32;
                    let cell_width = inner_width / grid_cols;
                    let cell_height = inner_height / grid_rows;

                    for col in 0..grid_cols {
                        for row in 0..grid_rows {
                            let offset = (row + col * grid_cols) as usize;
                            // println!(
                            //     " sync_ctrl.resource_infos {:?} {offset}",
                            //     sync_ctrl.resource_infos.keys()
                            // );
                            let Some((w, h)) = sync_ctrl
                                .resource_infos
                                .get(&offset)
                                .map(|v| (v.width as i32, v.height as i32))
                            else {
                                // println!("oops {offset}");
                                continue;
                            };
                            // log::warn!("drawing {offset}");
                            let x = row * cell_width;
                            let y = (grid_cols - col - 1) * cell_height;
                            let width = cell_width;
                            let height = cell_height;
                            if offset < self.tex_rgb.len() {
                                // if cell_width * h > cell_height * w {
                                //     let cell_width_new = cell_height * w / h;
                                //     x += (cell_width - cell_width_new) / 2;
                                //     width = cell_width_new;
                                // } else {
                                //     let cell_height_new = cell_width * h / w;
                                //     y -= (cell_height - cell_height_new) / 2;
                                //     height = cell_height_new;
                                // }
                                gl.viewport(x, y, width, height);
                                gl.active_texture(glow::TEXTURE0);
                                gl.bind_texture(glow::TEXTURE_2D, Some(self.tex_rgb[offset]));

                                // gl.active_texture(glow::TEXTURE1);
                                // gl.bind_texture(glow::TEXTURE_2D, Some(self.tex_uv[offset]));

                                gl.bind_vertex_array(self.vao);
                                gl.draw_arrays(glow::TRIANGLES, 0, 6);
                            }
                        }
                    }
                    log::debug!("[App]drawing done for <{pts}>",);
                }

                // let _ = elogger!(sync_ctrl.recycle(frames));
                // unsafe {
                //     gl.flush();
                //     gl.finish();
                // }
                self.surface
                    .as_ref()
                    .unwrap()
                    .swap_buffers(self.context.as_ref().unwrap())
                    .unwrap();

                // self.window.as_ref().unwrap().request_redraw();
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        self.window.as_ref().unwrap().request_redraw();
    }
}

fn create_program(gl: &glow::Context) -> glow::Program {
    unsafe {
        let vs = gl.create_shader(glow::VERTEX_SHADER).unwrap();
        gl.shader_source(
            vs,
            r#"#version 300 es
precision highp float;

layout (location = 0) in vec2 a_pos;
layout (location = 1) in vec2 a_uv;

out vec2 v_uv;

void main() {
    v_uv = a_uv;
    gl_Position = vec4(a_pos, 0.0, 1.0);
}"#,
            // v_uv = vec2(a_uv.x, 1.0 - a_uv.y); // flip if needed
        );
        gl.compile_shader(vs);
        // out vec2 v_uv;

        // void main() {
        //     vec2 pos = vec2(
        //         (gl_VertexID == 2) ? 3.0 : -1.0,
        //         (gl_VertexID == 1) ? 3.0 : -1.0
        //     );
        //     v_uv = pos * 0.5 + 0.5;
        //     v_uv.y = 1.0 - v_uv.y;
        //     gl_Position = vec4(pos, 0.0, 1.0);
        // }
        let fs = gl.create_shader(glow::FRAGMENT_SHADER).unwrap();
        gl.shader_source(
            fs,
            r#"#version 300 es
precision highp float;

in vec2 v_uv;
out vec4 color;

uniform sampler2D tex_rgb;

void main() {
    vec4 rgba = texture(tex_rgb, v_uv).rgba;
    color = rgba;
    // color = vec4(rgb, 1.0);
    // color = vec4(0.0, 0.5, 1.0, 1.0); // Blue
}
        "#,
        );
        gl.compile_shader(fs);

        let program = gl.create_program().unwrap();
        gl.attach_shader(program, vs);
        gl.attach_shader(program, fs);
        gl.link_program(program);
        gl.delete_shader(vs);
        gl.delete_shader(fs);
        program
    }
}

pub trait GluResource {
    fn set_id(&mut self, id: usize);
    fn get_id(&self) -> usize;
    fn register(&mut self, tex_rgb_id: u32) -> Result<()>;
    fn unregister(&mut self) -> Result<()>;
    fn start(&mut self) -> Result<()>;
    fn stop(&self) -> Result<()>;
    fn pause(&mut self) -> Result<()>;
    fn seek(&self, timestap: i64);
    fn draw(&self, frame: Option<&CudaFrame>) -> Result<()>;
    fn get_ctrl(&self) -> Option<&GluResourceCtrl>;
    fn get_info(&self) -> Option<GluResourceStatInfo>;
    // fn set_master_ctrl(&self) -> Option<Arc<GluResourceCtrl>>;
}

#[derive(Clone)]
pub struct GluResourceCtrlSync {
    ready_sndrs: HashMap<usize, crossbeam::channel::Sender<CudaFrame>>,
    sync_rcvr: crossbeam::channel::Receiver<HashMap<usize, CudaFrame>>,
    // sync_hook: std::sync::mpsc::Sender<HashMap<usize, CudaFrame>>,
    // decode_rcvr: crossbeam::channel::Receiver<DecodedFrame>,
    stat_rcvrs: HashMap<usize, crossbeam::channel::Receiver<GluResourceStat>>,
    resource_infos: HashMap<usize, GluResourceStatInfo>,
    total_frames: usize,
    frames: usize,
    framerate: f64,
    state: Arc<RwLock<GluResourceState>>,
    // start_time: Option<Instant>,
    // pause_time: Option<Instant>,
    // previous_idx: Option<usize>,
    // nppi_ctx: NppStreamContext,
    audio_streamer: Arc<Mutex<Option<AudioCtrl<f32>>>>,
    // timeline: Arc<AtomicU64>,
    // cmd_sndr: crossbeam::channel::Sender<GluResourceCommand>,
    // state_rcvr: crossbeam::channel::Receiver<GluResourceCommand>,
}

impl GluResourceCtrlSync {
    pub fn new(
        state: Arc<RwLock<GluResourceState>>,
        resources: &Vec<Box<dyn GluResource>>,
        decode_rcvr: crossbeam::channel::Receiver<DecodedFrame>,
        audio_streamer: Arc<Mutex<Option<AudioCtrl<f32>>>>,
        // nppi_ctx: NppStreamContext,
    ) -> Result<Self> {
        let mut stat_rcvrs = HashMap::new();
        let mut resource_infos = HashMap::new();
        let mut ready_sndrs = HashMap::new();
        let mut total_frames = None;
        let mut framerate = None;
        let mut samplerate = None;
        let (sync_sndr, sync_rcvr) = crossbeam::channel::bounded(CHANNEL_SIZE);
        for v in resources {
            let id = v.get_id();
            if let Some(info) = v.get_info() {
                if total_frames.is_none() {
                    total_frames = info.frames;
                }
                if framerate.is_none()
                    && let Some(v) = info.framerate
                {
                    framerate.replace(rsmpeg::ffi::av_q2d(v));
                }
                if samplerate.is_none()
                    && let Some(sr) = info.samplerate
                {
                    samplerate.replace(sr);
                }
                resource_infos.insert(id, info);
            }
            if let Some(ctrl) = v.get_ctrl() {
                stat_rcvrs.insert(id, ctrl.stat.clone());
                ready_sndrs.insert(id, ctrl.ready_sndr.clone());
            }
        }
        // let (sync_hook, sync_hook_rcvr) = std::sync::mpsc::channel::<HashMap<usize, CudaFrame>>();
        // let decode_rcvrs = resources
        //     .iter()
        //     .map(|v| {
        //         let id = v.get_id();
        //         if let Some(info) = v.get_info() {
        //             if total_frames.is_none() {
        //                 total_frames = info.frames;
        //             }
        //             if framerate.is_none() {
        //                 framerate = info.framerate;
        //             }
        //             if samplerate.is_none()
        //                 && let Some(sr) = info.samplerate
        //             {
        //                 samplerate.replace(sr);
        //             }
        //             resource_infos.insert(id, info);
        //         }
        //         (id, v.get_ctrl())
        //     })
        //     .filter(|v| v.1.is_some())
        //     .map(|(id, v)| {
        //         let ctrl = v.unwrap();
        //         stat_rcvrs.insert(id, ctrl.stat.clone());
        //         ready_sndrs.insert(id, ctrl.ready_sndr.clone());
        //         (id, ctrl.decode_rcvr.clone())
        //     })
        //     .collect::<HashMap<usize, crossbeam::channel::Receiver<CudaFrame>>>();
        // let (vidx_sndr, vidx_rcvr) = std::sync::mpsc::channel();
        // let timeline = Arc::new(AtomicU64::new(0));
        // let timeline1 = timeline.clone();
        // let timeline2 = timeline1.clone();
        let state1 = Arc::downgrade(&state);

        let Some(framerate) = framerate else {
            return Err(anyhow!("Framerate for audio is not set"));
        };
        let frame_duration = 1000f64 / framerate;
        // let framerate1 = rsmpeg::ffi::av_q2d(framerate);
        let Some(total_frames) = total_frames else {
            return Err(anyhow!("TotalFrames for audio is not set"));
        };
        let total_duration = (total_frames as f64 * 1000f64 / framerate).ceil() as i64;
        // let mut audio_streamer = None;
        let audio_opt = audio_streamer
            .lock()
            .map_err(|e| anyhow!("{e:?}"))?
            .as_ref()
            .map(|v| (v.idx, v.sndr.clone()));
        let ready_sndrs1 = ready_sndrs.clone();
        let resource_infos1 = resource_infos.clone();
        std::thread::spawn(move || {
            let mut video_cache = HashMap::new();
            let mut keys = vec![];
            for (i, ready_sndr) in &ready_sndrs1 {
                if let Some(info) = resource_infos1.get(i) {
                    for _ in 0..CHANNEL_SIZE {
                        if let Ok(cuda_frame) =
                            elogger!(CudaFrame::new(*i, info.width, info.height))
                        {
                            let _ = ready_sndr.send_timeout(cuda_frame, Duration::from_secs(10));
                        }
                    }
                }
                video_cache.insert(*i, vec![]);
                keys.push(*i);
            }
            let mut start_time = None;
            let mut pause_time = None;
            let mut start_pts = None;
            let mut clock = None;
            let mut audio_cache = vec![];
            let mut frames = 0;
            let mut eof = vec![];

            let timeout = Duration::from_millis(if audio_opt.is_some() {
                50
            } else {
                frame_duration as u64 / 2
            });
            'outer: loop {
                let mut local_state = GluResourceState::Pause;
                // 检查状态
                if Self::peek_state(&mut local_state, &state1) {
                    break;
                }
                // 如果是暂停则进入selfspin状态，为防止过度自旋，加了一个100ms的等待
                if local_state == GluResourceState::Pause {
                    if pause_time.is_none() {
                        pause_time.replace(Instant::now());
                    }
                    std::thread::sleep(Duration::from_millis(200));
                    continue;
                }

                if start_time.is_none() {
                    start_time.replace(Instant::now());
                } else if let Some(start_time) = &mut start_time
                    && let Some(pause_time) = pause_time.take()
                {
                    *start_time += pause_time.elapsed();
                }
                let start_time1 = start_time.as_mut().unwrap();

                let all_eof = DecodedFrame::all_eof(&eof, &keys);
                // log::debug!("[debug][align] all_eof {all_eof} ",);
                let decoded_frame = if !all_eof {
                    match decode_rcvr.recv_timeout(timeout) {
                        Ok(frame) => Some(frame),
                        Err(crossbeam::channel::RecvTimeoutError::Disconnected) => {
                            continue;
                        }
                        _ => None,
                    }
                } else {
                    None
                };
                let mut force_send = false;
                match decoded_frame {
                    Some(DecodedFrame::Video(v)) => {
                        // log::debug!("[debug][align] received video idx {} pts {} ", v.idx, v.pts,);
                        if audio_opt.is_none() && start_pts.is_none() {
                            start_pts.replace(v.pts);
                        }
                        if let Some(list) = video_cache.get_mut(&v.idx) {
                            list.push(v);
                        }
                    }
                    Some(DecodedFrame::Audio(v)) => {
                        // log::debug!("[debug][align] received audio idx {} pts {} ", v.idx, v.pts,);
                        if start_pts.is_none() {
                            start_pts.replace(v.pts);
                        }
                        audio_cache.push(v);
                    }
                    Some(DecodedFrame::EOF(idx)) => {
                        log::warn!("[Align] EOF received from idx {idx}");
                        eof.push(idx)
                    }
                    None => {
                        // 如果收不到帧，音频缓存为空，视频缓存都满了,则代表视频解码卡住了，出现这种问题是因为音视频交错存放，但是时间戳却未必正确按存放次序排布导致，所以需要手动释放视频的阻塞，让音频得以继续取得
                        if audio_opt.is_some()
                            && audio_cache.len() == 0
                            && video_cache.iter().any(|(_, v)| v.len() >= CHANNEL_SIZE)
                            && all_eof
                        {
                            log::warn!(
                                "[Align] Audio starving while video full/Deadlock found, forcing clearing"
                            );
                            force_send = true;
                            // if buffer_allowed == 0 {
                            //     panic!(
                            //         "[System] Audio and Video clock gap MUST NOT be greater than {} ",
                            //         CHANNEL_SIZE * 2
                            //     );
                            // }
                            // for (i, ready_sndr) in &ready_sndrs1 {
                            //     if let Some(info) = resource_infos1.get(i) {
                            //         if let Ok(cuda_frame) =
                            //             elogger!(CudaFrame::new(*i, info.width, info.height))
                            //         {
                            //             let _ = ready_sndr
                            //                 .send_timeout(cuda_frame, Duration::from_secs(10));
                            //         }
                            //     }
                            // }
                            // buffer_allowed -= 1;
                        }
                    }
                }
                // log::debug!(
                //     "[debug][align] deciding alignment audio {:?} video {:?}",
                //     audio_cache
                //         .iter()
                //         .map(|v| (v.idx, v.pts))
                //         .collect::<Vec<_>>(),
                //     video_cache
                //         .iter()
                //         .map(|(k, v)| (*k, v.iter().map(|s| s.pts).collect::<Vec<_>>()))
                //         .collect::<HashMap<usize, Vec<i64>>>()
                // );
                if let Some((_, audio_sndr)) = &audio_opt {
                    // log::debug!("[debug][align] sending samples",);
                    if let Some(pts) = Self::collect_samples(
                        &start_time1,
                        start_pts.unwrap_or(0),
                        total_duration,
                        &mut audio_cache,
                        audio_sndr,
                    ) {
                        clock.replace(pts);
                        // log::debug!(
                        //     "[debug][align] samples pts {} local {} remain {:?} ",
                        //     pts + start_time1.elapsed().as_millis() as i64,
                        //     start_time1.elapsed().as_millis() as i64,
                        //     audio_cache.last().map(|v| v.pts)
                        // );
                    }
                }

                // log::debug!("[debug][align] collecting frames",);
                'inner: loop {
                    let collected = Self::collect_frames(
                        &start_time1,
                        start_pts.unwrap_or(0),
                        &clock,
                        total_duration,
                        &mut video_cache,
                        &ready_sndrs1,
                        force_send,
                    );
                    if collected.len() == 0 {
                        break 'inner;
                    } else {
                        // log::debug!(
                        //     "[debug][align] frames pts {} local {}",
                        //     collected[0].values().map(|s| s.pts).collect::<Vec<_>>()[0],
                        //     start_time1.elapsed().as_millis() as i64
                        // );
                    }

                    for data in collected {
                        match sync_sndr.try_send(data) {
                            Ok(_) => {
                                if frames % 100 == 0 {
                                    let fps = frames * 1000
                                        / (start_time1.elapsed().as_millis() as usize + 1);
                                    log::info!("[App] frames fps is {fps:.4}");
                                }
                                frames += 1;
                            }
                            Err(crossbeam::channel::TrySendError::Full(mut dump)) => {
                                for (i, ready_sndr) in &ready_sndrs1 {
                                    if let Some(cuda_frame) = dump.remove(i) {
                                        let _ = ready_sndr
                                            .send_timeout(cuda_frame, Duration::from_secs(10));
                                    }
                                }
                            }
                            Err(_) => {
                                break 'outer;
                            }
                        }
                    }
                }

                if DecodedFrame::all_eof(&eof, &keys)
                    && audio_cache.len() == 0
                    && video_cache.iter().any(|(_, v)| v.len() == 0)
                {
                    start_time.replace(Instant::now());
                    start_pts.take();
                    frames = 0;
                    eof.clear();
                    log::debug!("[Align] Resetting Player",);
                }
            }
            log::info!("Quitting GluResourceCtrlSync Align Thread");
        });
        // let audio_opt = if has_audio {
        //     let Some(samplerate) = samplerate else {
        //         return Err(anyhow!("SampleRate for audio is not set"));
        //     };
        //     let (sample_sndr, sample_rcvr) = std::sync::mpsc::channel();
        //     let streamer = utils::make_stream(sample_rcvr)?;
        //     streamer.stream.play()?;
        //     audio_streamer.replace(Arc::new(streamer));
        //     Some((sample_sndr, samplerate))
        // } else {
        //     None
        // };
        // std::thread::spawn(move || {
        //     let mut start_time = None;
        //     let mut pause_time = None;
        //     let mut start_pts = None;
        //     let mut frames = 0;
        //     let mut cache = vec![];
        //     let frame_duration = (1000f64 / framerate1) as u64;
        //     // let default_audio_duration = (AUDIO_SAMPLES * 1000 / *samplerate as usize / 10) * 10;
        //     'outer: loop {
        //         // log::debug!("[debug] audio spin");
        //         let mut local_state = GluResourceState::Pause;
        //         // 检查状态
        //         if Self::peek_state(&mut local_state, &state1) {
        //             break;
        //         }
        //         // 如果是暂停则进入selfspin状态，为防止过度自旋，加了一个100ms的等待
        //         if local_state == GluResourceState::Pause {
        //             if pause_time.is_none() {
        //                 pause_time.replace(Instant::now());
        //             }
        //             std::thread::sleep(Duration::from_millis(200));
        //             continue;
        //         }
        //         // 如果非stop/pause状态，则需保证start_time存在
        //         if start_time.is_none() {
        //             start_time.replace(Instant::now());
        //         } else if let Some(start_time) = &mut start_time
        //             && let Some(pause_time) = pause_time.take()
        //         {
        //             *start_time += pause_time.elapsed();
        //         }
        //         let start_time1 = start_time.as_mut().unwrap();

        //         let wait_time = if let Some((sample_sndr, samplerate)) = &audio_opt {
        //             while let Ok(v) = audio_rcvr.try_recv() {
        //                 cache.push(v);
        //             }
        //             // log::debug!(
        //             //     "[debug][audio] cache state {:?}",
        //             //     cache
        //             //         .iter()
        //             //         .map(|v| (v.pts, v.data.len()))
        //             //         .collect::<Vec<_>>()
        //             // );
        //             let default_audio_duration =
        //                 (AUDIO_SAMPLES * 1000 / *samplerate as usize / 10) * 10;
        //             let Ok(time) = Self::tick_samples(
        //                 start_time1,
        //                 &mut start_pts,
        //                 &mut cache,
        //                 &mut frames,
        //                 &sample_sndr,
        //                 default_audio_duration,
        //             ) else {
        //                 continue 'outer;
        //             };
        //             if time > 0 {
        //                 // Some(Duration::from_millis(default_audio_duration as u64))
        //                 Some(Duration::from_millis(
        //                     std::cmp::max(time, default_audio_duration) as u64,
        //                 ))
        //             } else {
        //                 None
        //             }
        //         } else {
        //             Some(Duration::from_millis(frame_duration / 2))
        //         };

        //         Self::tick_frames(*start_time1, &mut frames, &vidx_sndr, framerate1);

        //         if let Some(t) = wait_time {
        //             // cache.iter().map(|v| v.pts).collect()
        //             // log::debug!("[Audio] sync waiting {}ms", t.as_millis(),);
        //             std::thread::sleep(t);
        //         }
        //     }
        //     log::info!("Quitting GluResourceCtrlSync Audio Thread");
        // });
        // if has_audio {
        //     let Some(_samplerate) = samplerate else {
        //         return Err(anyhow!("SampleRate for audio is not set"));
        //     };
        //     let (sample_sndr, sample_rcvr) = std::sync::mpsc::channel();
        //     let streamer = utils::make_stream(sample_rcvr)?;
        //     streamer.play()?;
        //     audio_streamer.replace(Arc::new(streamer));

        //     std::thread::spawn(move || {
        //         let mut start_time = None;
        //         let mut pause_time = None;
        //         let mut frames = 0;
        //         let mut cache = vec![];
        //         'outer: loop {
        //             let mut local_state = GluResourceState::Pause;
        //             // 检查状态
        //             if Self::peek_state(&mut local_state, &state1) {
        //                 break;
        //             }
        //             // 如果是暂停则进入selfspin状态，为防止过度自旋，加了一个100ms的等待
        //             if local_state == GluResourceState::Pause {
        //                 if pause_time.is_none() {
        //                     pause_time.replace(Instant::now());
        //                 }
        //                 std::thread::sleep(Duration::from_millis(100));
        //                 continue;
        //             }
        //             // 如果非stop/pause状态，则需保证start_time存在
        //             if start_time.is_none() {
        //                 start_time.replace(Instant::now());
        //             } else if let Some(start_time) = &mut start_time
        //                 && let Some(pause_time) = pause_time.take()
        //             {
        //                 *start_time += pause_time.elapsed();
        //             }
        //             let start_time1 = start_time.as_mut().unwrap();

        //             while let Ok(v) = audio_rcvr.try_recv() {
        //                 cache.push(v);
        //             }

        //             let Ok(should_wait) = Self::tick_samples(start_time1, &mut cache, &sample_sndr)
        //             else {
        //                 continue 'outer;
        //             };
        //             Self::tick_frames(
        //                 *start_time1,
        //                 &mut frames,
        //                 &vidx_sndr,
        //                 total_frames,
        //                 framerate1,
        //             );
        //             if should_wait {
        //                 std::thread::sleep(Duration::from_millis(20));
        //             }
        //         }
        //         log::info!("Quitting GluResourceCtrlSync Audio Thread");
        //     });
        // } else {
        //     std::thread::spawn(move || {
        //         let mut start_time = None;
        //         let mut pause_time = None;
        //         let mut frames = 0;
        //         let frame_duration = (1000f64 / framerate1) as u64;
        //         loop {
        //             let mut local_state = GluResourceState::Pause;
        //             // 检查状态
        //             if Self::peek_state(&mut local_state, &state1) {
        //                 break;
        //             }
        //             // 如果是暂停则进入selfspin状态，为防止过度自旋，加了一个100ms的等待
        //             if local_state == GluResourceState::Pause {
        //                 if pause_time.is_none() {
        //                     pause_time.replace(Instant::now());
        //                 }
        //                 std::thread::sleep(Duration::from_millis(100));
        //                 continue;
        //             }
        //             // 如果非stop/pause状态，则需保证start_time存在
        //             if start_time.is_none() {
        //                 start_time.replace(Instant::now());
        //             } else if let Some(start_time) = &mut start_time
        //                 && let Some(pause_time) = pause_time.take()
        //             {
        //                 *start_time += pause_time.elapsed();
        //             }
        //             let start_time1 = start_time.unwrap();
        //             Self::tick_frames(
        //                 start_time1,
        //                 &mut frames,
        //                 &vidx_sndr,
        //                 total_frames,
        //                 framerate1,
        //             );
        //             std::thread::sleep(Duration::from_millis(frame_duration / 2));
        //         }
        //         log::info!("Quitting GluResourceCtrlSync Sync Thread");
        //     });
        // }

        // let ready_sndrs1 = ready_sndrs.clone();
        // let decode_rcvrs1 = decode_rcvrs.clone();
        // let resource_infos1 = resource_infos.clone();
        // // let nppi_ctx1 = Arc::new(nppi_ctx);
        // std::thread::spawn(move || {
        //     let frame_duration = (1000f64 / framerate1) as u64;
        //     // 预存cuda_frame，用于打开解码线程的阻塞器开关，让CHANNEL_SIZE左右个帧加入到缓存队列中
        //     for (i, ready_sndr) in &ready_sndrs1 {
        //         if let Some(info) = resource_infos1.get(&i) {
        //             for _ in 0..CHANNEL_SIZE {
        //                 if let Ok(cuda_frame) = elogger!(CudaFrame::new(info.width, info.height)) {
        //                     let _ = ready_sndr.send_timeout(cuda_frame, Duration::from_secs(10));
        //                 }
        //             }
        //         }
        //     }
        //     let mut datas: HashMap<usize, Vec<CudaFrame>> = HashMap::new();
        //     let mut local_state = GluResourceState::Pause;
        //     // let mut previous_idx = None;
        //     let start_time = Instant::now();
        //     let mut frames = 0;
        //     'outer: loop {
        //         // log::debug!("[debug] video spin");
        //         // let timeout = Duration::from_secs(20);
        //         // 检查状态位
        //         if Self::peek_state(&mut local_state, &state2) {
        //             break;
        //         }
        //         // 确保回收队列为空，便于释放解码器线程的阻塞状态
        //         // while let Ok(recyled) = sync_hook_rcvr.try_recv() {
        //         //     for (i, cuda_frame) in recyled.into_iter() {
        //         //         if let Some(ready_sndr) = ready_sndrs1.get(&i) {
        //         //             let _ = ready_sndr.send_timeout(cuda_frame, Duration::from_secs(10));
        //         //         }
        //         //     }
        //         // }
        //         // 尝试从解码线程中获取帧数据，确保所有解码数据进入缓存
        //         // log::debug!("[App] receiving local_state {local_state:?}");
        //         for (i, decode_rcvr) in &decode_rcvrs1 {
        //             while let Ok(v) = decode_rcvr.try_recv() {
        //                 if let Some(list) = datas.get_mut(i) {
        //                     list.push(v);
        //                 } else {
        //                     datas.insert(*i, vec![v]);
        //                 }
        //             }
        //         }
        //         // log::debug!("[App][Video]peeking {previous_idx:?}",);
        //         // let vidx_opt = if previous_idx.is_some() {
        //         //     previous_idx
        //         // } else {

        //         // };
        //         let is_ready = datas.iter().any(|(_, v)| v.len() > CHANNEL_SIZE / 2);
        //         // 尝试获取同步指令，如果所有视频的缓存都大于一半以上，说明当前处理较快，可以阻塞性等待同步指令
        //         let vidx_opt = if is_ready {
        //             match vidx_rcvr.recv_timeout(Duration::from_secs(1)) {
        //                 Ok(v) => Some(v),
        //                 Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
        //                     continue;
        //                 }
        //                 _ => None,
        //             }
        //         } else {
        //             match vidx_rcvr.try_recv() {
        //                 Ok(v) => Some(v),
        //                 Err(std::sync::mpsc::TryRecvError::Disconnected) => {
        //                     continue;
        //                 }
        //                 _ => None,
        //             }
        //         };
        //         // log::debug!("[debug] validating datas datas {vidx_opt:?} {datas:?}");
        //         if datas.is_empty() || datas.values().find(|v| v.len() == 0).is_some() {
        //             std::thread::sleep(Duration::from_millis(frame_duration / 2));

        //             continue;
        //         }
        //         let Some(indexer) = vidx_opt else {
        //             std::thread::sleep(Duration::from_millis(frame_duration / 2));
        //             continue;
        //         };
        //         // log::debug!(
        //         //     "[App][Video]Index {} received pts {} datas {datas:?}",
        //         //     indexer.index,
        //         //     indexer.pts
        //         // );

        //         let collected =
        //             Self::collect_frames(indexer, total_frames, &mut datas, &ready_sndrs1);
        //         if collected.len() == 0 {
        //             // 如果收集不到数据且最早的idx晚于当前indexer，说明缓存数据尚不
        //             // if let Some(min_idx) = Self::earlist_idx(total_frames, &datas) {
        //             //     if indexer.earlier(&min_idx, total_frames) {
        //             //         previous_idx.replace(indexer);
        //             //     }
        //             // }
        //             std::thread::sleep(Duration::from_millis(frame_duration / 2));
        //             continue;
        //         }
        //         // previous_idx.take();
        //         // log::debug!(
        //         //     "[App][Video]Index {} received pts {} collected {:?}",
        //         //     indexer.index,
        //         //     indexer.pts,
        //         //     collected
        //         // );
        //         // 发送同步队列前，确保所有cuda操作都结束，阻塞行为
        //         // cuda_check!(
        //         //     cudaStreamSynchronize(nppi_ctx1.hStream),
        //         //     "GluResourceCtrlSync cudaStreamSynchronize Err"
        //         // );
        //         for data in collected {
        //             // if elogger!(onload(data)).is_err() {
        //             //     break 'outer;
        //             // }
        //             match sync_sndr.try_send(data) {
        //                 Ok(_) => {}
        //                 Err(crossbeam::channel::TrySendError::Full(mut dump)) => {
        //                     for (i, ready_sndr) in &ready_sndrs1 {
        //                         if let Some(cuda_frame) = dump.remove(i) {
        //                             let _ = ready_sndr
        //                                 .send_timeout(cuda_frame, Duration::from_secs(10));
        //                         }
        //                     }
        //                 }
        //                 Err(_) => {
        //                     break 'outer;
        //                 }
        //             }
        //         }
        //         if frames % 100 == 0 {
        //             let fps = frames * 1000 / start_time.elapsed().as_millis() as usize;
        //             log::info!("[App] frames fps is {fps:.4}");
        //         }
        //         frames += 1;

        //         // for (i, syn_sndr) in &syn_sndrs {
        //         //     'inner1: loop {
        //         //         if Self::peek_state(&mut local_state, &state1) {
        //         //             break 'outer;
        //         //         }
        //         //         if local_state != GluResourceState::Pause {
        //         //             break 'inner1;
        //         //         }
        //         //         std::thread::sleep(Duration::from_millis(100));
        //         //     }
        //         //     if let Some(cuda_frame) = datas.remove(&i) {
        //         //         if elogger!(syn_sndr.send_timeout(cuda_frame, timeout)).is_err() {
        //         //             break 'outer;
        //         //         }
        //         //     }
        //         // }
        //     }

        //     log::info!("Quitting GluResourceCtrlSync Video Thread");
        // });

        Ok(Self {
            ready_sndrs,
            // decode_rcvr,
            sync_rcvr,
            // sync_hook,
            stat_rcvrs,
            resource_infos,
            state,
            total_frames,
            framerate,
            frames: 0,
            // timeline,
            audio_streamer,
            // nppi_ctx,
            // master,
        })
    }

    /// 检查状态，非阻塞模式读
    fn peek_state(
        local_state: &mut GluResourceState,
        state: &Weak<RwLock<GluResourceState>>,
    ) -> bool {
        let Some(state) = state.upgrade() else {
            return true;
        };
        if let Ok(v) = state.try_read() {
            *local_state = *v;
        }
        let watch_stat = match *local_state {
            GluResourceState::Stop => true,
            GluResourceState::Start => false,
            GluResourceState::Pause => false,
            // GluResourceCommand::Seek(timestamp) => {
            //     let seek_time = convert_stream_time_to_av_time_base(timestamp);
            //     self.input_ctx
            //         .seek(-1, seek_time, rsmpeg::ffi::AVSEEK_FLAG_BACKWARD as i32)?;
            //     self.dec_ctx.flush_buffers();
            //     true
            // }
        };
        watch_stat
    }
    fn earlier(pts: i64, other: i64, total_duration: i64) -> bool {
        let gap = total_duration / 2;
        if ((pts - other).abs() as usize) < gap as usize {
            // return self.index < other.index && self.pts > other.pts;
            return pts < other;
        }
        pts > other + total_duration
    }
    fn collect_samples(
        start_time: &Instant,
        start_pts: i64,
        total_duration: i64,
        audio_cache: &mut Vec<AudioFrame>,
        audio_sndr: &std::sync::mpsc::Sender<Vec<f32>>,
    ) -> Option<i64> {
        let local_pts = start_time.elapsed().as_millis() as i64;
        let mut clock_drift = None;
        while audio_cache.len() > 0 {
            let audio_data = &audio_cache[0];
            let actual_pts = audio_data.pts - start_pts;
            // log::debug!(
            //     "[debug] collect_samples local_pts {local_pts} {} {start_pts} total_duration {total_duration} earlier {}",
            //     audio_data.pts,
            //     Self::earlier(local_pts, actual_pts, total_duration)
            // );
            if !Self::earlier(local_pts, actual_pts, total_duration) {
                let samples = bytemuck::cast_slice::<u8, f32>(&audio_data.data)
                    .into_iter()
                    .map(|v| *v)
                    .collect::<Vec<f32>>();
                log::debug!(
                    "[Audio] Queuing audio pts {actual_pts} duration {} ",
                    audio_data.dur
                );
                let _ = elogger!(audio_sndr.send(samples));
                clock_drift.replace(audio_data.pts - local_pts);
                audio_cache.remove(0);
            } else {
                break;
            }
        }
        clock_drift
    }
    fn collect_frames(
        start_time: &Instant,
        start_pts: i64,
        _clock_drift: &Option<i64>,
        total_duration: i64,
        video_cache: &mut HashMap<usize, Vec<CudaFrame>>,
        ready_sndrs: &HashMap<usize, crossbeam::channel::Sender<CudaFrame>>,
        force_send: bool,
    ) -> Vec<HashMap<usize, CudaFrame>> {
        let mut datas = vec![];
        let nb_frames = ready_sndrs.len();
        loop {
            if video_cache.len() != nb_frames {
                break;
            }
            let Some((pts, _idx)) = Self::earlist_pts(total_duration, video_cache) else {
                break;
            };
            // log::debug!("[debug][align] earliest pts {pts}",);
            if pts < start_pts {
                panic!("[System] Start PTS {start_pts} is later than Norminated PTS {pts}");
            }
            // let local_pts = clock_drift.unwrap_or(0) + start_time.elapsed().as_millis() as i64;
            let local_pts = start_time.elapsed().as_millis() as i64;
            if !force_send && Self::earlier(local_pts, pts - start_pts, total_duration) {
                break;
            }
            let mut items = HashMap::new();
            for (k, cuda_frames) in video_cache.iter_mut() {
                // 这里认为所有的cuda_frame idx都不为空
                let cuda_frame_pts = cuda_frames[0].pts;
                // 如果当前帧和最早帧号不匹配，该帧一定晚于最早帧号，所以会被忽略等待下一次轮训
                if cuda_frame_pts == pts {
                    items.insert(*k, cuda_frames.remove(0));
                }
            }
            let items_len = items.len();
            // 如果收集齐了，则放入返回队列，否则直接回收
            if items_len == nb_frames {
                datas.push(items);
            } else {
                for (k, v) in items {
                    if let Some(ready_sndr) = ready_sndrs.get(&k) {
                        let _ = ready_sndr.send_timeout(v, Duration::from_secs(10));
                    }
                }
                if items_len == 0 {
                    break;
                }
            }
            // if force_send && datas.len() >= CHANNEL_SIZE / 2 {
            //     force_send = false;
            // }
        }
        datas
    }
    /// 处理一片音频，如果处理时时间轴>
    // fn process_samples(
    //     start_time: Instant,
    //     audio_data: Option<&GluAudioData>,
    //     sample_sndr: &std::sync::mpsc::Sender<Vec<f32>>,
    //     // index: &Arc<AtomicU64>,
    //     frames: &mut usize,
    //     vidx_sndr: &std::sync::mpsc::Sender<GluVideoIndex>,
    //     total_frames: usize,
    //     framerate: f64,
    // ) -> Result<bool> {
    //     let local_pts = start_time.elapsed().as_millis() as i64;
    //     if let Some(ad) = audio_data {
    //         if local_pts < ad.pts {
    //             // std::thread::sleep(Duration::from_millis((ad.pts - local_pts) as u64 / 2));
    //             return Ok(false);
    //         }
    //         // let frame_duration = (1000f64 / framerate) as u64;
    //         let samples = bytemuck::cast_slice::<u8, f32>(&ad.data)
    //             .into_iter()
    //             .map(|v| *v)
    //             .collect::<Vec<f32>>();
    //         let _ = elogger!(sample_sndr.send(samples))?;
    //     }
    //     let should_play_idx = std::cmp::max(
    //         OrderedFloat(local_pts as f64 * framerate / 1000f64),
    //         OrderedFloat(0f64),
    //     )
    //     .round() as usize
    //         % total_frames;
    //     if should_play_idx != *frames {
    //         // log::debug!(
    //         //     "[debug][align]sending {} idx {should_play_idx} {local_pts}",
    //         //     audio_data.is_some()
    //         // );
    //         let _ = vidx_sndr.send(GluVideoIndex::new(should_play_idx, local_pts));
    //         *frames = should_play_idx;
    //         return Ok(true);
    //     }
    //     Ok(false)
    // }

    /// 处理一片音频，如果处理时时间轴>
    // fn tick_samples(
    //     start_time: &mut Instant,
    //     start_pts: &mut Option<i64>,
    //     cache: &mut Vec<GluAudioData>,
    //     frames: &mut usize,
    //     sample_sndr: &std::sync::mpsc::Sender<Vec<f32>>,
    //     default_duration: usize,
    // ) -> Result<usize> {
    //     // let mut should_wait = false;
    //     let local_pts = start_time.elapsed().as_millis() as i64;
    //     if cache.len() == 0 {
    //         // log::debug!("[debug][audio] empty cache",);
    //         return Ok(default_duration);
    //     }
    //     if cache[0].data.len() == 0 {
    //         *start_time = Instant::now();
    //         // log::debug!(
    //         //     "[debug][audio] resetting audio time cache {:?}",
    //         //     cache
    //         //         .iter()
    //         //         .map(|v| (v.pts, v.data.len()))
    //         //         .collect::<Vec<_>>()
    //         // );
    //         cache.remove(0);
    //         start_pts.take();
    //         *frames = 0;
    //         return Ok(0);
    //     }

    //     // 尽可能发送更多的采样
    //     while cache.len() > 0 {
    //         let audio_data = &cache[0];
    //         if start_pts.is_none() {
    //             start_pts.replace(audio_data.pts);
    //         }
    //         let actual_pts = audio_data.pts - start_pts.unwrap_or(0);
    //         if local_pts >= actual_pts {
    //             let samples = bytemuck::cast_slice::<u8, f32>(&audio_data.data)
    //                 .into_iter()
    //                 .map(|v| *v)
    //                 .collect::<Vec<f32>>();
    //             let _ = elogger!(sample_sndr.send(samples))?;
    //             // log::debug!(
    //             //     "[debug][audio] local_pts {local_pts} actual_pts {actual_pts}  audio_data.pts {} start_pts {:?}",
    //             //     audio_data.pts,
    //             //     start_pts
    //             // );
    //             cache.remove(0);
    //         } else {
    //             // log::debug!("[debug][audio] waiting {local_pts} actual_pts {actual_pts}",);
    //             return Ok((actual_pts - local_pts) as usize);
    //         }
    //     }
    //     Ok(0)
    // }
    // /// 处理一片音频，如果处理时时间轴>
    // fn tick_frames(
    //     start_time: Instant,
    //     // index: &Arc<AtomicU64>,
    //     frames: &mut usize,
    //     vidx_sndr: &std::sync::mpsc::Sender<GluVideoIndex>,
    //     framerate: f64,
    // ) {
    //     let local_pts = start_time.elapsed().as_millis() as i64;
    //     // let factor = if *frames < 20 { 10f64 } else { 1f64 };
    //     let should_play_idx = std::cmp::max(
    //         OrderedFloat(local_pts as f64 * framerate / 1000f64),
    //         OrderedFloat(0f64),
    //     )
    //     .round() as usize;

    //     // let mut wait_time = None;
    //     // let idx = GluVideoIndex::new(should_play_idx, local_pts);
    //     // if let Ok(idx) = elogger!(idx.pack_compact()) {
    //     //     let current = index.load(Ordering::Relaxed);
    //     //     if current != idx {
    //     //         index.store(idx, Ordering::Release);
    //     //         // let mut gap = (current as i128 - idx as i128).abs() as u64;
    //     //         // 环形，如果是边缘处，则有如下特征
    //     //         // if gap > total_frames as u64 / 2 {
    //     //         //     gap = total_frames as u64 - gap;
    //     //         // }
    //     //         // wait_time.replace(Duration::from_millis(gap));
    //     //     }
    //     // }
    //     if should_play_idx != *frames {
    //         // let idx = GluVideoIndex::new(should_play_idx, local_pts);
    //         // if let Ok(idx) = elogger!(idx.pack_compact()) {
    //         //     index.store(idx, Ordering::Relaxed);
    //         // }
    //         // index.store(idx.pack_compact(), Ordering::Relaxed);
    //         let _ = vidx_sndr.send(GluVideoIndex::new(should_play_idx, local_pts));
    //         // log::debug!("[debug][video] local_pts {local_pts} should_play_idx {should_play_idx}",);
    //         *frames = should_play_idx;
    //         // log::debug!(
    //         //     "[debug]tick_frameing {local_pts} framerate {framerate} total_frames {total_frames} should_play_idx {should_play_idx}",
    //         // );
    //     }
    //     // let wait_time = wait_time.unwrap_or_else(|| {
    //     //     let frame_duration = (1000f64 / framerate) as u64;
    //     //     Duration::from_millis(frame_duration / 2)
    //     // });
    //     // if should_play_idx > *frames {
    //     //     if let Ok(idx) = elogger!(idx.pack_compact()) {
    //     //         index.compare_exchange(*frames as u64, idx, Ordering::Relaxed);
    //     //     }
    //     //     *frames = should_play_idx;
    //     //     std::thread::sleep(Duration::from_millis(10));
    //     // } else {
    //     //     let frame_duration = (1000f64 / framerate) as u64;
    //     //     std::thread::sleep(Duration::from_millis(frame_duration / 2));
    //     //     return Ok(false);
    //     // }
    //     // Ok(true)
    // }

    /// 尝试获取最早的帧并对齐，直到找到idx位置的帧
    // fn collect_frames1(
    //     indexer: GluVideoIndex,
    //     total_frames: usize,
    //     cache: &mut HashMap<usize, Vec<CudaFrame>>,
    //     ready_sndrs: &HashMap<usize, crossbeam::channel::Sender<CudaFrame>>,
    // ) -> Vec<HashMap<usize, CudaFrame>> {
    //     let mut datas = vec![];
    //     let nb_frames = ready_sndrs.len();
    //     loop {
    //         if cache.len() != nb_frames {
    //             break;
    //         }
    //         let Some(min_idx) = Self::earlist_idx(total_frames, cache) else {
    //             break;
    //         };
    //         // if cache.values().find(|v| v.len() == 0).is_some() {
    //         //     break;
    //         // }
    //         // let order_idx = cache
    //         //     .values()
    //         //     .filter(|v| v.len() > 0 && v[0].idx.is_some())
    //         //     .map(|v| v[0].idx.unwrap())
    //         //     .map(|v| {
    //         //         let new_value = if v.index < total_frames / 2 {
    //         //             v.index + total_frames / 2
    //         //         } else {
    //         //             v.index
    //         //         };
    //         //         (new_value, v)
    //         //     })
    //         //     .collect::<Vec<(usize, GluVideoIndex)>>();
    //         // let (_, min_idx) = order_idx.into_iter().min_by(|a, b| a.0.cmp(&b.0)).unwrap();
    //         // log::debug!("[debug] comparing indexer {indexer:?} vs min_idx {min_idx:?} ");
    //         if indexer.earlier(&min_idx, total_frames) && min_idx.index > 1 {
    //             break;
    //         }
    //         // log::debug!("[debug] indexer {indexer:?} later than min_idx {min_idx:?} ");
    //         if cache.iter().any(|(_, v)| {
    //             let cuda_frame_idx = v[0].idx.unwrap();
    //             cuda_frame_idx.index == min_idx.index
    //         }) {
    //             let mut items = HashMap::new();
    //             for (k, cuda_frames) in cache.iter_mut() {
    //                 // 这里认为所有的cuda_frame idx都不为空
    //                 let cuda_frame_idx = cuda_frames[0].idx.unwrap();
    //                 // 如果当前帧和最早帧号不匹配，该帧一定晚于最早帧号，所以会被忽略等待下一次轮训
    //                 if cuda_frame_idx.index == min_idx.index {
    //                     items.insert(*k, cuda_frames.remove(0));
    //                 }
    //             }
    //             let items_len = items.len();
    //             // 如果收集齐了，则放入返回队列，否则直接回收
    //             if items_len == nb_frames {
    //                 datas.push(items);
    //             } else {
    //                 for (k, v) in items {
    //                     if let Some(ready_sndr) = ready_sndrs.get(&k) {
    //                         let _ = ready_sndr.send_timeout(v, Duration::from_secs(10));
    //                     }
    //                 }
    //                 if items_len == 0 {
    //                     break;
    //                 }
    //             }
    //         }
    //         // for (k, cuda_frames) in cache.iter_mut() {
    //         //     // 这里认为所有的cuda_frame idx都不为空
    //         //     let cuda_frame_idx = cuda_frames[0].idx.unwrap();
    //         //     // 如果当前帧和最早帧号不匹配，该帧一定晚于最早帧号，所以会被忽略等待下一次轮训
    //         //     if cuda_frame_idx.index == min_idx.index {
    //         //         items.insert(*k, cuda_frames.remove(0));
    //         //     }
    //         //     // else if let Some(ready_sndr) = ready_sndrs.get(k) {
    //         //     //     let _ = ready_sndr.send_timeout(cuda_frame, Duration::from_secs(10));
    //         //     // }
    //         // }
    //     }
    //     datas

    //     // // 取得滑动窗口内最早的帧号，默认视频不能太小，多个视频帧收集上来时，窗口不会相差1/2个total_frames，以这个为前提，获取1/2个total_frames内最小的那个值
    //     // let order_idx = idxs
    //     //     .iter()
    //     //     .map(|v| {
    //     //         let new_value = if *v < total_frames / 2 {
    //     //             *v + total_frames / 2
    //     //         } else {
    //     //             *v
    //     //         };
    //     //         (new_value, *v)
    //     //     })
    //     //     .collect::<Vec<(usize, usize)>>();
    //     // let (_, mut min_idx) = order_idx.into_iter().min_by(|a, b| a.0.cmp(&b.0)).unwrap();
    //     // let mut datas = vec![];
    //     // let nb_frames = cache.len();
    //     // loop {
    //     //     if indexer < min_idx {
    //     //         break;
    //     //     }
    //     //     // 如果同步帧号 晚于 最早帧号，表示解码帧已经
    //     //     // if GluVideoIndex::earlier(indexer.index, min_idx, total_frames) {
    //     //     //     break;
    //     //     // }
    //     //     let mut items = HashMap::new();
    //     //     for (k, v) in cache.iter_mut() {
    //     //         let cuda_frame = v.remove(0);
    //     //         // 如果当前帧和最小帧号部匹配，则直接回收，否则放入缓存队列
    //     //         if cuda_frame.idx.unwrap_or(0) == min_idx {
    //     //             items.insert(*k, cuda_frame);
    //     //         } else if let Some(ready_sndr) = ready_sndrs.get(k) {
    //     //             let _ = ready_sndr.send_timeout(cuda_frame, Duration::from_secs(10));
    //     //         }
    //     //     }
    //     //     // 如果收集齐了，则放入返回队列，否则回收
    //     //     if items.len() == nb_frames {
    //     //         datas.push(items);
    //     //     } else {
    //     //         for (k, v) in items {
    //     //             if let Some(ready_sndr) = ready_sndrs.get(&k) {
    //     //                 let _ = ready_sndr.send_timeout(v, Duration::from_secs(10));
    //     //             }
    //     //         }
    //     //         min_idx += 1;
    //     //     }
    //     // }
    //     // datas
    //     // 投票加权找到多数的帧号
    //     // let mut voter = HashMap::new();
    //     // for idx in &idxs {
    //     //     if let Some(v) = voter.get_mut(idx) {
    //     //         *v += 1;
    //     //     } else {
    //     //         voter.insert(*idx, 1);
    //     //     }
    //     // }
    //     // let most_votes = voter.values().map(|v| *v).max().unwrap_or(0);
    //     // // 如果投票帧的票数为1表示所有帧号都不相同，也就是乱序了，这时就取最小的
    //     // let voted_idx = if most_votes == 1 {
    //     //     let order_idx = idxs
    //     //         .iter()
    //     //         .map(|v| {
    //     //             let new_value = if *v < total_frames / 2 {
    //     //                 *v + total_frames / 2
    //     //             } else {
    //     //                 *v
    //     //             };
    //     //             (new_value, *v)
    //     //         })
    //     //         .collect::<Vec<(usize, usize)>>();
    //     //     let (_, min_idx) = order_idx.into_iter().min_by(|a, b| a.0.cmp(&b.0)).unwrap();
    //     //     min_idx
    //     // } else {
    //     //     voter
    //     //         .into_iter()
    //     //         .find(|(_, v)| *v == most_votes)
    //     //         .map(|(k, _)| k)
    //     //         .unwrap()
    //     // };

    //     // for (i, list) in cache {
    //     //     // let min_idx = list.iter().fold((usize::MAX, 0), |(min, max), f| {
    //     //     //     (
    //     //     //         std::cmp::min(min, f.idx.unwrap_or(usize::MAX)),
    //     //     //         std::cmp::min(v, f.idx.unwrap_or(usize::MAX)),
    //     //     //     )
    //     //     // });
    //     // }
    // }
    // /// 获取缓存中最早的帧号
    // fn earlist_idx(
    //     total_frames: usize,
    //     cache: &HashMap<usize, Vec<CudaFrame>>,
    // ) -> Option<GluVideoIndex> {
    //     if cache.is_empty() || cache.values().find(|v| v.len() == 0).is_some() {
    //         return None;
    //     }
    //     let order_idx = cache
    //         .values()
    //         .filter(|v| v.len() > 0 && v[0].idx.is_some())
    //         .map(|v| v[0].idx.unwrap())
    //         .map(|v| {
    //             let new_value = if v.index < total_frames / 2 {
    //                 v.index + total_frames / 2
    //             } else {
    //                 v.index
    //             };
    //             (new_value, v)
    //         })
    //         .collect::<Vec<(usize, GluVideoIndex)>>();
    //     let (_, min_idx) = order_idx.into_iter().min_by(|a, b| a.0.cmp(&b.0)).unwrap();
    //     Some(min_idx)
    // }
    fn earlist_pts(
        total_duration: i64,
        video_cache: &HashMap<usize, Vec<CudaFrame>>,
    ) -> Option<(i64, usize)> {
        if video_cache.is_empty() || video_cache.values().find(|v| v.len() == 0).is_some() {
            return None;
        }
        let order_pts = video_cache
            .iter()
            .filter(|(_, v)| v.len() > 0 && v[0].pts >= 0)
            .map(|(k, v)| {
                let new_value = if v[0].pts < total_duration / 2 {
                    v[0].pts + total_duration / 2
                } else {
                    v[0].pts
                };
                (new_value, v[0].pts, *k)
            })
            .collect::<Vec<(i64, i64, usize)>>();
        order_pts
            .into_iter()
            .min_by(|a, b| a.0.cmp(&b.0))
            .map(|(_, pts, idx)| (pts, idx))
    }
    pub fn load_frames(&self) -> Option<HashMap<usize, CudaFrame>> {
        self.sync_rcvr.try_recv().ok()
    }
    // pub fn recv(&mut self) -> Result<HashMap<usize, CudaFrame>> {
    //     if self.pause_time.is_some() {
    //         std::thread::sleep(Duration::from_millis(10));
    //         return Err(anyhow!("app pausing"));
    //     }
    //     let user_start = self
    //         .start_time
    //         .as_ref()
    //         .map(|v| v.elapsed().as_millis())
    //         .ok_or(anyhow!("start time missing"))?;
    //     let user_pause = self
    //         .pause_time
    //         .as_ref()
    //         .map(|v| v.elapsed().as_millis())
    //         .unwrap_or(0);
    //     let user_pts = user_start - user_pause;
    //     let should_play_idx = (user_pts as f64 * utils::av_q2d(self.framerate) / 1000f64) as usize;
    //     // self.frames += 1;
    //     if should_play_idx < self.frames {
    //         return Err(anyhow!("frames too far ahead, skipping"));
    //     }
    //     if self.frames < CHANNEL_SIZE {
    //         for (i, ready_sndr) in &self.ready_sndrs {
    //             if let Some(info) = self.resource_infos.get(&i) {
    //                 let _ = ready_sndr.send_timeout(
    //                     CudaFrame::new(info.width, info.height)?,
    //                     Duration::from_secs(10),
    //                 );
    //             }
    //         }
    //     }
    //     if self.frames % 100 == 0 && f64::MAX as usize > self.frames && user_pts > 0 {
    //         let fps = self.frames as f64 * 1000f64 / user_pts as f64;
    //         log::info!("[App] frames {} fps is {fps:.4}", self.frames);
    //     }
    //     // Ok(self.sync_rcvr.try_recv()?)
    //     let mut datas = HashMap::new();
    //     // let mut test_idx = None;
    //     for (i, decode_rcvr) in &self.decode_rcvrs {
    //         let cuda_frame = decode_rcvr.recv_timeout(Duration::from_secs(10))?;
    //         datas.insert(*i, cuda_frame);
    //     }
    //     Ok(datas)
    // }

    pub fn recycle(&mut self, mut frames: HashMap<usize, CudaFrame>) -> Result<()> {
        for (i, ready_sndr) in &self.ready_sndrs {
            if let Some(cuda_frame) = frames.remove(&i) {
                let _ = ready_sndr.send_timeout(cuda_frame, Duration::from_secs(10))?;
            }
        }
        self.frames += 1;
        Ok(())
    }
    //  if let Some(previous_idx) = &mut self.previous_idx {
    //                     *previous_idx += 1;
    //                 } else {
    //                     self.previous_idx.replace(0);
    //                 }
    // pub fn recycle(&self, mut cuda_frame: CudaFrame) {
    //     cuda_frame.idx.take();
    //     for (_, ready_sndr) in &self.ready_sndrs {
    //         if ready_sndr.try_send(cuda_frame.clone()).is_ok() {
    //             return;
    //         }
    //     }
    // }
    pub fn stop(&mut self) -> Result<()> {
        // for cmd_sndr in &self.cmd_sndrs {
        //     let _ = cmd_sndr.send_timeout(GluResourceCommand::Stop, Duration::from_secs(10))?;
        // }
        // let _ = self
        //     .cmd_sndr
        //     .send_timeout(GluResourceCommand::Stop, Duration::from_secs(10))?;
        let mut state = self.state.write().map_err(|e| anyhow!("state err {e:?}"))?;
        *state = GluResourceState::Stop;
        // self.sync_rcvrs.clear();
        Ok(())
    }
    pub fn terminate(self) -> Result<()> {
        //         ready_sndrs: HashMap<usize, crossbeam::channel::Sender<CudaFrame>>,
        // sync_rcvr: crossbeam::channel::Receiver<HashMap<usize, CudaFrame>>,
        // stat_rcvrs: HashMap<usize, crossbeam::channel::Receiver<GluResourceStat>>,
        // resource_infos: HashMap<usize, GluResourceStatInfo>,
        // total_frames: usize,
        // frames: usize,
        // framerate: AVRational,
        // state: Arc<RwLock<GluResourceState>>,
        // start_time: Option<Instant>,
        // pause_time: Option<Instant>,
        // previous_idx: Option<usize>,

        // self.ready_sndrs.clear();
        // self.resource_infos.clear();
        // self.state.clear();
        let stat_rcvrs = self.stat_rcvrs.clone();
        drop(self);
        for (_, stat_rcvr) in stat_rcvrs {
            let _ = stat_rcvr.recv_timeout(Duration::from_secs(10));
        }
        std::thread::sleep(Duration::from_millis(60));
        Ok(())
    }
    pub fn start(&mut self) -> Result<()> {
        // let _ = self
        //     .cmd_sndr
        //     .send_timeout(GluResourceCommand::Start, Duration::from_secs(10))?;
        let mut state = self.state.write().map_err(|e| anyhow!("state err {e:?}"))?;
        *state = GluResourceState::Start;
        // self.pause_time.take();
        // self.start_time.replace(Instant::now());
        // self.previous_idx.take();
        // for cmd_sndr in &self.cmd_sndrs {
        //     let _ = cmd_sndr.send_timeout(GluResourceCommand::Start, Duration::from_secs(10))?;
        // }
        Ok(())
    }
    pub fn pause(&mut self) -> Result<()> {
        // log::debug!("[App] pausing start");
        let mut state = self.state.write().map_err(|e| anyhow!("state err {e:?}"))?;
        if *state == GluResourceState::Start {
            // let _ = self
            //     .cmd_sndr
            //     .send_timeout(GluResourceCommand::Pause, Duration::from_secs(10))?;
            // let mut state = self.state.write().map_err(|e| anyhow!("state err {e:?}"))?;
            *state = GluResourceState::Pause;
            // self.pause_time.replace(Instant::now());
        } else if *state == GluResourceState::Pause {
            // let _ = self
            //     .cmd_sndr
            //     .send_timeout(GluResourceCommand::Start, Duration::from_secs(10))?;
            // let mut state = self.state.write().map_err(|e| anyhow!("state err {e:?}"))?;
            *state = GluResourceState::Start;
            // if let Some(start_time) = &mut self.start_time {
            //     *start_time += self.pause_time.take().unwrap().elapsed();
            // }
        }
        // log::debug!("[App] pausing done");
        // for cmd_sndr in &self.cmd_sndrs {
        //     let _ = cmd_sndr.send_timeout(GluResourceCommand::Pause, Duration::from_secs(10))?;
        // }
        Ok(())
    }
}

#[derive(Clone)]
pub struct GluResourceCtrl {
    pub state: Weak<RwLock<GluResourceState>>,
    // pub decode_rcvr: crossbeam::channel::Receiver<CudaFrame>,
    pub ready_sndr: crossbeam::channel::Sender<CudaFrame>,
    pub stat: crossbeam::channel::Receiver<GluResourceStat>,
    // slaves: Vec<GluResourceCtrl>,
}
impl GluResourceCtrl {
    pub fn new(
        // command: &tokio::sync::broadcast::Sender<GluResourceCommand>,
        state: &Arc<RwLock<GluResourceState>>,
        // decode_rcvr: crossbeam::channel::Receiver<CudaFrame>,
        ready_sndr: crossbeam::channel::Sender<CudaFrame>,
        stat: crossbeam::channel::Receiver<GluResourceStat>,
    ) -> Self {
        // for _ in 0..CHANNEL_SIZE {
        //     let _ = ready_sndr.send(CudaFrame::new(width, height)?);
        // }
        Self {
            // command: command.clone(),
            state: Arc::downgrade(state),
            // decode_rcvr,
            ready_sndr,
            stat,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum GluResourceCommand {
    Stop,
    Start,
    Pause,
    Seek(i64),
}

impl GluResourceCommand {
    pub fn stop() -> Self {
        Self::Stop
    }
    pub fn start() -> Self {
        Self::Start
    }
    pub fn pause() -> Self {
        Self::Pause
    }
    pub fn seek(timestamp: i64) -> Self {
        Self::Seek(timestamp)
    }
}

pub enum DecodedFrame {
    Video(CudaFrame),
    Audio(AudioFrame),
    EOF(usize),
}
impl DecodedFrame {
    pub fn all_eof(eof: &[usize], keys: &[usize]) -> bool {
        // log::debug!("[debug][align] all_eof=> {eof:?} keys=> {keys:?} ",);
        keys.len() == eof.len() && keys.iter().any(|v| eof.contains(v))
    }
}
#[derive(Clone, Debug)]
pub struct AudioFrame {
    pub idx: usize,
    pub pts: i64,
    pub data: Bytes,
    pub dur: i64,
}
impl AudioFrame {
    pub fn new(idx: usize, pts: i64, dur: i64, data: &[u8]) -> Self {
        Self {
            idx,
            pts,
            dur,
            data: Bytes::copy_from_slice(data),
        }
    }
}

/// RGB frame
#[derive(Clone, Debug)]
pub struct CudaFrame {
    // pub idx: Option<GluVideoIndex>,
    pub idx: usize,
    pub pts: i64,
    pub cache_rgba: *mut std::ffi::c_void,
    // pub cache_uv: *mut std::ffi::c_void,
    pub width: usize,
    pub height: usize,
}
impl CudaFrame {
    pub fn new(idx: usize, width: usize, height: usize) -> Result<Self> {
        let chan_size = width * height;
        let mut cache_rgba = null_mut();
        // let mut cache_uv = null_mut();
        cuda_error!(cudaMalloc(&mut cache_rgba, chan_size * 4))?;
        // cuda_error!(cudaMalloc(&mut cache_uv, chan_size / 2))?;
        cuda_error!(cudaMemset2D(cache_rgba.offset(3), 4, 1, 1, height * width))?;
        // let mut cache_uv = null_mut();
        // cuda_error!(cudaMalloc(&mut cache_uv, chan_size / 2))?;
        Ok(Self {
            idx,
            pts: -1,
            cache_rgba,
            width,
            height,
        })
    }
}
impl Drop for CudaFrame {
    fn drop(&mut self) {
        // println!("dropping frame {:?}", self.idx);
        if !self.cache_rgba.is_null() {
            cuda_check!(cudaFree(self.cache_rgba), "cudaFree RGB");
        }
        // if !self.cache_y.is_null() {
        //     cuda_check!(cudaFree(self.cache_y), "cudaFree UV");
        // }
        // if !self.cache_uv.is_null() {
        //     cuda_check!(cudaFree(self.cache_uv), "cudaFree UV");
        // }
        // println!("dropped frame {:?}", self.idx);
    }
}
unsafe impl Send for CudaFrame {}
unsafe impl Sync for CudaFrame {}

pub enum GluResourceStat {
    Info(GluResourceStatInfo),
    Code(i32),
    Err(anyhow::Error),
}

#[derive(Clone, Default)]
pub struct GluResourceStatInfo {
    pub width: usize,
    pub height: usize,
    pub framerate: Option<rsmpeg::ffi::AVRational>,
    pub pix_fmt: Option<rsmpeg::ffi::AVPixelFormat>,
    pub video_duration: Option<i64>,
    pub audio_duration: Option<i64>,
    pub frames: Option<usize>,
    pub samplerate: Option<i32>,
    pub channels: Option<i32>,
    pub sample_fmt: Option<i32>,
}
impl GluResourceStatInfo {
    pub fn new(
        width: usize,
        height: usize,
        framerate: Option<rsmpeg::ffi::AVRational>,
        pix_fmt: Option<rsmpeg::ffi::AVPixelFormat>,
        video_duration: Option<i64>,
        audio_duration: Option<i64>,
        frames: Option<usize>,
        samplerate: Option<i32>,
        channels: Option<i32>,
        sample_fmt: Option<i32>,
    ) -> Self {
        Self {
            width,
            height,
            framerate,
            pix_fmt,
            video_duration,
            audio_duration,
            frames,
            samplerate,
            channels,
            sample_fmt,
        }
    }
}

#[derive(PartialEq, Debug, Clone, Copy)]
pub enum GluResourceState {
    Start,
    Stop,
    Pause,
}

// #[derive(Clone, Copy, Debug, Default)]
// pub struct GluVideoIndex {
//     pub index: usize,
//     pub pts: i64,
// }
// impl GluVideoIndex {
//     pub fn new(index: usize, pts: i64) -> Self {
//         Self { index, pts }
//     }
//     fn earlier(&self, other: &Self, total_frames: usize) -> bool {
//         let gap = total_frames / 2;
//         if ((self.index as i64 - other.index as i64).abs() as usize) < gap {
//             // return self.index < other.index && self.pts > other.pts;
//             return self.pts < other.pts;
//         }
//         self.pts > other.pts
//     }
//     // pub fn pack_compact(&self) -> Result<u64> {
//     //     let idx = self.index;
//     //     let value = self.pts;
//     //     if idx >= 0x7FFF_FFFF {
//     //         return Err(anyhow!("idx must fit in 31 bits"));
//     //     }
//     //     if value <= -0x100_000_000 || value >= 0x100_000_000 {
//     //         return Err(anyhow!("value must fit in 32 bits"));
//     //     }

//     //     // 提取符号 (1=负, 0=正或0)
//     //     let sign_bit = if value < 0 { 1 } else { 0 };

//     //     // 取绝对值
//     //     let abs_value = value.abs() as u32;

//     //     // 打包：idx(31位) | sign(1位) | value(32位)
//     //     let packed = ((idx as u64 & 0x7FFF_FFFF) << 33) |  // idx 占位 33-63位
//     //         ((sign_bit as u64) << 32) |           // sign 占位 32位
//     //         (abs_value as u64); // value 占位 0-31位
//     //     Ok(packed)
//     // }

//     // pub fn unpack_compact(packed: u64) -> Self {
//     //     // 提取各部分
//     //     let idx = ((packed >> 33) & 0x7FFF_FFFF) as u32; // 31位
//     //     let sign_bit = ((packed >> 32) & 1) as u32; // 1位
//     //     let abs_value = (packed & 0xFF_FFF_FFF) as u32; // 32位

//     //     // 根据符号恢复有符号值
//     //     let value = if sign_bit == 1 {
//     //         -(abs_value as i64)
//     //     } else {
//     //         abs_value as i64
//     //     };

//     //     Self {
//     //         index: idx,
//     //         pts: value,
//     //     }
//     // }
// }
// impl PartialOrd for GluVideoIndex {
//     fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
//         self.pts.partial_cmp(&other.pts)
//     }
// }
// impl PartialEq for GluVideoIndex {
//     fn eq(&self, other: &Self) -> bool {
//         self.index == other.index && self.pts == other.pts
//     }
// }

#[derive(Clone)]
pub enum UserEvent {
    Dummy,
    Frames(HashMap<usize, CudaFrame>),
}

// #[derive(Clone, Default)]
// pub struct GluVideoFramesEvent {
//     frames: HashMap<usize, CudaFrame>,
// }
// impl GluVideoFramesEvent {}
