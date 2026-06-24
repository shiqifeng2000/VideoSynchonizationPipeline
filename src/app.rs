use crate::cuda::{cudaFree, cudaMalloc, cudaMemset2D};
use crate::utils::{AudioCtrl, CHANNEL_SIZE};
use crate::{cuda_check, cuda_error, elogger};
use anyhow::{Context, Result, anyhow, bail};
use bytes::Bytes;
use glow::HasContext;
use std::collections::HashMap;
use std::ptr::null_mut;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock, Weak};
use std::time::{Duration, Instant};
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

pub struct D3d11App {
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

impl D3d11App {
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

    pub fn create_textures(&mut self, resources: &mut Vec<Box<dyn GluResource>>) -> Result<()> {
        for resource in resources {
            let Some(GluResourceStatInfo { width, height, .. }) = resource.get_info() else {
                continue;
            };
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

            resource.register(texure.as_raw())?;

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

impl Drop for D3d11App {
    fn drop(&mut self) {
        for texure in self.tex_rgba.split_off(0) {
            self.egui_renderer.unregister_user_texture(texure);
        }
    }
}

// #[derive(Default)]
pub struct AppRunner {
    pub running: Arc<AtomicBool>,
    window: Option<Window>,
    // window_attributes: WindowAttributes,

    // pub gl: Option<glow::Context>,
    // pub surface: Option<Surface<WindowSurface>>,
    // pub context: Option<PossiblyCurrentContext>,
    // pub program: Option<glow::Program>,
    // pub vao: Option<glow::NativeVertexArray>,
    // pub tex_rgb: Vec<glow::NativeTexture>,
    d3d11_app: Option<D3d11App>,
    resources: Vec<Box<dyn GluResource>>,

    pub last_op: Instant,
    pub sync_ctrl: Option<GluResourceCtrlSync>,
    // pub redraw: bool,
    // pub onload: Box<dyn FnMut(HashMap<usize, CudaFrame>) -> Result<()> + Send + 'static>,
}

impl AppRunner {
    pub fn new(
        running: Arc<AtomicBool>,
        resources: Vec<Box<dyn GluResource>>,
        sync_ctrl: Option<GluResourceCtrlSync>,
    ) -> Self {
        Self {
            running,
            // window_attributes: WindowAttributes::default(),
            window: None,
            d3d11_app: None,
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
impl ApplicationHandler<UserEvent> for AppRunner {
    /// 在windows中gl drop和windows是绑定的，所以务必确保退出时即使反注册
    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        for resource in &mut self.resources {
            let _ = elogger!(resource.unregister());
        }
    }
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        let window_attrs = WindowAttributes::default()
            .with_title("D3D11 CUDA Demo")
            .with_inner_size(PhysicalSize::new(WIDTH, HEIGHT));
        let window = event_loop
            .create_window(window_attrs)
            .expect("Failed to create window");
        let mut demo_app = D3d11App::new(&window).expect("Fail to create demo app");
        let _ = demo_app
            .enable_multithread_protection()
            .expect("Enabling multithread protection failed");
        demo_app
            .create_textures(&mut self.resources)
            .expect("Texture creating error");
        self.d3d11_app.replace(demo_app);
        self.window.replace(window);

        if let Some(sync_ctrl) = &mut self.sync_ctrl {
            let _ = elogger!(sync_ctrl.start());
        }
    }
    fn suspended(&mut self, _: &ActiveEventLoop) {
        for resource in &mut self.resources {
            let _ = elogger!(resource.unregister());
        }
        self.d3d11_app.take();
        self.window.take();
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
            WindowEvent::Resized(new_size) => {
                if let Some(d3d11_app) = &mut self.d3d11_app {
                    d3d11_app.resize(&new_size);
                }
            }
            WindowEvent::RedrawRequested => {
                let Some(window) = &self.window else {
                    return;
                };
                let Some(sync_ctrl) = &mut self.sync_ctrl else {
                    return;
                };
                let Some(d3d11_app) = &mut self.d3d11_app else {
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
                d3d11_app.multithread_enter();
                for i in 0..self.resources.len() {
                    let _ = elogger!(self.resources[i].draw(data.get(&i)));
                }
                d3d11_app.multithread_leave();

                let pts = data.iter().find(|_| true).map(|(_, v)| v.pts).unwrap();
                let _ = elogger!(sync_ctrl.recycle(data));

                d3d11_app.render(window);
                log::debug!("[App]drawing done for <{pts}>",);
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
    fn register(&mut self, tex_rgba_id: *mut std::ffi::c_void) -> Result<()>;
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
