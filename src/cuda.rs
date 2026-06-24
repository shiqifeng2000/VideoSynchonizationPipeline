#[link(name = "cuda")]
unsafe extern "C" {
    pub fn cudaMalloc(ptr: *mut *mut std::ffi::c_void, size: usize) -> i32;
    pub fn cudaFree(ptr: *mut std::ffi::c_void) -> i32;
    pub fn cudaMemcpy(
        dst: *mut std::ffi::c_void,
        src: *const std::ffi::c_void,
        size: usize,
        kind: i32,
    ) -> i32;
    pub fn cudaGraphicsGLRegisterImage(
        resource: *mut *mut std::ffi::c_void,
        image: u32,
        target: u32,
        flags: u32,
    ) -> i32;
    pub fn cudaGraphicsD3D11RegisterResource(
        cuda_resource: *mut *mut std::ffi::c_void,
        d3d_resource: *mut std::ffi::c_void,
        flags: u32,
    ) -> i32;
    pub fn cudaGraphicsUnregisterResource(resource: *mut std::ffi::c_void) -> i32;
    pub fn cudaGraphicsMapResources(
        count: i32,
        resources: *mut *mut std::ffi::c_void,
        stream: *mut std::ffi::c_void,
    ) -> i32;
    pub fn cudaGraphicsSubResourceGetMappedArray(
        array: *mut *mut std::ffi::c_void,
        resource: *mut std::ffi::c_void,
        array_index: u32,
        mip_level: u32,
    ) -> i32;
    pub fn cudaMemcpy2D(
        dst: *mut ::std::os::raw::c_void,
        dpitch: usize,
        src: *const ::std::os::raw::c_void,
        spitch: usize,
        width: usize,
        height: usize,
        kind: i32,
    ) -> i32;
    pub fn cudaMemcpy2DToArray(
        dst: *mut std::ffi::c_void,
        w_offset: usize,
        h_offset: usize,
        src: *const std::ffi::c_void,
        spitch: usize,
        width: usize,
        height: usize,
        kind: i32,
    ) -> i32;
    pub fn cudaGraphicsUnmapResources(
        count: i32,
        resources: *mut *mut std::ffi::c_void,
        stream: *mut std::ffi::c_void,
    ) -> i32;
    pub fn cudaMallocPitch(
        devPtr: *mut *mut ::std::os::raw::c_void,
        pitch: *mut usize,
        width: usize,
        height: usize,
    ) -> i32;
    pub fn cudaMemset2D(
        devPtr: *mut ::std::os::raw::c_void,
        pitch: usize,
        value: ::std::os::raw::c_int,
        width: usize,
        height: usize,
    ) -> i32;
    pub fn cudaStreamSynchronize(stream: *mut std::ffi::c_void) -> i32;

    pub fn cudaDeviceSynchronize() -> i32;
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct NppiSize {
    pub width: ::std::os::raw::c_int,
    pub height: ::std::os::raw::c_int,
}
#[allow(non_snake_case)]
#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct NppStreamContext {
    pub hStream: *mut std::ffi::c_void,
    pub nCudaDeviceId: ::std::os::raw::c_int,
    pub nMultiProcessorCount: ::std::os::raw::c_int,
    pub nMaxThreadsPerMultiProcessor: ::std::os::raw::c_int,
    pub nMaxThreadsPerBlock: ::std::os::raw::c_int,
    pub nSharedMemPerBlock: usize,
    pub nCudaDevAttrComputeCapabilityMajor: ::std::os::raw::c_int,
    pub nCudaDevAttrComputeCapabilityMinor: ::std::os::raw::c_int,
    pub nStreamFlags: ::std::os::raw::c_uint,
    pub nReserved0: ::std::os::raw::c_int,
}
unsafe impl Sync for NppStreamContext {}
unsafe impl Send for NppStreamContext {}

#[link(name = "nppc")]
unsafe extern "C" {
    pub fn nppGetStreamContext(ctx: *mut NppStreamContext) -> i32;
}
#[link(name = "nppicc")]
unsafe extern "C" {
    pub fn nppiNV12ToRGB_8u_P2C3R_Ctx(
        pSrc: *const *const std::ffi::c_uchar,
        rSrcStep: ::std::os::raw::c_int,
        pDst: *mut std::ffi::c_uchar,
        nDstStep: ::std::os::raw::c_int,
        oSizeROI: NppiSize,
        nppStreamCtx: NppStreamContext,
    ) -> i32;
}

pub const CUDA_MEMCPY_HOST_TO_HOST: i32 = 0;
pub const CUDA_MEMCPY_HOST_TO_DEVICE: i32 = 1;
pub const CUDA_MEMCPY_DEVICE_TO_HOST: i32 = 2;
pub const CUDA_MEMCPY_DEVICE_TO_DEVICE: i32 = 3;
pub const CUDA_GRAPHIC_REGISTER_FLAG_SURFACE_LOAD_STORE: u32 = 4;

// #[repr(C)]
// #[derive(Debug, Copy, Clone)]
// pub struct cudaGraphicsResource {
//     _unused: [u8; 0],
// }
// // constants
// pub const CUDA_MEMCPY_HOST_TO_DEVICE: i32 = 1;
// pub const CUDA_MEMCPY_DEVICE_TO_DEVICE: i32 = 2;
// pub const CUDA_GRAPHICS_REGISTER_FLAGS_WRITE_DISCARD: u32 = 2;
