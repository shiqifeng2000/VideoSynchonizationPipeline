use crate::{
    cuda::{
        CUDA_MEMCPY_DEVICE_TO_DEVICE, CUDA_MEMCPY_HOST_TO_DEVICE, cudaFree,
        cudaGraphicsGLRegisterImage, cudaGraphicsMapResources,
        cudaGraphicsSubResourceGetMappedArray, cudaGraphicsUnmapResources,
        cudaGraphicsUnregisterResource, cudaMalloc, cudaMemcpy, cudaMemcpy2DToArray,
    },
    cuda_check, cuda_error,
    image::NV12Buffer,
};
use anyhow::{Result, anyhow};
use std::{io::Read, ptr::null_mut};

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
    pub path: String,
    /// gpu显存指针
    cache_y: *mut std::ffi::c_void,
    cache_uv: *mut std::ffi::c_void,
    pub tex_y_id: Option<u32>,
    pub tex_uv_id: Option<u32>,
    res_y: Option<*mut std::ffi::c_void>,
    res_uv: Option<*mut std::ffi::c_void>,
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
            let NV12Buffer {
                width,
                height,
                data,
            } = NV12Buffer::from_image(path)?;
            (width as usize, height as usize, data)
        };
        let chan_size = width * height;
        let mut cache_y = null_mut();
        cuda_error!(cudaMalloc(&mut cache_y, chan_size))?;
        let a = data.as_ptr();
        cuda_error!(cudaMemcpy(
            cache_y,
            a as *const _,
            chan_size,
            CUDA_MEMCPY_HOST_TO_DEVICE
        ))?;
        let mut cache_uv = null_mut();
        cuda_error!(cudaMalloc(&mut cache_uv, chan_size / 2))?;
        cuda_error!(cudaMemcpy(
            cache_uv,
            data.as_ptr().offset(chan_size as isize) as *const _,
            chan_size / 2,
            CUDA_MEMCPY_HOST_TO_DEVICE
        ))?;

        Ok(Self {
            path: path.to_owned(),
            width,
            height,
            type_,
            cache_y,
            cache_uv,
            tex_y_id: None,
            tex_uv_id: None,
            res_y: None,
            res_uv: None,
        })
    }

    pub fn register(&mut self, tex_y_id: u32, tex_uv_id: u32) -> Result<()> {
        let res_y = {
            let mut res = null_mut();
            cuda_error!(cudaGraphicsGLRegisterImage(
                &mut res,
                tex_y_id,
                glow::TEXTURE_2D,
                0
            ))?;
            res
        };
        let res_uv = {
            let mut res = null_mut();
            cuda_error!(cudaGraphicsGLRegisterImage(
                &mut res,
                tex_uv_id,
                glow::TEXTURE_2D,
                0
            ))?;
            res
        };
        self.tex_y_id.replace(tex_y_id);
        self.tex_uv_id.replace(tex_uv_id);
        self.res_y.replace(res_y);
        self.res_uv.replace(res_uv);
        Ok(())
    }

    pub fn draw(&self) -> Result<()> {
        let Some(res_y) = self.res_y else {
            return Err(anyhow!("res_y is empty when drawing"));
        };
        let Some(res_uv) = self.res_uv else {
            return Err(anyhow!("res_y is empty when drawing"));
        };
        let mut resources = [res_y, res_uv];
        cuda_error!(cudaGraphicsMapResources(
            2,
            resources.as_mut_ptr(),
            std::ptr::null_mut()
        ))?;
        let mut array_y = std::ptr::null_mut();
        let mut array_uv = std::ptr::null_mut();
        cuda_error!(cudaGraphicsSubResourceGetMappedArray(
            &mut array_y,
            res_y,
            0,
            0
        ))?;
        cuda_error!(cudaGraphicsSubResourceGetMappedArray(
            &mut array_uv,
            res_uv,
            0,
            0
        ))?;
        // 3. CUDA → GL
        cuda_error!(cudaMemcpy2DToArray(
            array_y,
            0,
            0,
            self.cache_y,
            self.width,
            self.width,
            self.height,
            CUDA_MEMCPY_DEVICE_TO_DEVICE
        ))?;
        cuda_error!(cudaMemcpy2DToArray(
            array_uv,
            0,
            0,
            self.cache_uv,
            self.width,
            self.width,
            self.height / 2,
            CUDA_MEMCPY_DEVICE_TO_DEVICE
        ))?;
        // 4. unmap
        cuda_error!(cudaGraphicsUnmapResources(
            2,
            resources.as_mut_ptr(),
            std::ptr::null_mut()
        ))?;
        Ok(())
    }
}

impl Drop for GluResourceImage {
    fn drop(&mut self) {
        log::info!("Dropping cuda resource for <{}>", self.path);
        if let Some(res) = self.res_y {
            if !res.is_null() {
                cuda_check!(
                    cudaGraphicsUnregisterResource(res),
                    "Unregister Cuda Resource Y"
                );
            }
        }
        if let Some(res) = self.res_uv {
            if !res.is_null() {
                cuda_check!(
                    cudaGraphicsUnregisterResource(res),
                    "Unregister Cuda Resource UV"
                );
            }
        }
        if !self.cache_y.is_null() {
            cuda_check!(cudaFree(self.cache_y), "cudaFree Y");
        }
        if !self.cache_uv.is_null() {
            cuda_check!(cudaFree(self.cache_uv), "cudaFree UV");
        }
    }
}

#[derive(Clone)]
struct GluResourceVideo {
    path: String,
    type_: i32,
}
