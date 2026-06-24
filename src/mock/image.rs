use crate::utils;
use anyhow::{Result, anyhow};
use glow::HasContext;
use glutin::config::ConfigTemplateBuilder;
use glutin::context::{ContextAttributesBuilder, PossiblyCurrentContext};
use glutin::display::GetGlDisplay;
use glutin::prelude::*;
use glutin::surface::{Surface, SurfaceAttributesBuilder, WindowSurface};
use glutin_winit::DisplayBuilder;
use std::f64;
use std::ffi::CString;
use std::num::NonZeroU32;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::event_loop::EventLoop;
use winit::keyboard::{KeyCode, PhysicalKey};
#[allow(deprecated)]
use winit::raw_window_handle::HasRawWindowHandle;
use winit::window::{Window, WindowAttributes};

const WIDTH: i32 = 1920;
const HEIGHT: i32 = 1080;

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

struct App {
    running: Arc<AtomicBool>,
    window: Option<Window>,
    gl: Option<glow::Context>,
    surface: Option<Surface<WindowSurface>>,
    context: Option<PossiblyCurrentContext>,

    tex_y: Vec<glow::NativeTexture>,
    tex_uv: Vec<glow::NativeTexture>,
    // image_path: Vec<String>,
    // images: Vec<GluResourceImage>,
    images: Vec<*const std::ffi::c_void>,
    program: Option<glow::Program>,
    vao: Option<glow::NativeVertexArray>,

    register_image_demo_texture: unsafe extern "C" fn(
        tex_y_id: std::ffi::c_uint,
        tex_uv_id: std::ffi::c_uint,
        handle: *const std::ffi::c_void,
    ) -> std::ffi::c_int,
    draw_image_demo_texture:
        unsafe extern "C" fn(handle: *const std::ffi::c_void) -> std::ffi::c_int,
    destroy_image_demo: unsafe extern "C" fn(handle: *const std::ffi::c_void),

    frame: usize,
    start_time: Instant,
}
impl App {
    fn handle_key_press(
        &mut self,
        key_code: KeyCode,
        event_loop: &winit::event_loop::ActiveEventLoop,
    ) {
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
            _ => {}
        }
    }
}
impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let window_attrs = WindowAttributes::default()
            .with_title("模拟显示4个Texture")
            .with_inner_size(PhysicalSize::new(WIDTH, HEIGHT));

        let template = ConfigTemplateBuilder::new();
        let display_builder = DisplayBuilder::new().with_window_attributes(Some(window_attrs));

        let (window, gl_config) = display_builder
            .build(event_loop, template, |mut configs| configs.next().unwrap())
            .unwrap();

        let window = window.unwrap();

        #[allow(deprecated)]
        let context_attributes =
            ContextAttributesBuilder::new().build(window.raw_window_handle().ok());

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

        let gl = unsafe {
            glow::Context::from_loader_function(|s| {
                gl_config
                    .display()
                    .get_proc_address(&CString::new(s).unwrap())
            })
        };
        println!("gg");

        for i in 0..4 {
            // ---- textures ----
            let tex_y = unsafe {
                let t = gl.create_texture().unwrap();
                gl.bind_texture(glow::TEXTURE_2D, Some(t));
                gl.tex_image_2d(
                    glow::TEXTURE_2D,
                    0,
                    glow::R8 as i32,
                    WIDTH,
                    HEIGHT,
                    0,
                    glow::RED,
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

            let tex_uv = unsafe {
                let t = gl.create_texture().unwrap();
                gl.bind_texture(glow::TEXTURE_2D, Some(t));
                gl.tex_image_2d(
                    glow::TEXTURE_2D,
                    0,
                    glow::RG8 as i32,
                    WIDTH / 2,
                    HEIGHT / 2,
                    0,
                    glow::RG,
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
            println!("hh {i}");
            unsafe {
                if (self.register_image_demo_texture)(tex_y.0.get(), tex_uv.0.get(), self.images[i])
                    < 0
                {
                    log::error!("demo texture register failed for {i}th image");
                }
            };
            println!("ii");
            // let _ = elogger!(self.images[i].register(tex_y.0.get(), tex_uv.0.get()));
            self.tex_y.push(tex_y);
            self.tex_uv.push(tex_uv);
        }

        let program = create_program(&gl);
        let vao = unsafe { gl.create_vertex_array().unwrap() };

        self.window = Some(window);
        self.gl = Some(gl);
        self.surface = Some(surface);
        self.context = Some(context);
        self.program = Some(program);
        self.vao = Some(vao);
        // self.cuda_y = cuda_y;
        // self.cuda_uv = cuda_uv;
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
                return;
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
                println!("ff");
                let gl = self.gl.as_ref().unwrap();
                // let request_start = Instant::now();

                for i in 0..4 {
                    unsafe {
                        // upload Y
                        gl.bind_texture(glow::TEXTURE_2D, Some(self.tex_y[i]));
                        // upload UV
                        gl.bind_texture(glow::TEXTURE_2D, Some(self.tex_uv[i]));
                        // let _ = elogger!(self.images[i].draw());
                        if (self.draw_image_demo_texture)(self.images[i]) < 0 {
                            log::error!("drawing {i}th image failed");
                        }
                    }
                }
                println!("gg");
                // println!("graph map time {}", request_start.elapsed().as_millis());
                // ---- render ----
                unsafe {
                    gl.clear_color(0.1, 0.1, 0.1, 1.0);
                    gl.clear(glow::COLOR_BUFFER_BIT);

                    gl.use_program(self.program);

                    if let Some(program) = self.program {
                        let loc_y = gl.get_uniform_location(program, "tex_y");
                        let loc_uv = gl.get_uniform_location(program, "tex_uv");

                        gl.uniform_1_i32(loc_y.as_ref(), 0);
                        gl.uniform_1_i32(loc_uv.as_ref(), 1);
                    }

                    // 去掉toolbar影响
                    let window = self.window.as_ref().unwrap();
                    let size = window.inner_size();
                    let inner_width = size.width as i32;
                    let inner_height = size.height as i32;

                    let grid_cols = 2;
                    let grid_rows = 2;
                    let cell_width = inner_width / grid_cols;
                    let cell_height = inner_height / grid_rows;

                    for col in 0..grid_cols {
                        for row in 0..grid_rows {
                            let offset = (row + col * grid_cols) as usize;
                            gl.viewport(
                                row * cell_width,
                                (grid_cols - col - 1) * cell_height,
                                cell_width,
                                cell_height,
                            );
                            gl.active_texture(glow::TEXTURE0);
                            gl.bind_texture(glow::TEXTURE_2D, Some(self.tex_y[offset]));

                            gl.active_texture(glow::TEXTURE1);
                            gl.bind_texture(glow::TEXTURE_2D, Some(self.tex_uv[offset]));

                            gl.bind_vertex_array(self.vao);
                            gl.draw_arrays(glow::TRIANGLES, 0, 3);
                        }
                    }
                }
                // println!("paint time {}", request_start.elapsed().as_millis());
                // self.surface
                //     .as_ref()
                //     .unwrap()
                //     .swap_buffers(self.context.as_ref().unwrap())
                //     .unwrap();
                self.surface
                    .as_ref()
                    .unwrap()
                    .swap_buffers(self.context.as_ref().unwrap())
                    .unwrap();
                // .set_swap_interval(
                //     self.context.as_ref().unwrap(),
                //     SwapInterval::Wait(NonZero::new(10).unwrap()),
                // )
                // .unwrap();

                // println!("total time {}", request_start.elapsed().as_millis());
                self.frame += 1;
                let elapsed_secs = self.start_time.elapsed().as_secs();
                if self.frame % 100 == 0
                    && f64::MAX as usize > self.frame
                    && f64::MAX as u64 > elapsed_secs
                {
                    let fps = self.frame as f64 / elapsed_secs as f64;
                    log::debug!("frames {} fps is {fps:.4}", self.frame);
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        self.window.as_ref().unwrap().request_redraw();
    }
}

impl Drop for App {
    fn drop(&mut self) {
        unsafe {
            for i in 0..4 {
                // println!("destropying handle {i}");
                (self.destroy_image_demo)(self.images[i]);
            }
        }
        // if !self.cuda_y.is_null() {
        //     cuda_check!(cudaFree(self.cuda_y), "cudaFree Y");
        // }
        // if !self.cuda_uv.is_null() {
        //     cuda_check!(cudaFree(self.cuda_uv), "cudaFree UV");
        // }
    }
}

fn create_program(gl: &glow::Context) -> glow::Program {
    unsafe {
        let vs = gl.create_shader(glow::VERTEX_SHADER).unwrap();
        gl.shader_source(
            vs,
            r#"#version 330
out vec2 v_uv;

void main() {
    vec2 pos = vec2(
        (gl_VertexID == 2) ? 3.0 : -1.0,
        (gl_VertexID == 1) ? 3.0 : -1.0
    );
    v_uv = pos * 0.5 + 0.5;
    v_uv.y = 1.0 - v_uv.y; 
    gl_Position = vec4(pos, 0.0, 1.0);
}"#,
        );
        gl.compile_shader(vs);

        let fs = gl.create_shader(glow::FRAGMENT_SHADER).unwrap();
        gl.shader_source(
            fs,
            r#"#version 330
        in vec2 v_uv;
        out vec4 color;

        uniform sampler2D tex_y;
        uniform sampler2D tex_uv;

        void main() {
            float y = texture(tex_y, v_uv).r;
            vec2 uv = texture(tex_uv, v_uv).rg;

            float u = uv.x - 0.5;
            float v = uv.y - 0.5;

            vec3 rgb;
            rgb.r = y + 1.402 * v;
            rgb.g = y - 0.344 * u - 0.714 * v;
            rgb.b = y + 1.772 * u;

            color = vec4(rgb, 1.0);
        }"#,
        );
        gl.compile_shader(fs);

        let program = gl.create_program().unwrap();
        gl.attach_shader(program, vs);
        gl.attach_shader(program, fs);
        gl.link_program(program);

        program
    }
}

pub fn mock_image_window(
    media_path: *const std::ffi::c_char,
    width: i32,
    height: i32,
    init_image_demo: unsafe extern "C" fn(
        path: *const std::ffi::c_char,
        width: std::ffi::c_int,
        height: std::ffi::c_int,
    ) -> *const std::ffi::c_void,
    register_image_demo_texture: unsafe extern "C" fn(
        tex_y_id: std::ffi::c_uint,
        tex_uv_id: std::ffi::c_uint,
        handle: *const std::ffi::c_void,
    ) -> std::ffi::c_int,
    draw_image_demo_texture: unsafe extern "C" fn(
        handle: *const std::ffi::c_void,
    ) -> std::ffi::c_int,
    destroy_image_demo: unsafe extern "C" fn(handle: *const std::ffi::c_void),
) -> Result<()> {
    unsafe {
        let _ = std::env::set_var("RUST_LOG", "debug");
    }
    // env_logger::init();
    let _ = utils::init_logger("./log");
    let event_loop = EventLoop::new()?;
    let media_path1 = PathBuf::from_str(&utils::c_str_to_string(media_path)?)?;

    let mut images = vec![];
    for i in 0..4 {
        let img_path = media_path1
            .join(format!("test{i}.jpg"))
            .to_string_lossy()
            .to_string();
        let img_path_cstr = utils::str_to_c_str(&img_path);
        let glu_image = unsafe { init_image_demo(img_path_cstr.as_ptr(), width, height) };
        if glu_image.is_null() {
            return Err(anyhow!("creating glu_image failed for <{img_path}>"));
        }
        images.push(glu_image);
        // if let Ok(glu_image) = elogger!(GluResourceImage::new(&img_path, 0, None, None,)) {
        //     images.push(glu_image);
        // };
    }
    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();
    let mut app = App {
        running,
        window: None,
        gl: None,
        surface: None,
        context: None,
        tex_y: vec![],
        tex_uv: vec![],
        program: None,
        vao: None,
        images,

        register_image_demo_texture,
        draw_image_demo_texture,
        destroy_image_demo,
        frame: 0,
        start_time: Instant::now(),
    };
    println!("cc");
    let (sndr, rcvr) = std::sync::mpsc::channel::<i32>();
    ctrlc::set_handler(move || {
        log::warn!("Ctrl-C received, gracefully clearing up cuda");
        r.store(false, Ordering::SeqCst);
        let _ = rcvr.recv();
        log::warn!("Gracefully clearing up done");
    })?;

    println!("dd");
    event_loop.run_app(&mut app)?;
    println!("ee");
    drop(app);
    let _ = sndr.send(0);
    // println!("quitting");
    // drop(app);
    // println!("quitting1");
    // std::thread::sleep(Duration::from_millis(1000));
    // println!("quitting2");
    Ok(())
}
