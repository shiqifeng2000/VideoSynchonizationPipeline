use crate::app::{App, CudaFrame, GluResource, GluResourceCtrl, GluResourceStatInfo, UserEvent};
use crate::{
    cuda::{
        CUDA_MEMCPY_DEVICE_TO_DEVICE, CUDA_MEMCPY_HOST_TO_DEVICE, cudaFree,
        cudaGraphicsGLRegisterImage, cudaGraphicsMapResources,
        cudaGraphicsSubResourceGetMappedArray, cudaGraphicsUnmapResources,
        cudaGraphicsUnregisterResource, cudaMalloc, cudaMemcpy, cudaMemcpy2DToArray,
    },
    cuda_check, cuda_error,
};
use crate::{elogger, utils};
use anyhow::{Result, anyhow};
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::{io::Read, ptr::null_mut};
use winit::event_loop::EventLoop;

pub struct NV12Buffer {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>, // Y plane + interleaved UV plane
}

impl NV12Buffer {
    pub fn new(width: u32, height: u32) -> Self {
        let size = (width * height * 3 / 2) as usize;
        Self {
            width,
            height,
            data: vec![0u8; size],
        }
    }

    pub fn from_image(path: &str) -> Result<Self> {
        let img = image::open(path)?;
        let rgb = img.to_rgb8();
        Self::from_rgb(&rgb)
    }

    pub fn from_rgb(rgb: &image::RgbImage) -> Result<Self> {
        let (width, height) = rgb.dimensions();
        let mut nv12 = Self::new(width, height);

        // Y plane (full resolution)
        let y_plane_size = (width * height) as usize;

        for y in 0..height {
            for x in 0..width {
                let pixel = rgb.get_pixel(x, y);
                let r = pixel[0] as f32;
                let g = pixel[1] as f32;
                let b = pixel[2] as f32;

                // Convert to YUV (BT.601)
                let y_val = (0.299 * r + 0.587 * g + 0.114 * b).clamp(0.0, 255.0) as u8;
                let u_val = (-0.147 * r - 0.289 * g + 0.436 * b + 128.0).clamp(0.0, 255.0) as u8;
                let v_val = (0.615 * r - 0.515 * g - 0.100 * b + 128.0).clamp(0.0, 255.0) as u8;

                // Set Y
                nv12.data[(y * width + x) as usize] = y_val;

                // Set UV (only for even coordinates)
                if x % 2 == 0 && y % 2 == 0 {
                    let uv_index = y_plane_size + ((y / 2) * width + (x / 2) * 2) as usize;
                    nv12.data[uv_index as usize] = u_val;
                    nv12.data[uv_index as usize + 1] = v_val;
                }
            }
        }

        Ok(nv12)
    }
}

// #[test]
// fn test_jpeg() {
//     let nv12_buffer = NV12Buffer::from_image("/data/workspace/boe/glucube/media/test.jpg").unwrap();
//     {
//         let mut fs = std::fs::File::create("/data/workspace/boe/glucube/test.nv12").unwrap();
//         let _ = fs.write_all(&mut nv12_buffer.data.clone());
//         let _ = fs.flush();
//     }
// }

// #[derive(Clone, Default)]
// struct GluResource {
//     image: Option<GluResourceImage>,
//     video: Option<GluResourceVideo>,
//     width: usize,
//     height: usize,
// }

// impl GluResource {
//     pub fn image(
//         path: &str,
//         width: usize,
//         height: usize,
//         type_: i32,
//         tex_y_id: Option<u32>,
//         tex_uv_id: Option<u32>,
//     ) -> Result<Self> {
//         let image = GluResourceImage::new(path, width, height, type_, tex_y_id, tex_uv_id)?;
//         Ok(Self {
//             image: Some(image),
//             video: None,
//             width,
//             height,
//         })
//     }
// }

#[derive(Clone)]
pub struct GluResourceImage {
    id: usize,
    pub path: String,
    /// gpu显存指针
    cache_rgb: *mut std::ffi::c_void,
    pub tex_rgb_id: Option<u32>,
    // pub tex_uv_id: Option<u32>,
    res_rgb: Option<*mut std::ffi::c_void>,
    // res_uv: Option<*mut std::ffi::c_void>,
    pub width: usize,
    pub height: usize,
    pub type_: i32,
}
// fn mytest_fn()->Result<()> {
//     1;
//     2;
//     3;
//     return Err(anyhow!("oops"));
//     3;
// }
impl GluResourceImage {
    pub fn new(
        path: &str,
        type_: i32,
        width: Option<usize>,
        height: Option<usize>,
    ) -> Result<Self> {
        let (width, height, data) = if type_ == 23 {
            let mut data = vec![];
            let mut fs = std::fs::File::open(path)?;
            let _ = fs.read_to_end(&mut data)?;
            let Some(w) = width else {
                return Err(anyhow!("width should be set for nv12 type"));
            };
            let Some(h) = height else {
                return Err(anyhow!("height should be set for nv12 type"));
            };
            (w, h, data)
        } else {
            // let NV12Buffer {
            //     width,
            //     height,
            //     data,
            // } = NV12Buffer::from_image(path)?;
            let img = image::open(path)?;
            let rgb = img.to_rgba8();
            (img.width() as usize, img.height() as usize, rgb.to_vec())
        };
        // {
        //     use std::io::Write;
        //     let mut fs = std::fs::File::create("test.rgb").unwrap();
        //     let _ = fs.write_all(&data);
        // }
        let chan_size = width * height;
        let mut cache_rgb = null_mut();
        cuda_error!(cudaMalloc(&mut cache_rgb, chan_size * 4))?;
        let a = data.as_ptr();
        cuda_error!(cudaMemcpy(
            cache_rgb,
            a as *const _,
            chan_size * 4,
            CUDA_MEMCPY_HOST_TO_DEVICE
        ))?;

        Ok(Self {
            id: rand::random::<usize>(),
            path: path.to_owned(),
            width,
            height,
            type_,
            cache_rgb,
            tex_rgb_id: None,
            res_rgb: None, // tex_y_id: None,
                           // tex_uv_id: None,
                           // res_y: None,
                           // res_uv: None,
        })
    }
}

impl GluResource for GluResourceImage {
    fn get_info(&self) -> Option<GluResourceStatInfo> {
        Some(GluResourceStatInfo {
            width: self.width,
            height: self.height,
            ..GluResourceStatInfo::default()
        })
    }
    fn get_id(&self) -> usize {
        self.id
    }
    fn set_id(&mut self, id: usize) {
        self.id = id;
    }
    fn get_ctrl(&self) -> Option<&GluResourceCtrl> {
        None
    }
    fn start(&mut self) -> Result<()> {
        Ok(())
    }
    fn stop(&self) -> Result<()> {
        Ok(())
    }
    fn pause(&mut self) -> Result<()> {
        Ok(())
    }
    fn seek(&self, _timestap: i64) {}
    fn register(&mut self, tex_rgb_id: u32) -> Result<()> {
        let res_rgb = {
            let mut res = null_mut();
            cuda_error!(cudaGraphicsGLRegisterImage(
                &mut res,
                tex_rgb_id,
                glow::TEXTURE_2D,
                0
            ))?;
            res
        };
        self.tex_rgb_id.replace(tex_rgb_id);
        self.res_rgb.replace(res_rgb);
        // let res_uv = {
        //     let mut res = null_mut();
        //     cuda_error!(cudaGraphicsGLRegisterImage(
        //         &mut res,
        //         tex_uv_id,
        //         glow::TEXTURE_2D,
        //         0
        //     ))?;
        //     res
        // };
        // self.tex_y_id.replace(tex_y_id);
        // self.tex_uv_id.replace(tex_uv_id);
        // self.res_y.replace(res_y);
        // self.res_uv.replace(res_uv);
        Ok(())
    }
    fn unregister(&mut self) -> Result<()> {
        if let Some(res) = self.res_rgb {
            if !res.is_null() {
                cuda_check!(
                    cudaGraphicsUnregisterResource(res),
                    "Unregister Cuda Resource Y"
                );
            }
        }
        self.res_rgb.take();
        self.tex_rgb_id.take();
        // if let Some(res) = self.res_uv {
        //     if !res.is_null() {
        //         cuda_check!(
        //             cudaGraphicsUnregisterResource(res),
        //             "Unregister Cuda Resource UV"
        //         );
        //     }
        // }
        // self.res_y.take();
        // self.res_uv.take();
        // self.tex_y_id.take();
        // self.tex_uv_id.take();
        Ok(())
    }
    fn draw(&self, _ctrl: Option<&CudaFrame>) -> Result<()> {
        let Some(res_rgb) = self.res_rgb else {
            return Err(anyhow!("res_y is empty when drawing"));
        };
        // let Some(res_uv) = self.res_uv else {
        //     return Err(anyhow!("res_y is empty when drawing"));
        // };
        let mut resources = [res_rgb];
        cuda_error!(cudaGraphicsMapResources(
            1,
            resources.as_mut_ptr(),
            std::ptr::null_mut()
        ))?;
        let mut array_rgb = std::ptr::null_mut();
        cuda_error!(cudaGraphicsSubResourceGetMappedArray(
            &mut array_rgb,
            res_rgb,
            0,
            0
        ))?;
        // 3. CUDA → GL
        cuda_error!(cudaMemcpy2DToArray(
            array_rgb,
            0,
            0,
            self.cache_rgb,
            self.width * 4,
            self.width * 4,
            self.height,
            CUDA_MEMCPY_DEVICE_TO_DEVICE
        ))?;

        // {
        //     let mut data = vec![0u8; 1920 * 1080 * 3];
        //     cuda_check!(
        //         cudaMemcpy2DToArray(
        //             data.as_mut_ptr() as *mut std::ffi::c_void,
        //             0,
        //             0,
        //             self.cache_rgb,
        //             self.width,
        //             self.width,
        //             self.height,
        //             CUDA_MEMCPY_DEVICE_TO_HOST
        //         ),
        //         "oops"
        //     );
        //     // use std::io::Write;
        //     // let mut fs = std::fs::File::create("test.rgb").unwrap();
        //     // let _ = fs.write_all(&data);
        // }
        // 4. unmap
        cuda_error!(cudaGraphicsUnmapResources(
            1,
            resources.as_mut_ptr(),
            std::ptr::null_mut()
        ))?;
        Ok(())
    }
}
impl Drop for GluResourceImage {
    fn drop(&mut self) {
        log::info!("Dropping cuda resource for <{}>", self.path);
        if let Some(res) = self.res_rgb {
            if !res.is_null() {
                cuda_check!(
                    cudaGraphicsUnregisterResource(res),
                    "Unregister Cuda Resource RGB"
                );
            }
        }
        if !self.cache_rgb.is_null() {
            cuda_check!(cudaFree(self.cache_rgb), "cudaFree RGB");
        }
    }
}

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

pub fn run_image_window() -> Result<()> {
    // unsafe {
    //     let _ = std::env::set_var("RUST_LOG", "debug");
    // }
    // env_logger::init();
    let _ = utils::cleaner("./log", false);
    let _ = utils::init_logger("./log");
    // unsafe {
    //     let mut pitch: usize = 0;
    //     let mut ptr: *mut std::ffi::c_void = std::ptr::null_mut();
    //     cudaMallocPitch(&mut ptr, &mut pitch, (1920 * 3) as usize, 1080 as usize);
    //     println!("CUDA allocated pitch: {} bytes", pitch);
    // }

    // let event_loop = EventLoop::new()?;
    let mut builder = EventLoop::<UserEvent>::with_user_event();
    let event_loop = builder.build()?;
    let media_path = PathBuf::from_str("./media")?;
    let mut resources = vec![];
    let view_ports = 5;
    for i in 0..view_ports {
        let img_path = media_path
            .join(format!("test{i}.jpg"))
            .to_string_lossy()
            .to_string();
        println!("img_path {img_path}",);
        // let glu_image = GluResourceImage::new(&img_path, 0, None, None,).expect("23");
        // images.push(glu_image);
        let a = elogger!(GluResourceImage::new(&img_path, 0, None, None,));
        // match a {
        //     Ok(glu_image) => {
        //         println!("pushing {i}");
        //         resources.push(Box::new(glu_image) as Box<dyn GluResource>);
        //     }
        //     Err(e) => {
        //         println!("err {:?}", e);
        //     }
        // }
        if let Ok(mut glu_image) = a {
            glu_image.set_id(i);
            resources.push(Box::new(glu_image) as Box<dyn GluResource>);
        };
    }
    let running = Arc::new(AtomicBool::new(true));
    let r = running.clone();
    let mut app = App::new(running, resources, None);
    let (sndr, rcvr) = std::sync::mpsc::channel::<i32>();
    ctrlc::set_handler(move || {
        log::warn!("Ctrl-C received, gracefully clearing up cuda");
        r.store(false, Ordering::SeqCst);
        let _ = rcvr.recv();
        log::warn!("Gracefully clearing up done");
    })?;

    event_loop.run_app(&mut app)?;
    drop(app);
    let _ = sndr.send(0);
    Ok(())
}
