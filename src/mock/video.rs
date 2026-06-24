use crate::api::UnityParam;
use crate::utils;
use anyhow::{Result, anyhow};
use glow::HasContext;
use glutin::config::ConfigTemplateBuilder;
use glutin::context::{ContextAttributesBuilder, PossiblyCurrentContext};
use glutin::display::GetGlDisplay;
use glutin::prelude::*;
use glutin::surface::{Surface, SurfaceAttributesBuilder, SwapInterval, WindowSurface};
use glutin_winit::DisplayBuilder;
use std::ffi::CString;
use std::num::NonZeroU32;
use std::path::PathBuf;
use std::ptr::null;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
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
pub struct MockVideoApp {
    pub running: Arc<AtomicBool>,
    pub window: Option<Window>,
    pub gl: Option<glow::Context>,
    pub surface: Option<Surface<WindowSurface>>,
    pub context: Option<PossiblyCurrentContext>,

    pub tex_rgb: Vec<glow::NativeTexture>,
    pub program: Option<glow::Program>,
    pub vao: Option<glow::NativeVertexArray>,

    pub last_op: Instant,
    pub player: *const std::ffi::c_void,
    view_ports: usize,

    get_ratios: extern "C" fn(
        widths: *mut std::ffi::c_int,
        heights: *mut std::ffi::c_int,
        length: std::ffi::c_int,
        player: *const std::ffi::c_void,
    ) -> std::ffi::c_int,
    register_textures: extern "C" fn(_evt_id: std::ffi::c_int, param: *mut std::ffi::c_void),
    unregister_textures: extern "C" fn(_evt_id: std::ffi::c_int, param: *mut std::ffi::c_void),
    start_player: extern "C" fn(player: *const std::ffi::c_void) -> std::ffi::c_int,
    pause_player: extern "C" fn(player: *const std::ffi::c_void) -> std::ffi::c_int,
    load_frames: extern "C" fn(player: *const std::ffi::c_void) -> std::ffi::c_int,
    draw_frames: extern "C" fn(_evt_id: std::ffi::c_int, param: *mut std::ffi::c_void),
    // recycle_frames: extern "C" fn(player: *const std::ffi::c_void) -> std::ffi::c_int,
}

impl MockVideoApp {
    pub fn new(
        running: Arc<AtomicBool>,
        player: *const std::ffi::c_void,
        view_ports: usize,
        get_ratios: extern "C" fn(
            widths: *mut std::ffi::c_int,
            heights: *mut std::ffi::c_int,
            length: std::ffi::c_int,
            player: *const std::ffi::c_void,
        ) -> std::ffi::c_int,
        register_textures: extern "C" fn(_evt_id: std::ffi::c_int, param: *mut std::ffi::c_void),
        unregister_textures: extern "C" fn(_evt_id: std::ffi::c_int, param: *mut std::ffi::c_void),
        start_player: extern "C" fn(player: *const std::ffi::c_void) -> std::ffi::c_int,
        pause_player: extern "C" fn(player: *const std::ffi::c_void) -> std::ffi::c_int,
        load_frames: extern "C" fn(player: *const std::ffi::c_void) -> std::ffi::c_int,
        draw_frames: extern "C" fn(_evt_id: std::ffi::c_int, param: *mut std::ffi::c_void),
        // recycle_frames: extern "C" fn(player: *const std::ffi::c_void) -> std::ffi::c_int,
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
            player,
            view_ports,
            last_op: Instant::now(),
            get_ratios,
            register_textures,
            unregister_textures,
            start_player,
            pause_player,
            load_frames,
            draw_frames,
            // recycle_frames,
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
                let code = (self.pause_player)(self.player);
                if code < 0 {
                    panic!("[Mock][KeyboardInput] pause failed {code}");
                }
                self.last_op = Instant::now();
            }
            _ => {}
        }
    }
}
impl ApplicationHandler for MockVideoApp {
    /// 在windows中gl drop和windows是绑定的，所以务必确保退出时即使反注册
    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        let mut param = UnityParam {
            texture_rgba_ids: null(),
            length: 0,
            handle: self.player,
            code: -1,
        };
        (self.unregister_textures)(0, &mut param as *mut _ as *mut std::ffi::c_void);
        if param.code < 0 {
            panic!("[Mock][exiting]unregister_textures failed {}", param.code);
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

        let mut widths = vec![0; self.view_ports];
        let mut heights = vec![0; self.view_ports];

        // let widths1: Vec<*mut i32> = widths.iter_mut().map(|v| v as *mut i32).collect();
        // let heights1: Vec<*mut i32> = heights.iter_mut().map(|v| v as *mut i32).collect();
        let mut code = (self.get_ratios)(
            widths.as_mut_ptr(),
            heights.as_mut_ptr(),
            self.view_ports as i32,
            self.player,
        );
        // println!("[debug] {widths:?} {heights:?}");
        if code < 0 {
            panic!("[Mock][Resume]get_ratios failed {code}");
        }
        let mut tex_rgbs = vec![];
        let mut texture_rgba_ids = vec![];
        for i in 0..self.view_ports {
            let width = widths[i];
            let height = heights[i];
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
            texture_rgba_ids.push(tex_rgb.0.get());
            tex_rgbs.push(tex_rgb);
        }

        let mut param = UnityParam {
            texture_rgba_ids: texture_rgba_ids.as_ptr(),
            length: texture_rgba_ids.len() as i32,
            handle: self.player,
            code: -1,
        };
        (self.register_textures)(0, &mut param as *mut _ as *mut std::ffi::c_void);
        // code = (self.register_textures)(
        //     texture_rgba_ids.as_ptr(),
        //     self.view_ports as i32,
        //     self.player,
        // );
        if param.code < 0 {
            panic!("[Mock][Resume]register_textures failed {}", param.code);
        }
        self.tex_rgb.append(&mut tex_rgbs);

        let program = create_program(&gl);

        let vao = unsafe {
            let vao = gl.create_vertex_array().unwrap();
            gl.bind_vertex_array(Some(vao));
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

        code = (self.start_player)(self.player);
        if code < 0 {
            panic!("[Mock][Resume]start_player failed {code}");
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
                if self.player.is_null() {
                    return;
                }
                let code = (self.load_frames)(self.player);
                if code <= 0 {
                    return;
                }
                let gl = self.gl.as_ref().unwrap();
                let mut param = UnityParam {
                    texture_rgba_ids: null(),
                    length: 0,
                    handle: self.player,
                    code: -1,
                };
                (self.draw_frames)(0, &mut param as *mut _ as *mut std::ffi::c_void);
                if param.code < 0 {
                    panic!("[Mock][Redraw]draw_frame failed {}", param.code);
                }
                // for i in 0..self.view_ports {
                //     unsafe {
                //         gl.bind_texture(glow::TEXTURE_2D, Some(self.tex_rgb[i]));
                //     }
                // }
                // ---- render ----
                unsafe {
                    gl.clear_color(0.1, 0.1, 0.1, 1.0);
                    gl.clear(glow::COLOR_BUFFER_BIT);

                    gl.use_program(self.program);

                    if let Some(program) = self.program {
                        let loc_rgb = gl.get_uniform_location(program, "tex_rgb");
                        gl.uniform_1_i32(loc_rgb.as_ref(), 0);
                    }

                    // 去掉toolbar影响
                    let window = self.window.as_ref().unwrap();
                    let size = window.inner_size();
                    let inner_width = size.width as i32;
                    let inner_height = size.height as i32;

                    let matrix = (self.view_ports as f32).sqrt().ceil() as usize;

                    let grid_cols = matrix as i32;
                    let grid_rows = matrix as i32;
                    let cell_width = inner_width / grid_cols;
                    let cell_height = inner_height / grid_rows;

                    for col in 0..grid_cols {
                        for row in 0..grid_rows {
                            let offset = (row + col * grid_cols) as usize;
                            // log::warn!("drawing {offset}");
                            if offset < self.tex_rgb.len() {
                                gl.viewport(
                                    row * cell_width,
                                    (grid_cols - col - 1) * cell_height,
                                    cell_width,
                                    cell_height,
                                );
                                gl.active_texture(glow::TEXTURE0);
                                gl.bind_texture(glow::TEXTURE_2D, Some(self.tex_rgb[offset]));

                                gl.bind_vertex_array(self.vao);
                                gl.draw_arrays(glow::TRIANGLES, 0, 6);
                            }
                        }
                    }
                    log::debug!("[App]drawing done");
                }
                // code = (self.recycle_frames)(self.player);
                if code < 0 {
                    panic!("[Mock][Redraw]recycle_frames failed {code}");
                }
                self.surface
                    .as_ref()
                    .unwrap()
                    .swap_buffers(self.context.as_ref().unwrap())
                    .unwrap();
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

pub fn mock_video_window(
    type_: i32,
    view_ports: usize,
    media_path: *const std::ffi::c_char,
    init_player: extern "C" fn(
        video_paths: *const *const std::ffi::c_char,
        length: std::ffi::c_int,
    ) -> *const std::ffi::c_void,
    get_ratios: extern "C" fn(
        widths: *mut std::ffi::c_int,
        heights: *mut std::ffi::c_int,
        length: std::ffi::c_int,
        player: *const std::ffi::c_void,
    ) -> std::ffi::c_int,
    register_textures: extern "C" fn(_evt_id: std::ffi::c_int, param: *mut std::ffi::c_void),
    unregister_textures: extern "C" fn(_evt_id: std::ffi::c_int, param: *mut std::ffi::c_void),
    start_player: extern "C" fn(player: *const std::ffi::c_void) -> std::ffi::c_int,
    pause_player: extern "C" fn(player: *const std::ffi::c_void) -> std::ffi::c_int,
    load_frames: extern "C" fn(player: *const std::ffi::c_void) -> std::ffi::c_int,
    draw_frames: extern "C" fn(_evt_id: std::ffi::c_int, param: *mut std::ffi::c_void),
    // recycle_frames: extern "C" fn(player: *const std::ffi::c_void) -> std::ffi::c_int,
    stop_player: extern "C" fn(player: *const std::ffi::c_void) -> std::ffi::c_int,
    terminate_player: extern "C" fn(player: *const std::ffi::c_void) -> std::ffi::c_int,
) -> Result<()> {
    // 初始化日志
    // init_logger(utils::str_to_c_str("./log").as_ptr(), 1);

    let event_loop = EventLoop::new()?;
    // let mut builder = EventLoop::<UserEvent>::with_user_event();
    // let event_loop = builder.build()?;
    let media_path = PathBuf::from_str(&utils::c_str_to_string(media_path)?)?;

    let video_name = match type_ {
        0 => "video",
        1 => "video_hevc",
        2 => "sample",
        3 => "sample_idx",
        4 => "test2/screen_",
        5 => "test03/screen_",
        6 => "test04_nvenc_hevc_long/fh_screen_",
        7 => "test05_nvenc_hevc_scale/fh_screen_",
        8 => "video_short",
        _ => "",
    };
    let video_paths = (0..view_ports)
        .into_iter()
        .map(|v| {
            utils::string_to_c_str(
                media_path
                    .join(format!("{video_name}{v}.mp4"))
                    .to_string_lossy()
                    .to_string(),
            )
        })
        .collect::<Vec<CString>>();
    let video_path_cstr = video_paths
        .iter()
        .map(|v| v.as_ptr())
        .collect::<Vec<*const std::ffi::c_char>>();
    // 初始化播放器
    let player = init_player(video_path_cstr.as_ptr(), view_ports as i32);
    if player.is_null() {
        return Err(anyhow!("player is null"));
    }
    // let proxy = event_loop.create_proxy();
    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();
    let mut app = MockVideoApp::new(
        running,
        player,
        view_ports,
        get_ratios,
        register_textures,
        unregister_textures,
        start_player,
        pause_player,
        load_frames,
        draw_frames,
        // recycle_frames,
    );
    let (sndr, rcvr) = std::sync::mpsc::channel::<i32>();
    ctrlc::set_handler(move || {
        log::warn!("Ctrl-C received, gracefully clearing up cuda");
        r.store(false, Ordering::SeqCst);
        let _ = rcvr.recv();
        // std::thread::sleep(Duration::from_secs(1));
        // std::thread::sleep(Duration::from_millis(60));
        log::warn!("Gracefully clearing up done");
    })?;
    let mut code;
    let player = {
        event_loop.run_app(&mut app)?;
        let player = app.player;
        code = stop_player(player);
        if code < 0 {
            panic!("stop_player failed {code}");
        }
        drop(app);
        player
    };
    code = terminate_player(player);
    if code < 0 {
        panic!("terminate_player failed {code}");
    }
    let _ = sndr.send(0);
    // std::thread::sleep(Duration::from_millis(60));
    Ok(())
}
