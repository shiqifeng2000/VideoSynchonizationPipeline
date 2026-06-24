use anyhow::{Result, anyhow};
use glam::{Mat4, vec3};
use glium::backend::Facade;
use glium::backend::glutin::Display as GliumDisplay;
use glium::{Surface, glutin};
use glow::HasContext;
use glutin::config::ConfigTemplateBuilder;
use glutin::context::{ContextAttributesBuilder, PossiblyCurrentContext};
use glutin::display::GetGlDisplay;
use glutin::prelude::*;
use glutin::surface::{SurfaceAttributesBuilder, SwapInterval, WindowSurface};
use glutin_winit::DisplayBuilder;
use std::collections::HashMap;
use std::f64;
use std::ffi::CString;
use std::num::NonZeroU32;
use std::ptr::null_mut;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
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

#[derive(Copy, Clone)]
struct Vertex {
    position: [f32; 3],
    tex_coords: [f32; 2],
}

glium::implement_vertex!(Vertex, position, tex_coords);

pub struct App {
    // pub running: Arc<AtomicBool>,
    pub window: Option<Window>,
    pub display: Option<GliumDisplay<WindowSurface>>,
    pub gl: Option<glow::Context>,
    pub texture: Option<glium::Texture2d>,
    pub mvp: Option<glam::Mat4>,
    pub vertex_buffer: Option<glium::vertex::VertexBuffer<Vertex>>,
    pub program: Option<glium::Program>,

    pub tex_y: Vec<glow::NativeTexture>,
    pub tex_uv: Vec<glow::NativeTexture>,
    // pub program: Option<glow::Program>,
    pub vao: Option<glow::NativeVertexArray>,
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
    /// 在windows中gl drop和windows是绑定的，所以务必确保退出时即使反注册
    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {}
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

        // SAFETY: context is current, valid
        let display = unsafe { GliumDisplay::from_context_surface(context, surface).unwrap() };
        // let gl = display.get_context();
        // surface
        //     .set_swap_interval(
        //         &context,
        //         // SwapInterval::Wait(NonZeroU32::new(1).unwrap()), // VSync 开启
        //         SwapInterval::DontWait, // 或关闭 VSync
        //     )
        //     .unwrap();

        let gl = unsafe {
            glow::Context::from_loader_function(|s| {
                gl_config
                    .display()
                    .get_proc_address(&CString::new(s).unwrap())
            })
        };

        // for i in 0..VIEW_PORTS {
        //     // ---- textures ----
        //     let tex_y = unsafe {
        //         let t = gl.create_texture().unwrap();
        //         gl.bind_texture(glow::TEXTURE_2D, Some(t));
        //         gl.tex_image_2d(
        //             glow::TEXTURE_2D,
        //             0,
        //             glow::R8 as i32,
        //             WIDTH,
        //             HEIGHT,
        //             0,
        //             glow::RED,
        //             glow::UNSIGNED_BYTE,
        //             None,
        //         );
        //         gl.tex_parameter_i32(
        //             glow::TEXTURE_2D,
        //             glow::TEXTURE_MIN_FILTER,
        //             glow::LINEAR as i32,
        //         );
        //         gl.tex_parameter_i32(
        //             glow::TEXTURE_2D,
        //             glow::TEXTURE_MAG_FILTER,
        //             glow::LINEAR as i32,
        //         );
        //         gl.tex_parameter_i32(
        //             glow::TEXTURE_2D,
        //             glow::TEXTURE_WRAP_S,
        //             glow::CLAMP_TO_EDGE as i32,
        //         );
        //         gl.tex_parameter_i32(
        //             glow::TEXTURE_2D,
        //             glow::TEXTURE_WRAP_T,
        //             glow::CLAMP_TO_EDGE as i32,
        //         );
        //         t
        //     };

        //     let tex_uv = unsafe {
        //         let t = gl.create_texture().unwrap();
        //         gl.bind_texture(glow::TEXTURE_2D, Some(t));
        //         gl.tex_image_2d(
        //             glow::TEXTURE_2D,
        //             0,
        //             glow::RG8 as i32,
        //             WIDTH / 2,
        //             HEIGHT / 2,
        //             0,
        //             glow::RG,
        //             glow::UNSIGNED_BYTE,
        //             None,
        //         );

        //         gl.tex_parameter_i32(
        //             glow::TEXTURE_2D,
        //             glow::TEXTURE_MIN_FILTER,
        //             glow::LINEAR as i32,
        //         );
        //         gl.tex_parameter_i32(
        //             glow::TEXTURE_2D,
        //             glow::TEXTURE_MAG_FILTER,
        //             glow::LINEAR as i32,
        //         );
        //         gl.tex_parameter_i32(
        //             glow::TEXTURE_2D,
        //             glow::TEXTURE_WRAP_S,
        //             glow::CLAMP_TO_EDGE as i32,
        //         );
        //         gl.tex_parameter_i32(
        //             glow::TEXTURE_2D,
        //             glow::TEXTURE_WRAP_T,
        //             glow::CLAMP_TO_EDGE as i32,
        //         );
        //         t
        //     };
        //     // self.resources[i].stop();
        //     // println!("app registering {} {}", tex_y.0.get(), tex_uv.0.get());
        //     let _ = elogger!(self.resources[i].register(tex_y.0.get(), tex_uv.0.get()));
        //     self.resources[i].start();
        //     self.tex_y.push(tex_y);
        //     self.tex_uv.push(tex_uv);
        // }

        // let program = create_program(&gl);
        // let vao = unsafe { gl.create_vertex_array().unwrap() };

        // ---- define 4 base points (front face) ----
        let p0 = [-1.0, -1.0, -1.0];
        let p1 = [1.0, -1.0, -1.0];
        let p2 = [1.0, 1.0, -1.0];
        let p3 = [-1.0, 1.0, -1.0];

        let depth = 2.0;

        // ---- build cube (6 faces) ----
        let vertices = vec![
            // FRONT
            v(p0, [0.0, 0.0]),
            v(p1, [1.0, 0.0]),
            v(p2, [1.0, 1.0]),
            v(p0, [0.0, 0.0]),
            v(p2, [1.0, 1.0]),
            v(p3, [0.0, 1.0]),
            // BACK
            v(offset(p1, depth), [0.0, 0.0]),
            v(offset(p0, depth), [1.0, 0.0]),
            v(offset(p3, depth), [1.0, 1.0]),
            v(offset(p1, depth), [0.0, 0.0]),
            v(offset(p3, depth), [1.0, 1.0]),
            v(offset(p2, depth), [0.0, 1.0]),
            // LEFT
            v(p0, [0.0, 0.0]),
            v(p3, [1.0, 0.0]),
            v(offset(p3, depth), [1.0, 1.0]),
            v(p0, [0.0, 0.0]),
            v(offset(p3, depth), [1.0, 1.0]),
            v(offset(p0, depth), [0.0, 1.0]),
            // RIGHT
            v(p1, [0.0, 0.0]),
            v(offset(p1, depth), [1.0, 0.0]),
            v(offset(p2, depth), [1.0, 1.0]),
            v(p1, [0.0, 0.0]),
            v(offset(p2, depth), [1.0, 1.0]),
            v(p2, [0.0, 1.0]),
            // TOP
            v(p3, [0.0, 0.0]),
            v(p2, [1.0, 0.0]),
            v(offset(p2, depth), [1.0, 1.0]),
            v(p3, [0.0, 0.0]),
            v(offset(p2, depth), [1.0, 1.0]),
            v(offset(p3, depth), [0.0, 1.0]),
            // BOTTOM
            v(p0, [0.0, 0.0]),
            v(offset(p0, depth), [1.0, 0.0]),
            v(offset(p1, depth), [1.0, 1.0]),
            v(p0, [0.0, 0.0]),
            v(offset(p1, depth), [1.0, 1.0]),
            v(p1, [0.0, 1.0]),
        ];

        let vertex_buffer = glium::VertexBuffer::new(&display, &vertices).unwrap();

        // ---- texture ----
        let img = image::open("/data/workspace/boe/2026_heterogeneous_space_aigc/media/0.jpg")
            .unwrap()
            .to_rgba8();
        let dims = img.dimensions();
        let raw = glium::texture::RawImage2d::from_raw_rgba_reversed(&img.into_raw(), dims);

        let texture = glium::texture::Texture2d::new(&display, raw).unwrap();

        // ---- shader ----
        let program = glium::Program::from_source(
            &display,
            r#"
            #version 330
            in vec3 position;
            in vec2 tex_coords;
            out vec2 v_uv;

            uniform mat4 mvp;

            void main() {
                v_uv = tex_coords;
                gl_Position = mvp * vec4(position, 1.0);
            }
        "#,
            r#"
            #version 330
            in vec2 v_uv;
            out vec4 color;

            uniform sampler2D tex;

            void main() {
                color = texture(tex, v_uv);
            }
        "#,
            None,
        )
        .unwrap();
        let proj = Mat4::perspective_rh_gl(45f32.to_radians(), 1.0, 0.1, 100.0);
        let view = Mat4::look_at_rh(
            vec3(3.0, 3.0, 3.0),
            vec3(0.0, 0.0, 0.0),
            vec3(0.0, 1.0, 0.0),
        );
        let model = Mat4::IDENTITY;

        let mvp = proj * view * model;

        self.window = Some(window);
        self.display = Some(display);
        self.gl = Some(gl);
        self.texture = Some(texture);
        self.mvp = Some(mvp);
        self.vertex_buffer = Some(vertex_buffer);
        self.program = Some(program);
        // self.surface = Some(surface);
        // self.context = Some(context);
        // self.program = Some(program);
        // self.vao = Some(vao);
        // self.cuda_y = cuda_y;
        // self.cuda_uv = cuda_uv;
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _id: winit::window::WindowId,
        event: WindowEvent,
    ) {
        // if !self.running.load(Ordering::SeqCst) {
        //     event_loop.exit();
        //     return;
        // }

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
                let display = self.display.as_ref().unwrap();
                let mut frame = display.draw();
                // let mvp = self.mvp.as_ref().unwrap();
                frame.clear_color_and_depth((0.1, 0.1, 0.1, 1.0), 1.0);

                let uniforms = glium::uniform! {
                    tex: self.texture.as_ref().unwrap(),
                    mvp: self.mvp.as_ref().unwrap().to_cols_array_2d(),
                };

                frame
                    .draw(
                        self.vertex_buffer.as_ref().unwrap(),
                        glium::index::NoIndices(glium::index::PrimitiveType::TrianglesList),
                        self.program.as_ref().unwrap(),
                        &uniforms,
                        &Default::default(),
                    )
                    .unwrap();

                frame.finish().unwrap();
                // let gl = self.gl.as_ref().unwrap();
                // let request_start = Instant::now();
                // for i in 0..VIEW_PORTS {
                //     unsafe {
                //         // upload Y
                //         gl.bind_texture(glow::TEXTURE_2D, Some(self.tex_y[i]));
                //         // upload UV
                //         gl.bind_texture(glow::TEXTURE_2D, Some(self.tex_uv[i]));
                //         let _ = elogger!(self.resources[i].draw(self.sync_ctrl.as_ref()));
                //     }
                // }
                // // log::debug!("==>draw time {}", request_start.elapsed().as_millis());
                // // println!("graph map time {}", request_start.elapsed().as_millis());
                // // ---- render ----
                // unsafe {
                //     gl.clear_color(0.1, 0.1, 0.1, 1.0);
                //     gl.clear(glow::COLOR_BUFFER_BIT);

                //     gl.use_program(self.program);

                //     if let Some(program) = self.program {
                //         let loc_y = gl.get_uniform_location(program, "tex_y");
                //         let loc_uv = gl.get_uniform_location(program, "tex_uv");

                //         gl.uniform_1_i32(loc_y.as_ref(), 0);
                //         gl.uniform_1_i32(loc_uv.as_ref(), 1);
                //     }

                //     // 去掉toolbar影响
                //     let window = self.window.as_ref().unwrap();
                //     let size = window.inner_size();
                //     let inner_width = size.width as i32;
                //     let inner_height = size.height as i32;

                //     let grid_cols = 2;
                //     let grid_rows = 2;
                //     let cell_width = inner_width / grid_cols;
                //     let cell_height = inner_height / grid_rows;

                //     for col in 0..grid_cols {
                //         for row in 0..grid_rows {
                //             let offset = (row + col * grid_cols) as usize;
                //             gl.viewport(
                //                 row * cell_width,
                //                 (grid_cols - col - 1) * cell_height,
                //                 cell_width,
                //                 cell_height,
                //             );
                //             gl.active_texture(glow::TEXTURE0);
                //             gl.bind_texture(glow::TEXTURE_2D, Some(self.tex_y[offset]));

                //             gl.active_texture(glow::TEXTURE1);
                //             gl.bind_texture(glow::TEXTURE_2D, Some(self.tex_uv[offset]));

                //             gl.bind_vertex_array(self.vao);
                //             gl.draw_arrays(glow::TRIANGLES, 0, 3);
                //         }
                //     }

                //     {
                //         gl.viewport(
                //             cell_width * 3 / 4,
                //             cell_height * 5 / 4,
                //             cell_width / 2,
                //             cell_height / 2,
                //         );
                //         gl.active_texture(glow::TEXTURE0);
                //         gl.bind_texture(glow::TEXTURE_2D, Some(self.tex_y[4]));

                //         gl.active_texture(glow::TEXTURE1);
                //         gl.bind_texture(glow::TEXTURE_2D, Some(self.tex_uv[4]));

                //         gl.bind_vertex_array(self.vao);
                //         gl.draw_arrays(glow::TRIANGLES, 0, 3);
                //     }
                // }
                // // self.surface
                // //     .as_ref()
                // //     .unwrap()
                // //     .swap_buffers(self.context.as_ref().unwrap())
                // //     .unwrap();
                // // log::debug!("==>texture time {}", request_start.elapsed().as_millis());
                // self.surface
                //     .as_ref()
                //     .unwrap()
                //     .swap_buffers(self.context.as_ref().unwrap())
                //     .unwrap();

                // // 创建时指定三重缓冲
                // // let config_template = glutin::config::ConfigTemplateBuilder::new()
                // //     .with_alpha_size(8)
                // //     .with_transparency(true)
                // //     // .with_double_buffer(Some(true))
                // //     .with_stereoscopy(false)
                // //     // .with_buffer_count(3) // 三重缓冲
                // //     .build();

                // // 或者在渲染循环中动态设置
                // // self.surface
                // //     .as_ref()
                // //     .unwrap()
                // //     .set_swap_interval(
                // //         self.context.as_ref().unwrap(),
                // //         SwapInterval::Wait(NonZeroU32::new(0).unwrap()), // 最小等待
                // //     )
                // //     .unwrap();

                // // self.surface
                // //     .as_ref()
                // //     .unwrap()
                // //     .set_swap_interval(self.context.as_ref().unwrap(), SwapInterval::Wait(NonZeroU32::new(10).unwrap()))
                // //     .unwrap();

                // // log::debug!(
                // //     "==>total time {} {}",
                // //     request_start.elapsed().as_millis(),
                // //     self.frames
                // // );
                // self.frames += 1;
                // let elapsed_secs = self.start_time.elapsed().as_millis();
                // if self.frames % 100 == 0
                //     && f64::MAX as usize > self.frames
                //     && f64::MAX as u64 > elapsed_secs as u64 / 1000u64
                // {
                //     let fps = self.frames as f64 * 1000f64 / elapsed_secs as f64;
                //     log::info!("frames {} fps is {fps:.4}", self.frames);
                // }
                // // self.window.as_ref().unwrap().request_redraw();
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        self.window.as_ref().unwrap().request_redraw();
    }
}

// fn create_program(gl: &glow::Context) -> glow::Program {
//     unsafe {
//         let vs = gl.create_shader(glow::VERTEX_SHADER).unwrap();
//         gl.shader_source(
//             vs,
//             r#"#version 300 es
//         precision highp float;
//         out vec2 v_uv;

//         void main() {
//             vec2 pos = vec2(
//                 (gl_VertexID == 2) ? 3.0 : -1.0,
//                 (gl_VertexID == 1) ? 3.0 : -1.0
//             );
//             v_uv = pos * 0.5 + 0.5;
//             v_uv.y = 1.0 - v_uv.y;
//             gl_Position = vec4(pos, 0.0, 1.0);
//         }"#,
//         );
//         gl.compile_shader(vs);

//         let fs = gl.create_shader(glow::FRAGMENT_SHADER).unwrap();
//         gl.shader_source(
//             fs,
//             r#"#version 300 es
//         precision highp float;

//         in vec2 v_uv;
//         out vec4 color;

//         uniform sampler2D tex_y;
//         uniform sampler2D tex_uv;

//         void main() {
//             float y = texture(tex_y, v_uv).r;
//             vec2 uv = texture(tex_uv, v_uv).rg;

//             float u = uv.x - 0.5;
//             float v = uv.y - 0.5;

//             vec3 rgb;
//             rgb.r = y + 1.402 * v;
//             rgb.g = y - 0.344 * u - 0.714 * v;
//             rgb.b = y + 1.772 * u;

//             color = vec4(rgb, 1.0);
//         }"#,
//         );
//         gl.compile_shader(fs);

//         let program = gl.create_program().unwrap();
//         gl.attach_shader(program, vs);
//         gl.attach_shader(program, fs);
//         gl.link_program(program);

//         program
//     }
// }
fn main() {
    unsafe {
        let _ = std::env::set_var("RUST_LOG", "debug");
    }
    {
        let env = env_logger::Env::default()
            .filter("RUST_LOG")
            .write_style("RUST_LOG_STYLE");
        env_logger::Builder::from_env(env)
            // .format_level(false)
            .format_timestamp_micros()
            .init();
    }
    // env_logger::init();
    let event_loop = EventLoop::new().unwrap();
    // let media_path = PathBuf::from_str("./media")?;
    // let mut resources = vec![];
    // {
    //     let video_path = "./test.mp4";
    //     // let glu_image = GluResourceImage::new(&img_path, 0, None, None,).expect("23");
    //     // images.push(glu_image);
    //     if let Ok(glu_image) = elogger!(GluResourceVideo::new(video_path)) {
    //         resources.push(Box::new(glu_image) as Box<dyn GluResource>);
    //     };
    // }
    // for i in 0..VIEW_PORTS {
    //     let video_path = media_path
    //         .join(format!("video{i}.mp4"))
    //         .to_string_lossy()
    //         .to_string();
    //     // let glu_image = GluResourceImage::new(&img_path, 0, None, None,).expect("23");
    //     // images.push(glu_image);
    //     if let Ok(mut glu_image) = elogger!(GluResourceVideo::new(&video_path)) {
    //         glu_image.set_id(i);
    //         resources.push(Box::new(glu_image) as Box<dyn GluResource>);
    //     };
    // }
    // {
    //     let video_path = media_path
    //         .join(format!("h264.mp4"))
    //         .to_string_lossy()
    //         .to_string();
    //     // let glu_image = GluResourceImage::new(&img_path, 0, None, None,).expect("23");
    //     // images.push(glu_image);
    //     if let Ok(mut glu_image) = elogger!(GluResourceVideo::new(&video_path)) {
    //         glu_image.set_id(4);
    //         resources.push(Box::new(glu_image) as Box<dyn GluResource>);
    //     };
    // }
    // for i in 2..4 {
    //     let img_path = media_path
    //         .join(format!("test{i}.jpg"))
    //         .to_string_lossy()
    //         .to_string();
    //     // let glu_image = GluResourceImage::new(&img_path, 0, None, None,).expect("23");
    //     // images.push(glu_image);
    //     if let Ok(glu_image) = elogger!(GluResourceImage::new(&img_path, 0, None, None,)) {
    //         resources.push(Box::new(glu_image) as Box<dyn GluResource>);
    //     };
    // }
    let mut app = App {
        window: None,
        gl: None,
        display: None,
        tex_y: vec![],
        tex_uv: vec![],
        program: None,
        vao: None,
        texture: None,
        mvp: None,
        vertex_buffer: None,
    };
    {
        event_loop.run_app(&mut app);
    }
}

// ---- helpers ----

fn v(pos: [f32; 3], uv: [f32; 2]) -> Vertex {
    Vertex {
        position: pos,
        tex_coords: uv,
    }
}

fn offset(p: [f32; 3], d: f32) -> [f32; 3] {
    [p[0], p[1], p[2] + d]
}
