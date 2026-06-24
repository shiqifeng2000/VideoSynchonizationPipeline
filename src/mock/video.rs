use crate::api::UnityParam;
use crate::app::D3d11App;
use crate::utils;
use crate::utils::{AudioCtrl, CHANNEL_SIZE};
use crate::{cuda_check, cuda_error, elogger};
use anyhow::{Context, Result, anyhow, bail};
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
use std::path::PathBuf;
use std::ptr::null;
use std::ptr::null_mut;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, RwLock, Weak};
use std::time::Duration;
use std::time::Instant;
use std::{f64, usize};
use windows::Win32::Graphics::Direct3D::D3D11_SRV_DIMENSION_TEXTURE2D;
use windows::Win32::Graphics::Direct3D11::{D3D11_TEX2D_SRV, ID3D11Multithread, ID3D11Resource};
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT, DXGI_FORMAT_R8G8B8A8_UNORM};
use windows::Win32::Graphics::Dxgi::DXGI_PRESENT;
use windows::Win32::{
    Foundation::{HMODULE, HWND},
    Graphics::{
        Direct3D::{D3D_DRIVER_TYPE_UNKNOWN, D3D_FEATURE_LEVEL_11_0},
        Direct3D11, Dxgi,
    },
};
use windows::core::Interface;
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
const PADDING: i32 = 4;
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

pub struct MockD3d11App {
    device: Direct3D11::ID3D11Device,
    device_context: Direct3D11::ID3D11DeviceContext,
    swap_chain: Dxgi::IDXGISwapChain,
    render_target: Option<Direct3D11::ID3D11RenderTargetView>,
    egui_ctx: egui::Context,
    egui_renderer: egui_directx11::Renderer,
    egui_winit: egui_winit::State,
    multithread: Option<ID3D11Multithread>,
    tex_rgba: Vec<egui::TextureId>,
}

impl MockD3d11App {
    pub fn new(window: &Window) -> Result<Self> {
        use winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
        let RawWindowHandle::Win32(window_handle) = window.window_handle()?.as_raw() else {
            bail!("Unexpected RawWindowHandle variant");
        };

        let (device, device_context, swap_chain) = {
            let PhysicalSize { width, height } = window.inner_size();
            Self::create_device_and_swap_chain(
                HWND(window_handle.hwnd.get() as _),
                width,
                height,
                DXGI_FORMAT_R8G8B8A8_UNORM,
            )
        }
        .context("Failed to create device and swap chain")?;

        let render_target = Some(
            Self::create_render_target_for_swap_chain(&device, &swap_chain)
                .context("Failed to create render target")?,
        );

        let egui_ctx = egui::Context::default();
        let egui_renderer =
            egui_directx11::Renderer::new(&device).context("Failed to create egui renderer")?;
        let egui_winit = egui_winit::State::new(
            egui_ctx.clone(),
            egui_ctx.viewport_id(),
            &window,
            None,
            None,
            None,
        );

        Ok(Self {
            device,
            device_context,
            swap_chain,
            render_target,
            egui_ctx,
            egui_renderer,
            egui_winit,
            tex_rgba: vec![],
            multithread: None,
        })
    }

    pub fn create_textures(
        &mut self,
        width: &[i32],
        height: &[i32],
        view_ports: usize,
        handle: *const std::ffi::c_void,
        register_textures: extern "C" fn(_evt_id: std::ffi::c_int, param: *mut std::ffi::c_void),
    ) -> Result<()> {
        let mut raw_textures = vec![];
        for i in 0..view_ports {
            let width = width[i];
            let height = height[i];
            // Image source: https://www.publicdomainpictures.net/en/view-image.php?image=308608
            // let bytes = Decoder::new(BufReader::new(&include_bytes!("./1080p.jpg")[..]))
            //     .decode()
            //     .unwrap();
            // let bytes = Vec::from_iter(
            //     bytes
            //         .chunks_exact(3)
            //         .map(|slice| u32::from_le_bytes([slice[0], slice[1], slice[2], 0])),
            // );
            let desc = Direct3D11::D3D11_TEXTURE2D_DESC {
                Width: width as _,
                Height: height as _,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_R8G8B8A8_UNORM,
                SampleDesc: Dxgi::Common::DXGI_SAMPLE_DESC {
                    Count: 1,
                    Quality: 0,
                },
                Usage: Direct3D11::D3D11_USAGE_DEFAULT,
                BindFlags: Direct3D11::D3D11_BIND_SHADER_RESOURCE.0 as _,
                CPUAccessFlags: 0,
                ..Default::default()
            };
            // let subresource_data = Direct3D11::D3D11_SUBRESOURCE_DATA {
            //     pSysMem: bytes.as_ptr() as _,
            //     SysMemPitch: (1920 * 4) as u32,
            //     SysMemSlicePitch: 0,
            // };
            let mut texure = None;
            unsafe { self.device.CreateTexture2D(&desc, None, Some(&mut texure)) }.unwrap();
            let texure = texure.unwrap();

            raw_textures.push(texure.as_raw());

            let mut shader_resource_view = None;
            let shader_desc = Direct3D11::D3D11_SHADER_RESOURCE_VIEW_DESC {
                Format: DXGI_FORMAT_R8G8B8A8_UNORM,
                ViewDimension: D3D11_SRV_DIMENSION_TEXTURE2D,
                Anonymous: Direct3D11::D3D11_SHADER_RESOURCE_VIEW_DESC_0 {
                    Texture2D: D3D11_TEX2D_SRV {
                        MostDetailedMip: 0,
                        MipLevels: 1,
                    },
                },
            };
            // device.CreateShaderResourceView(&texture, &shader_desc, &mut srv)?;
            unsafe {
                self.device.CreateShaderResourceView(
                    &texure,
                    Some(&shader_desc),
                    Some(&raw mut shader_resource_view),
                )
            }
            .context("CreateShaderResourceView Failed")?;
            let srv = shader_resource_view.unwrap();
            let id = self.egui_renderer.register_user_texture(srv);
            self.tex_rgba.push(id);
        }

        let mut param = UnityParam {
            texture_rgba_ids: raw_textures.as_ptr(),
            length: raw_textures.len() as i32,
            handle,
            code: -1,
        };
        (register_textures)(0, &mut param as *mut _ as *mut std::ffi::c_void);
        if param.code < 0 {
            bail!("[Mock][Resume]register_textures failed {}", param.code);
        }
        Ok(())
    }

    // fn on_event(&mut self, window: &Window, event: &WindowEvent) {
    //     let egui_response = self.egui_winit.on_window_event(&window, event);
    //     if !egui_response.consumed {
    //         match event {
    //             WindowEvent::Resized(new_size) => self.resize(new_size),
    //             WindowEvent::RedrawRequested => self.render(window),
    //             _ => (),
    //         }
    //     }
    // }

    // fn on_exit(&mut self) {
    //     for texure in self.tex_rgba.split_off(0) {
    //         self.egui_renderer.unregister_user_texture(texure);
    //     }
    // }

    fn multithread_enter(&self) {
        if let Some(multithread) = &self.multithread {
            unsafe { multithread.Enter() };
        }
    }
    fn multithread_leave(&self) {
        if let Some(multithread) = &self.multithread {
            unsafe { multithread.Leave() };
        }
    }
    fn render(&mut self, window: &Window) {
        if let Some(render_target) = &self.render_target {
            let size = window.inner_size();
            let inner_width = size.width as i32;
            let inner_height = size.height as i32;
            let matrix = (self.tex_rgba.len() as f32).sqrt().ceil() as usize;

            let cell_width = (inner_width - PADDING) / matrix as i32;
            let cell_height = (inner_height - PADDING) / matrix as i32;

            let egui_input = self.egui_winit.take_egui_input(window);
            let tex_rgba = self.tex_rgba.clone();
            let egui_output = self.egui_ctx.run(egui_input, |ctx| {
                let tex_rgba = tex_rgba.clone();
                egui::CentralPanel::default()
                    .frame(
                        egui::Frame::none() // 移除默认边框和内边距
                            .inner_margin(egui::Margin::ZERO)
                            .outer_margin(egui::Margin::ZERO),
                    )
                    .show(ctx, |ui| {
                        egui::Grid::new("video_grid")
                            .num_columns(matrix)
                            .spacing([PADDING as f32, PADDING as f32])
                            .show(ui, |ui| {
                                for (i, texure) in tex_rgba.into_iter().enumerate() {
                                    if i > 0 && i % matrix == 0 {
                                        ui.end_row();
                                    }
                                    let image = egui::widgets::Image::from_texture((
                                        texure,
                                        egui::Vec2::new(cell_width as f32, cell_height as f32),
                                    ))
                                    .max_size(
                                        egui::Vec2::new(cell_width as f32, cell_height as f32),
                                    );
                                    // .shrink_to_fit();
                                    ui.add(image);
                                }
                            });
                    });
            });
            let (renderer_output, platform_output, _) = egui_directx11::split_output(egui_output);
            self.egui_winit
                .handle_platform_output(window, platform_output);
            unsafe {
                self.device_context
                    .ClearRenderTargetView(render_target, &[0.0, 0.0, 0.0, 1.0]);
            }
            let _ = self.egui_renderer.render(
                &self.device_context,
                render_target,
                &self.egui_ctx,
                renderer_output,
            );
            let _ = unsafe { self.swap_chain.Present(1, DXGI_PRESENT(0)) };
        } else {
            unreachable!()
        }
    }

    fn create_device_and_swap_chain(
        window: HWND,
        frame_width: u32,
        frame_height: u32,
        frame_format: DXGI_FORMAT,
    ) -> Result<(
        Direct3D11::ID3D11Device,
        Direct3D11::ID3D11DeviceContext,
        Dxgi::IDXGISwapChain,
    )> {
        let dxgi_factory: Dxgi::IDXGIFactory = unsafe { Dxgi::CreateDXGIFactory() }?;
        let dxgi_adapter: Dxgi::IDXGIAdapter = unsafe { dxgi_factory.EnumAdapters(0) }?;

        let mut device = None;
        let mut device_context = None;
        unsafe {
            Direct3D11::D3D11CreateDevice(
                &dxgi_adapter,
                D3D_DRIVER_TYPE_UNKNOWN,
                HMODULE(std::ptr::null_mut()),
                if cfg!(debug_assertions) {
                    Direct3D11::D3D11_CREATE_DEVICE_DEBUG
                } else {
                    Direct3D11::D3D11_CREATE_DEVICE_FLAG(0)
                },
                Some(&[D3D_FEATURE_LEVEL_11_0]),
                Direct3D11::D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut device_context),
            )
        }?;
        let device = device.unwrap();
        let device_context = device_context.unwrap();

        let swap_chain_desc = Dxgi::DXGI_SWAP_CHAIN_DESC {
            BufferDesc: Dxgi::Common::DXGI_MODE_DESC {
                Width: frame_width,
                Height: frame_height,
                Format: frame_format,
                ..Dxgi::Common::DXGI_MODE_DESC::default()
            },
            SampleDesc: Dxgi::Common::DXGI_SAMPLE_DESC {
                Count: 1,
                Quality: 0,
            },
            BufferUsage: Dxgi::DXGI_USAGE_RENDER_TARGET_OUTPUT,
            BufferCount: 2,
            OutputWindow: window,
            Windowed: true.into(),
            SwapEffect: Dxgi::DXGI_SWAP_EFFECT_DISCARD,
            Flags: 0,
        };

        let mut swap_chain = None;
        unsafe { dxgi_factory.CreateSwapChain(&device, &swap_chain_desc, &mut swap_chain) }.ok()?;
        let swap_chain = swap_chain.unwrap();

        unsafe { dxgi_factory.MakeWindowAssociation(window, Dxgi::DXGI_MWA_NO_ALT_ENTER) }?;
        Ok((device, device_context, swap_chain))
    }

    fn create_render_target_for_swap_chain(
        device: &Direct3D11::ID3D11Device,
        swap_chain: &Dxgi::IDXGISwapChain,
    ) -> Result<Direct3D11::ID3D11RenderTargetView> {
        let swap_chain_texture = unsafe { swap_chain.GetBuffer::<Direct3D11::ID3D11Texture2D>(0) }?;
        let mut render_target = None;
        unsafe {
            device.CreateRenderTargetView(&swap_chain_texture, None, Some(&mut render_target))
        }?;
        Ok(render_target.unwrap())
    }

    fn resize(&mut self, new_size: &PhysicalSize<u32>) {
        if let Err(err) = self.resize_swap_chain_and_render_target(
            new_size.width,
            new_size.height,
            DXGI_FORMAT_R8G8B8A8_UNORM,
        ) {
            panic!("Failed to resize framebuffers: {err:?}");
        }
    }

    fn resize_swap_chain_and_render_target(
        &mut self,
        new_width: u32,
        new_height: u32,
        new_format: DXGI_FORMAT,
    ) -> Result<()> {
        self.render_target.take();
        unsafe {
            self.swap_chain.ResizeBuffers(
                2,
                new_width,
                new_height,
                new_format,
                Dxgi::DXGI_SWAP_CHAIN_FLAG(0),
            )
        }?;
        self.render_target
            .replace(Self::create_render_target_for_swap_chain(
                &self.device,
                &self.swap_chain,
            )?);
        Ok(())
    }

    pub fn enable_multithread_protection(&mut self) -> Result<()> {
        unsafe {
            use windows::Win32::Graphics::Direct3D11::ID3D11Multithread;
            // Query the multithread interface from the device context
            let multithread: ID3D11Multithread = self.device_context.cast()?;
            // Enable protection (this is the key!)
            if multithread.SetMultithreadProtected(true).as_bool() {
                self.multithread.replace(multithread);
            }
        }
        Ok(())
    }
}

impl Drop for MockD3d11App {
    fn drop(&mut self) {
        for texure in self.tex_rgba.split_off(0) {
            self.egui_renderer.unregister_user_texture(texure);
        }
    }
}

// #[derive(Default)]
pub struct MockVideoApp {
    pub running: Arc<AtomicBool>,
    pub window: Option<Window>,
    d3d11_app: Option<MockD3d11App>,

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
            d3d11_app: None,
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
    fn suspended(&mut self, _: &ActiveEventLoop) {
        let mut param = UnityParam {
            texture_rgba_ids: null(),
            length: 0,
            handle: self.player,
            code: -1,
        };
        (self.unregister_textures)(0, &raw mut param as *mut std::ffi::c_void);
        self.d3d11_app.take();
        self.window.take();
    }
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let window_attrs = WindowAttributes::default()
            .with_title("D3D11 CUDA Demo")
            .with_inner_size(PhysicalSize::new(WIDTH * 1 / 2, HEIGHT * 1 / 2));

        let window = event_loop
            .create_window(window_attrs)
            .expect("Failed to create window");
        let mut widths = vec![0; self.view_ports];
        let mut heights = vec![0; self.view_ports];
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

        let mut demo_app = MockD3d11App::new(&window).expect("Fail to create demo app");
        let _ = demo_app
            .enable_multithread_protection()
            .expect("Enabling multithread protection failed");

        demo_app
            .create_textures(
                &widths,
                &heights,
                self.view_ports,
                self.player,
                self.register_textures,
            )
            .expect("Texture creating error");
        self.d3d11_app.replace(demo_app);
        self.window.replace(window);

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
            WindowEvent::Resized(new_size) => {
                if let Some(d3d11_app) = &mut self.d3d11_app {
                    d3d11_app.resize(&new_size);
                }
            }
            WindowEvent::RedrawRequested => {
                let Some(window) = &self.window else {
                    return;
                };
                if self.player.is_null() {
                    return;
                }
                let Some(d3d11_app) = &mut self.d3d11_app else {
                    return;
                };
                let code = (self.load_frames)(self.player);
                if code <= 0 {
                    std::thread::sleep(Duration::from_millis(10));
                    return;
                }
                d3d11_app.multithread_enter();
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
                d3d11_app.multithread_leave();
                d3d11_app.render(window);
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
