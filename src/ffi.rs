//! C-compatible API exposed through FFI
//!
//! All functions marked with #[no_mangle] and extern "C" will be
//! callable from C/C++.

use crate::mock::mock_image_window;
use crate::worker::GluResourceImage;
use crate::{elogger, utils};
use anyhow::Result;
use std::ptr::null;
use std::sync::{Arc, Mutex};

// #[unsafe(no_mangle)]
// pub unsafe extern "C" fn init_demo1();
#[unsafe(no_mangle)]
pub unsafe extern "C" fn init_demo(
    path: *const std::ffi::c_char,
    width: std::ffi::c_int,
    height: std::ffi::c_int,
    // tex_y_id: std::ffi::c_uint,
    // tex_uv_id: std::ffi::c_uint,
) -> *const std::ffi::c_void {
    elogger!(init_demo_inner(path, width, height, 0,)).unwrap_or(null())
}

fn init_demo_inner(
    path: *const std::ffi::c_char,
    width: std::ffi::c_int,
    height: std::ffi::c_int,
    type_: std::ffi::c_int,
    // tex_y_id: std::ffi::c_uint,
    // tex_uv_id: std::ffi::c_uint,
) -> Result<*const std::ffi::c_void> {
    let path_string = utils::c_str_to_string(path)?;
    let width = if width > 0 {
        Some(width as usize)
    } else {
        None
    };
    let height = if height > 0 {
        Some(height as usize)
    } else {
        None
    };
    // if tex_y_id == 0 {
    //     return Err(anyhow!("tex_y_id should not be 0"));
    // };
    // if tex_uv_id == 0 {
    //     return Err(anyhow!("tex_uv_id should not be 0"));
    // };
    let res_img = Arc::new(Mutex::new(GluResourceImage::new(
        &path_string,
        type_,
        width,
        height,
    )?));
    Ok(Arc::into_raw(res_img) as *const std::ffi::c_void)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn register_demo_texture(
    tex_y_id: std::ffi::c_uint,
    tex_uv_id: std::ffi::c_uint,
    handle: *const std::ffi::c_void,
) -> std::ffi::c_int {
    register_demo_texture_inner(tex_y_id, tex_uv_id, handle).unwrap_or(-1)
}

fn register_demo_texture_inner(
    tex_y_id: std::ffi::c_uint,
    tex_uv_id: std::ffi::c_uint,
    handle: *const std::ffi::c_void,
) -> Result<i32> {
    unsafe {
        let res_img = Arc::from_raw(handle as *const Mutex<GluResourceImage>);
        if let Ok(mut lock) = res_img.lock() {
            lock.register(tex_y_id, tex_uv_id)?;
        }
        let _ = Arc::into_raw(res_img) as *const std::ffi::c_void;
    }
    Ok(0)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn draw_demo_texture(handle: *const std::ffi::c_void) -> std::ffi::c_int {
    draw_demo_texture_inner(handle).unwrap_or(-1)
}

fn draw_demo_texture_inner(handle: *const std::ffi::c_void) -> Result<i32> {
    unsafe {
        let res_img = Arc::from_raw(handle as *const Mutex<GluResourceImage>);
        if let Ok(lock) = res_img.lock() {
            lock.draw()?;
        }
        let _ = Arc::into_raw(res_img) as *const std::ffi::c_void;
    }
    Ok(0)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn destroy_demo(handle: *const std::ffi::c_void) {
    let _ = unsafe { Arc::from_raw(handle as *const Mutex<GluResourceImage>) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn run_app(
    media_path: *const std::ffi::c_char,
    width: i32,
    height: i32,
    init_demo: unsafe extern "C" fn(
        path: *const std::ffi::c_char,
        width: std::ffi::c_int,
        height: std::ffi::c_int,
    ) -> *const std::ffi::c_void,
    register_demo_texture: unsafe extern "C" fn(
        tex_y_id: std::ffi::c_uint,
        tex_uv_id: std::ffi::c_uint,
        handle: *const std::ffi::c_void,
    ) -> std::ffi::c_int,
    draw_demo_texture: unsafe extern "C" fn(handle: *const std::ffi::c_void) -> std::ffi::c_int,
    destroy_demo: unsafe extern "C" fn(handle: *const std::ffi::c_void),
) -> std::ffi::c_int {
    if let Err(_) = elogger!(mock_image_window(
        media_path,
        width,
        height,
        init_demo,
        register_demo_texture,
        draw_demo_texture,
        destroy_demo,
    )) {
        return -1;
    }
    0
}
