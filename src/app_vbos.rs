use crate::cuda::{cudaFree, cudaMalloc, cudaMemset2D};
use crate::utils::{self, CHANNEL_SIZE};
use crate::{cuda_check, cuda_error, elogger};
use anyhow::{Result, anyhow};
use glow::HasContext;
use glutin::config::ConfigTemplateBuilder;
use glutin::context::{ContextAttributesBuilder, PossiblyCurrentContext};
use glutin::display::GetGlDisplay;
use glutin::prelude::*;
use glutin::surface::{Surface, SurfaceAttributesBuilder, SwapInterval, WindowSurface};
use glutin_winit::DisplayBuilder;
use rsmpeg::ffi::AVRational;
use std::collections::HashMap;
use std::f64;
use std::ffi::CString;
use std::num::NonZeroU32;
use std::ptr::null_mut;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock, Weak};
use std::time::{Duration, Instant};
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
    pub vaos: Vec<glow::NativeVertexArray>,

    pub last_op: Instant,
    pub sync_ctrl: Option<GluResourceCtrlSync>,
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
            vaos: vec![],
            // frames: 0,
            // start_time: Instant::now(),
            resources,
            sync_ctrl,
            last_op: Instant::now(),
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
impl ApplicationHandler for App {
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

        let matrix = std::cmp::max((self.resources.len() as f32).sqrt().ceil() as usize, 1);

        let grid_cols = matrix as i32;
        let grid_rows = matrix as i32;
        let portion = 2f32 / matrix as f32;
        for row in 0..grid_rows {
            for col in 0..grid_cols {
                let offset = (row + col) as usize;
                if offset < self.resources.len() {
                    // let x = col as f32* portion;
                    // let y = row as f32* portion;
                    let bl_x = -1f32 + col as f32 * portion;
                    let bl_y = -1f32 + (grid_rows - row - 1) as f32 * portion;

                    let tr_x = bl_x + portion;
                    let tr_y = bl_y + portion;
                    unsafe {
                        let vao = gl.create_vertex_array().unwrap();
                        gl.bind_vertex_array(Some(vao));
                        let vertices: [f32; 24] = [
                            // First triangle
                            bl_x, bl_y, 0.0, 1.0, // bottom-left
                            tr_x, bl_y, 1.0, 1.0, // bottom-right
                            bl_x, tr_y, 0.0, 0.0, // top-left
                            // Second triangle
                            bl_x, tr_y, 0.0, 0.0, // top-left
                            tr_x, bl_y, 1.0, 1.0, // bottom-right
                            tr_x, tr_y, 1.0, 0.0, // top-right
                        ];
                        // println!("vertices {vertices:?} for id {offset} portion {portion}");
                        // Vertex data: positions (x, y) and texture coordinates (u, v)
                        // let vertices: [f32; 24] = [
                        //     // First triangle
                        //     -1.0, -1.0, 0.0, 1.0, // bottom-left
                        //     1.0, -1.0, 1.0, 1.0, // bottom-right
                        //     -1.0, 1.0, 0.0, 0.0, // top-left
                        //     // Second triangle
                        //     -1.0, 1.0, 0.0, 0.0, // top-left
                        //     1.0, -1.0, 1.0, 1.0, // bottom-right
                        //     1.0, 1.0, 1.0, 0.0, // top-right
                        // ];
                        //    [-1.0, 0.0, 0.0, 1.0,
                        //     0.0, 0.0, 1.0, 1.0,
                        //     -1.0, 1.0, 0.0, 0.0,
                        //     -1.0, 1.0, 0.0, 0.0,
                        //      0.0, 0.0, 1.0, 1.0,
                        //      0.0, 1.0, 1.0, 0.0]

                        // Create and fill vertex buffer
                        let vbo = gl.create_buffer().unwrap();
                        gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
                        gl.buffer_data_u8_slice(
                            glow::ARRAY_BUFFER,
                            &vertices
                                // .iter()
                                // .map(|v| v / 2f32)
                                // .collect::<Vec<f32>>()
                                .align_to::<u8>()
                                .1,
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

                        self.vaos.push(vao);
                    };
                }
            }
        }

        // for i in 0..self.resources.len() {

        // }

        self.window = Some(window);
        self.gl = Some(gl);
        self.surface = Some(surface);
        self.context = Some(context);
        self.program = Some(program);
        // self.vaos = ;
        // self.cuda_y = cuda_y;
        // self.cuda_uv = cuda_uv;

        if let Some(sync_ctrl) = &mut self.sync_ctrl {
            let _ = elogger!(sync_ctrl.start());
        }
    }

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
                let Ok(frames) = sync_ctrl.recv() else {
                    return;
                };
                let fidx = frames
                    .values()
                    .map(|v| v.idx)
                    .filter(|v| v.is_some())
                    .map(|v| v.unwrap())
                    .collect::<Vec<usize>>();
                log::debug!("[App]received {fidx:?}");
                let gl = self.gl.as_ref().unwrap();
                for i in 0..self.resources.len() {
                    unsafe {
                        // upload Y
                        gl.bind_texture(glow::TEXTURE_2D, Some(self.tex_rgb[i]));
                        // upload UV
                        // gl.bind_texture(glow::TEXTURE_2D, Some(self.tex_uv[i]));
                        let _ = elogger!(self.resources[i].draw(frames.get(&i)));
                    }
                }
                unsafe {
                    gl.flush();
                    gl.finish();
                }
                // ---- render ----
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

                    for i in 0..self.resources.len() {
                        gl.active_texture(glow::TEXTURE0);
                        gl.bind_texture(glow::TEXTURE_2D, Some(self.tex_rgb[i]));

                        // gl.active_texture(glow::TEXTURE1);
                        // gl.bind_texture(glow::TEXTURE_2D, Some(self.tex_uv[offset]));

                        gl.bind_vertex_array(Some(self.vaos[i]));
                        gl.draw_arrays(glow::TRIANGLES, 0, 6);
                    }
                    gl.flush();

                    // let matrix =
                    //     std::cmp::max((self.resources.len() as f32).log2().ceil() as usize, 1);

                    // let grid_cols = matrix as i32;
                    // let grid_rows = matrix as i32;
                    // let cell_width = inner_width / grid_cols;
                    // let cell_height = inner_height / grid_rows;

                    // for col in 0..grid_cols {
                    //     for row in 0..grid_rows {
                    //         let offset = (row + col * grid_cols) as usize;
                    //         // log::warn!("drawing {offset}");
                    //         if offset < self.tex_rgb.len() {
                    //             gl.viewport(
                    //                 row * cell_width,
                    //                 (grid_cols - col - 1) * cell_height,
                    //                 cell_width,
                    //                 cell_height,
                    //             );

                    //         }
                    //     }
                    // }
                    log::debug!("[App]drawing done {fidx:?}");
                }
                let _ = elogger!(sync_ctrl.recycle(frames));
                unsafe {
                    gl.finish();
                }
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
    fn draw(&mut self, frame: Option<&CudaFrame>) -> Result<()>;
    fn get_ctrl(&self) -> Option<&GluResourceCtrl>;
    fn get_info(&self) -> Option<GluResourceStatInfo>;
    // fn set_master_ctrl(&self) -> Option<Arc<GluResourceCtrl>>;
}

#[derive(Clone)]
pub struct GluResourceCtrlSync {
    ready_sndrs: HashMap<usize, crossbeam::channel::Sender<CudaFrame>>,
    sync_rcvr: crossbeam::channel::Receiver<HashMap<usize, CudaFrame>>,
    stat_rcvrs: HashMap<usize, crossbeam::channel::Receiver<GluResourceStat>>,
    resource_infos: HashMap<usize, GluResourceStatInfo>,
    total_frames: usize,
    frames: usize,
    framerate: AVRational,
    state: Arc<RwLock<GluResourceState>>,
    start_time: Option<Instant>,
    pause_time: Option<Instant>,
    previous_idx: Option<usize>,
    // cmd_rcvr: crossbeam::channel::Receiver<GluResourceCommand>,
}

impl GluResourceCtrlSync {
    pub fn new(
        state: Arc<RwLock<GluResourceState>>,
        resources: &Vec<Box<dyn GluResource>>,
    ) -> Self {
        // let mut syn_sndrs = HashMap::new();
        // let mut sync_rcvrs = HashMap::new();
        let mut stat_rcvrs = HashMap::new();
        let mut resource_infos = HashMap::new();
        let (sync_sndr, sync_rcvr) = crossbeam::channel::bounded(CHANNEL_SIZE);
        let mut ready_sndrs = HashMap::new();
        // for resource in resources {
        //     let (sync_sndr, sync_rcvr) = crossbeam::channel::bounded(VIEW_PORTS);
        //     let id = resource.get_id();
        //     syn_sndrs.insert(id, sync_sndr);
        //     sync_rcvrs.insert(id, sync_rcvr);
        // }
        let mut total_frames = None;
        let mut framerate = None;
        let decode_rcvrs = resources
            .iter()
            .map(|v| {
                let id = v.get_id();
                if let Some(info) = v.get_info() {
                    if total_frames.is_none() {
                        total_frames = info.frames;
                    }
                    if framerate.is_none() {
                        framerate = info.framerate;
                    }
                    resource_infos.insert(id, info);
                }
                (id, v.get_ctrl())
            })
            .filter(|v| v.1.is_some())
            .map(|(id, v)| {
                let ctrl = v.unwrap();
                // let (sync_sndr, sync_rcvr) = crossbeam::channel::bounded(CHANNEL_SIZE);
                // syn_sndrs.insert(id, sync_sndr);
                // sync_rcvrs.insert(id, sync_rcvr);
                stat_rcvrs.insert(id, ctrl.stat.clone());
                ready_sndrs.insert(id, ctrl.ready_sndr.clone());
                (id, ctrl.decode_rcvr.clone())
            })
            .collect::<HashMap<usize, crossbeam::channel::Receiver<CudaFrame>>>();
        let state1 = Arc::downgrade(&state);
        std::thread::spawn(move || {
            'outer: loop {
                let timeout = Duration::from_secs(20);
                let mut datas = HashMap::new();
                let mut test_idx = None;
                let mut local_state = GluResourceState::Pause;
                if Self::peek_state(&mut local_state, &state1) {
                    break;
                }
                if local_state == GluResourceState::Pause {
                    std::thread::sleep(Duration::from_millis(100));
                    continue;
                }
                // log::debug!("[App] receiving local_state {local_state:?}");
                for (i, decode_rcvr) in &decode_rcvrs {
                    'inner: loop {
                        if Self::peek_state(&mut local_state, &state1) {
                            break 'outer;
                        }
                        if local_state != GluResourceState::Pause {
                            break 'inner;
                        }
                        std::thread::sleep(Duration::from_millis(100));
                    }
                    'inner1: loop {
                        match decode_rcvr.recv_timeout(Duration::from_secs(1)) {
                            Ok(v) => {
                                log::debug!("[App]received {:?} for {}", v.idx, i);
                                if let Some(tidx) = &test_idx {
                                    if v.idx.as_ref().map(|s| *s != *tidx).unwrap_or(false) {
                                        log::error!(
                                            "[App]id {i} idx mismatch from {tidx} vs {:?}",
                                            v.idx
                                        );
                                    }
                                } else if let Some(idx) = v.idx {
                                    test_idx.replace(idx);
                                }
                                datas.insert(*i, v);
                                break 'inner1;
                            }
                            Err(crossbeam::channel::RecvTimeoutError::Timeout) => {
                                if Self::peek_state(&mut local_state, &state1) {
                                    break 'outer;
                                }
                                continue 'inner1;
                                // if local_state != GluResourceState::Pause {
                                //     break 'inner1;
                                // }
                                // std::thread::sleep(Duration::from_millis(100));
                            }
                            _ => {
                                break 'outer;
                            }
                        }
                    }
                    // if let Ok(v) = elogger!(decode_rcvr.recv_timeout(timeout)) {
                    // } else {
                    //     break 'outer;
                    // }
                }
                log::debug!("[App]sending {test_idx:?}");
                if elogger!(sync_sndr.send_timeout(datas, timeout)).is_err() {
                    break 'outer;
                }
                // for (i, syn_sndr) in &syn_sndrs {
                //     'inner1: loop {
                //         if Self::peek_state(&mut local_state, &state1) {
                //             break 'outer;
                //         }
                //         if local_state != GluResourceState::Pause {
                //             break 'inner1;
                //         }
                //         std::thread::sleep(Duration::from_millis(100));
                //     }
                //     if let Some(cuda_frame) = datas.remove(&i) {
                //         if elogger!(syn_sndr.send_timeout(cuda_frame, timeout)).is_err() {
                //             break 'outer;
                //         }
                //     }
                // }
            }

            log::info!("Quitting GluResourceCtrlSync thread");
        });
        Self {
            ready_sndrs,
            sync_rcvr,
            stat_rcvrs,
            resource_infos,
            state,
            start_time: None,
            pause_time: None,
            previous_idx: None, // cmd_rcvr, // ready_sndrs,
            total_frames: total_frames.unwrap_or(0),
            framerate: framerate.unwrap_or(AVRational { num: 1, den: 1 }),
            frames: 0,
        }
    }

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
    // pub fn recv(&self, id: usize) -> Result<CudaFrame> {
    //     let sync_rcvr = self
    //         .sync_rcvrs
    //         .get(&id)
    //         .ok_or(anyhow!("no such sync_rcvr"))?;
    //     let cufa_frame = sync_rcvr.recv_timeout(Duration::from_secs(10))?;
    //     Ok(cufa_frame)
    // }
    pub fn recv(&mut self) -> Result<HashMap<usize, CudaFrame>> {
        if self.pause_time.is_some() {
            std::thread::sleep(Duration::from_millis(10));
            return Err(anyhow!("app pausing"));
        }
        let user_start = self
            .start_time
            .as_ref()
            .map(|v| v.elapsed().as_millis())
            .ok_or(anyhow!("start time missing"))?;
        let user_pause = self
            .pause_time
            .as_ref()
            .map(|v| v.elapsed().as_millis())
            .unwrap_or(0);
        let user_pts = user_start - user_pause;
        let should_play_idx = (user_pts as f64 * utils::av_q2d(self.framerate) / 1000f64) as usize;
        // self.frames += 1;
        if should_play_idx < self.frames {
            return Err(anyhow!("frames too far ahead, skipping"));
        }
        if self.frames < CHANNEL_SIZE {
            for (i, ready_sndr) in &self.ready_sndrs {
                if let Some(info) = self.resource_infos.get(&i) {
                    let _ = ready_sndr.send_timeout(
                        CudaFrame::new(None, info.width, info.height)?,
                        Duration::from_secs(10),
                    );
                }
            }
        }
        if self.frames % 100 == 0 && f64::MAX as usize > self.frames && user_pts > 0 {
            let fps = self.frames as f64 * 1000f64 / user_pts as f64;
            log::info!("[App] frames {} fps is {fps:.4}", self.frames);
        }
        Ok(self.sync_rcvr.recv_timeout(Duration::from_secs(10))?)
    }
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
        Ok(())
    }
    pub fn start(&mut self) -> Result<()> {
        // let _ = self
        //     .cmd_sndr
        //     .send_timeout(GluResourceCommand::Start, Duration::from_secs(10))?;
        let mut state = self.state.write().map_err(|e| anyhow!("state err {e:?}"))?;
        *state = GluResourceState::Start;
        self.pause_time.take();
        self.start_time.replace(Instant::now());
        self.previous_idx.take();
        // for cmd_sndr in &self.cmd_sndrs {
        //     let _ = cmd_sndr.send_timeout(GluResourceCommand::Start, Duration::from_secs(10))?;
        // }
        Ok(())
    }
    pub fn pause(&mut self) -> Result<()> {
        if self.pause_time.is_none() {
            // let _ = self
            //     .cmd_sndr
            //     .send_timeout(GluResourceCommand::Pause, Duration::from_secs(10))?;
            let mut state = self.state.write().map_err(|e| anyhow!("state err {e:?}"))?;
            *state = GluResourceState::Pause;
            self.pause_time.replace(Instant::now());
        } else {
            let mut state = self.state.write().map_err(|e| anyhow!("state err {e:?}"))?;
            *state = GluResourceState::Start;
            if let Some(start_time) = &mut self.start_time {
                *start_time += self.pause_time.take().unwrap().elapsed();
            }
        }
        // for cmd_sndr in &self.cmd_sndrs {
        //     let _ = cmd_sndr.send_timeout(GluResourceCommand::Pause, Duration::from_secs(10))?;
        // }
        Ok(())
    }
}

#[derive(Clone)]
pub struct GluResourceCtrl {
    pub state: Weak<RwLock<GluResourceState>>,
    pub decode_rcvr: crossbeam::channel::Receiver<CudaFrame>,
    pub ready_sndr: crossbeam::channel::Sender<CudaFrame>,
    pub stat: crossbeam::channel::Receiver<GluResourceStat>,
    // slaves: Vec<GluResourceCtrl>,
}
impl GluResourceCtrl {
    pub fn new(
        // command: &tokio::sync::broadcast::Sender<GluResourceCommand>,
        state: &Arc<RwLock<GluResourceState>>,
        decode_rcvr: crossbeam::channel::Receiver<CudaFrame>,
        ready_sndr: crossbeam::channel::Sender<CudaFrame>,
        stat: crossbeam::channel::Receiver<GluResourceStat>,
    ) -> Self {
        // for _ in 0..CHANNEL_SIZE {
        //     let _ = ready_sndr.send(CudaFrame::new(width, height)?);
        // }
        Self {
            // command: command.clone(),
            state: Arc::downgrade(state),
            decode_rcvr,
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

/// RGB frame
#[derive(Clone)]
pub struct CudaFrame {
    pub idx: Option<usize>,
    pub cache_rgba: *mut std::ffi::c_void,
    // pub cache_uv: *mut std::ffi::c_void,
    pub width: usize,
    pub height: usize,
}
impl CudaFrame {
    pub fn new(idx: Option<usize>, width: usize, height: usize) -> Result<Self> {
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
            cache_rgba,
            // cache_uv,
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
    pub duration: Option<i64>,
    pub frames: Option<usize>,
}
impl GluResourceStatInfo {
    pub fn new(
        width: usize,
        height: usize,
        framerate: Option<rsmpeg::ffi::AVRational>,
        pix_fmt: Option<rsmpeg::ffi::AVPixelFormat>,
        duration: Option<i64>,
        frames: Option<usize>,
    ) -> Self {
        Self {
            width,
            height,
            framerate,
            pix_fmt,
            duration,
            frames,
        }
    }
}

#[derive(PartialEq, Debug, Clone, Copy)]
pub enum GluResourceState {
    Start,
    Stop,
    Pause,
}
