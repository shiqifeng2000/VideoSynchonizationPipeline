use crate::mock::video::mock_video_window;
use crate::video::GluPlayer;
use crate::{elogger, utils};
use std::ptr::null;
use std::sync::{Arc, Mutex};

// lazy_static! {
//     pub static ref PLAYER: Arc<Mutex<Option<GluPlayer>>> = Arc::new(Mutex::new(None));
// }

#[repr(C)]
pub struct UnityParam {
    pub texture_rgba_ids: *const *mut std::ffi::c_void,
    pub length: std::ffi::c_int,
    pub handle: *const std::ffi::c_void,
    pub code: std::ffi::c_int,
}
/// 初始化日志
#[unsafe(no_mangle)]
pub extern "C" fn init_logger(logger: *const std::ffi::c_char, clear: std::ffi::c_int) {
    if clear == 1 {
        let _ = utils::cleaner("./log", false);
    }
    let logger_path = utils::c_str_to_string(logger).unwrap_or("./log".to_owned());
    if let Err(e) = utils::init_logger(&logger_path) {
        println!("[FFI] Error {e:?}");
    }
}

/// 初始化播放器
#[unsafe(no_mangle)]
pub extern "C" fn init_player(
    video_paths: *const *const std::ffi::c_char,
    length: std::ffi::c_int,
) -> *const std::ffi::c_void {
    // let logger_path = utils::c_str_to_string(logger).ok();
    let mut paths = vec![];
    unsafe {
        for (i, slice) in std::slice::from_raw_parts(video_paths, length as usize)
            .into_iter()
            .enumerate()
        {
            let Ok(video_path) = utils::c_str_to_string(*slice) else {
                log::error!("[FFI] Number {i} slice parsing error!");
                return null();
            };
            paths.push(video_path)
        }
    };
    let Ok(player) = elogger!(GluPlayer::new(&paths)) else {
        return null();
    };
    let player_arc = Arc::new(Mutex::new(player));
    Arc::into_raw(player_arc) as *const std::ffi::c_void
}

/// 获取播放源分辨率
#[unsafe(no_mangle)]
pub extern "C" fn get_ratios(
    widths: *mut std::ffi::c_int,
    heights: *mut std::ffi::c_int,
    length: std::ffi::c_int,
    player: *const std::ffi::c_void,
) -> std::ffi::c_int {
    let code = 0;
    unsafe {
        let glu_player = Arc::from_raw(player as *const Mutex<GluPlayer>);
        let player_rst = elogger!(glu_player.lock());
        if player_rst.is_err() {
            drop(player_rst);
            Arc::into_raw(glu_player) as *const std::ffi::c_void;
            return -1;
        }
        let player = player_rst.unwrap();
        let ratios = player.get_ratios();

        if ratios.len() > length as usize {
            drop(player);
            Arc::into_raw(glu_player) as *const std::ffi::c_void;
            return -2;
        }
        for (i, slice) in std::slice::from_raw_parts_mut(widths, length as usize)
            .into_iter()
            .enumerate()
        {
            *slice = ratios[i].0 as i32;
        }
        for (i, slice) in std::slice::from_raw_parts_mut(heights, length as usize)
            .into_iter()
            .enumerate()
        {
            *slice = ratios[i].1 as i32;
        }
        drop(player);
        Arc::into_raw(glu_player) as *const std::ffi::c_void;
    }
    code
}

/// 注册texture
#[unsafe(no_mangle)]
pub extern "C" fn register_textures(_evt_id: std::ffi::c_int, param: *mut std::ffi::c_void) {
    unsafe {
        let param = param as *mut UnityParam;
        let texture_ids =
            std::slice::from_raw_parts((*param).texture_rgba_ids, (*param).length as usize)
                .into_iter()
                .map(|v| *v)
                .collect::<Vec<*mut std::ffi::c_void>>();

        let glu_player = Arc::from_raw((*param).handle as *const Mutex<GluPlayer>);
        let player_rst = elogger!(glu_player.lock());
        if player_rst.is_err() {
            drop(player_rst);
            Arc::into_raw(glu_player) as *const std::ffi::c_void;
            (*param).code = -1;
            return;
        }
        let mut player = player_rst.unwrap();

        if elogger!(player.register_textures(&texture_ids)).is_err() {
            drop(player);
            Arc::into_raw(glu_player) as *const std::ffi::c_void;
            (*param).code = -2;
            return;
        }
        drop(player);
        Arc::into_raw(glu_player) as *const std::ffi::c_void;
        (*param).code = 0;
    }
}

/// 反注册texture，在停止前需要执行，避免异常
#[unsafe(no_mangle)]
pub extern "C" fn unregister_textures(_evt_id: std::ffi::c_int, param: *mut std::ffi::c_void) {
    unsafe {
        let param = param as *mut UnityParam;
        let glu_player = Arc::from_raw((*param).handle as *const Mutex<GluPlayer>);
        let player_rst = elogger!(glu_player.lock());
        if player_rst.is_err() {
            drop(player_rst);
            Arc::into_raw(glu_player) as *const std::ffi::c_void;
            (*param).code = -1;
            return;
        }
        let mut player = player_rst.unwrap();

        if elogger!(player.unregister_textures()).is_err() {
            drop(player);
            Arc::into_raw(glu_player) as *const std::ffi::c_void;
            (*param).code = -2;
            return;
        }
        drop(player);
        Arc::into_raw(glu_player) as *const std::ffi::c_void;
        (*param).code = -0;
    }
}

/// 启动播放器 = 启动解码 + 同步队列
#[unsafe(no_mangle)]
pub extern "C" fn start_player(player: *const std::ffi::c_void) -> std::ffi::c_int {
    let code = 0;
    unsafe {
        let glu_player = Arc::from_raw(player as *const Mutex<GluPlayer>);
        let player_rst = elogger!(glu_player.lock());
        if player_rst.is_err() {
            drop(player_rst);
            Arc::into_raw(glu_player) as *const std::ffi::c_void;
            return -1;
        }
        let mut player = player_rst.unwrap();

        if elogger!(player.start()).is_err() {
            drop(player);
            Arc::into_raw(glu_player) as *const std::ffi::c_void;
            return -2;
        }
        drop(player);
        Arc::into_raw(glu_player) as *const std::ffi::c_void;
    }
    code
}

/// 加载帧播放序列，该序列必须为对齐的帧
#[unsafe(no_mangle)]
pub extern "C" fn load_frames(player: *const std::ffi::c_void) -> std::ffi::c_int {
    let ret;
    unsafe {
        let glu_player = Arc::from_raw(player as *const Mutex<GluPlayer>);
        let player_rst = elogger!(glu_player.lock());
        if player_rst.is_err() {
            drop(player_rst);
            Arc::into_raw(glu_player) as *const std::ffi::c_void;
            return -1;
        }
        let mut player = player_rst.unwrap();

        let Ok(loaded) = player.load_frames() else {
            drop(player);
            Arc::into_raw(glu_player) as *const std::ffi::c_void;
            return -2;
        };
        ret = loaded;
        drop(player);
        Arc::into_raw(glu_player) as *const std::ffi::c_void;
        return ret;
    }
}

/// 绘制播放帧序列，必须在[load_frames]之后, 这时应该已经使用过 [gl.bind_texture]，在使用该方法之后，应用应该启动着色器工作也就是 [gl.draw_arrays]
#[unsafe(no_mangle)]
pub extern "C" fn draw_frames(
    // idx: std::ffi::c_int,
    // player: *const std::ffi::c_void,
    _evt_id: std::ffi::c_int,
    param: *mut std::ffi::c_void,
) {
    unsafe {
        // let frame_arc = Arc::from_raw(frames as *const HashMap<usize, CudaFrame>);
        let param = param as *mut UnityParam;
        let glu_player = Arc::from_raw((*param).handle as *const Mutex<GluPlayer>);
        let player_rst = elogger!(glu_player.lock());
        if player_rst.is_err() {
            drop(player_rst);
            Arc::into_raw(glu_player) as *const std::ffi::c_void;
            (*param).code = -1;
            return;
        }
        let mut player = player_rst.unwrap();

        if elogger!(player.draw_frames()).is_err() {
            drop(player);
            Arc::into_raw(glu_player) as *const std::ffi::c_void;
            (*param).code = -2;
            return;
        }
        drop(player);
        Arc::into_raw(glu_player) as *const std::ffi::c_void;
        (*param).code = 0;
    }
}
// #[unsafe(no_mangle)]
// pub extern "C" fn draw_frames(
//     frames: *const std::ffi::c_void,
//     player: *const std::ffi::c_void,
// ) -> std::ffi::c_int {
//     let code = 0;
//     unsafe {
//         let frame_arc = Arc::from_raw(frames as *const HashMap<usize, CudaFrame>);

//         let glu_player = Arc::from_raw(player as *const Mutex<GluPlayer>);
//         let player_rst = elogger!(glu_player.lock());
//         if player_rst.is_err() {
//             drop(player_rst);
//             Arc::into_raw(glu_player) as *const std::ffi::c_void;
//             Arc::into_raw(frame_arc) as *const std::ffi::c_void;
//             return -1;
//         }
//         let mut player = player_rst.unwrap();

//         if elogger!(player.draw_frames(frame_arc.as_ref())).is_err() {
//             drop(player);
//             Arc::into_raw(glu_player) as *const std::ffi::c_void;
//             Arc::into_raw(frame_arc) as *const std::ffi::c_void;
//             return -2;
//         }
//         drop(player);
//         Arc::into_raw(glu_player) as *const std::ffi::c_void;
//         Arc::into_raw(frame_arc) as *const std::ffi::c_void;
//     }
//     code
// }

/// 回收播放帧序列，将[load_frames]得到的数据放回到缓存队列中，用于后续解码使用，节约内存和显存
// #[unsafe(no_mangle)]
// pub extern "C" fn recycle_frames(player: *const std::ffi::c_void) -> std::ffi::c_int {
//     let code = 0;
//     unsafe {
//         // let frame_arc = Arc::from_raw(frames as *const HashMap<usize, CudaFrame>);
//         let glu_player = Arc::from_raw(player as *const Mutex<GluPlayer>);
//         let player_rst = elogger!(glu_player.lock());
//         if player_rst.is_err() {
//             drop(player_rst);
//             Arc::into_raw(glu_player) as *const std::ffi::c_void;
//             // Arc::into_raw(frame_arc) as *const std::ffi::c_void;
//             return -1;
//         }
//         let mut player = player_rst.unwrap();
//         // let Some(frames) = Arc::into_inner(frame_arc) else {
//         //     drop(player);
//         //     Arc::into_raw(glu_player) as *const std::ffi::c_void;
//         //     return -2;
//         // };
//         if elogger!(player.recycle_frames()).is_err() {
//             drop(player);
//             Arc::into_raw(glu_player) as *const std::ffi::c_void;
//             return -2;
//         }
//         drop(player);
//         Arc::into_raw(glu_player) as *const std::ffi::c_void;
//     }
//     code
// }

/// 播控，暂停，再次触发则恢复播放态，播控操作不建议1秒内频繁操作
#[unsafe(no_mangle)]
pub extern "C" fn pause_player(player: *const std::ffi::c_void) -> std::ffi::c_int {
    let code = 0;
    unsafe {
        let glu_player = Arc::from_raw(player as *const Mutex<GluPlayer>);
        let player_rst = elogger!(glu_player.lock());
        if player_rst.is_err() {
            drop(player_rst);
            Arc::into_raw(glu_player) as *const std::ffi::c_void;
            return -1;
        }
        let mut player = player_rst.unwrap();

        if elogger!(player.pause()).is_err() {
            drop(player);
            Arc::into_raw(glu_player) as *const std::ffi::c_void;
            return -2;
        }
        drop(player);
        Arc::into_raw(glu_player) as *const std::ffi::c_void;
    }
    code
}

/// 停止播放 = 解码结束 + 同步结束， 仅代表播放状态，不能代表gpu资源安全回收
#[unsafe(no_mangle)]
pub extern "C" fn stop_player(player: *const std::ffi::c_void) -> std::ffi::c_int {
    let code = 0;
    unsafe {
        let glu_player = Arc::from_raw(player as *const Mutex<GluPlayer>);
        let player_rst = elogger!(glu_player.lock());
        if player_rst.is_err() {
            drop(player_rst);
            Arc::into_raw(glu_player) as *const std::ffi::c_void;
            return -1;
        }
        let mut player = player_rst.unwrap();

        if elogger!(player.stop()).is_err() {
            drop(player);
            Arc::into_raw(glu_player) as *const std::ffi::c_void;
            return -2;
        }
        drop(player);
        Arc::into_raw(glu_player) as *const std::ffi::c_void;
    }
    code
}

/// 销毁播放器 = 等待解码线程flush， 阻塞，完成后gpu资源安全回收
#[unsafe(no_mangle)]
pub extern "C" fn terminate_player(player: *const std::ffi::c_void) -> std::ffi::c_int {
    let code = 0;
    unsafe {
        let glu_player = Arc::from_raw(player as *const Mutex<GluPlayer>);
        let Some(glu_player_mutex) = Arc::into_inner(glu_player) else {
            return -1;
        };
        let Ok(player) = elogger!(glu_player_mutex.into_inner()) else {
            return -2;
        };
        if elogger!(player.terminate()).is_err() {
            return -3;
        }
    }
    code
}

/// 销毁播放器 = 等待解码线程flush， 阻塞，完成后gpu资源安全回收
#[unsafe(no_mangle)]
pub extern "C" fn mock_video_player(
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
) -> std::ffi::c_int {
    let code = 0;
    if let Err(e) = mock_video_window(
        type_,
        view_ports,
        media_path,
        init_player,
        get_ratios,
        register_textures,
        unregister_textures,
        start_player,
        pause_player,
        load_frames,
        draw_frames,
        // recycle_frames,
        stop_player,
        terminate_player,
    ) {
        panic!("[Error] {e:?}");
    }
    code
}

pub type UnityRenderThreadEventData = unsafe extern "C" fn(std::ffi::c_int, *mut std::ffi::c_void);

#[unsafe(no_mangle)]
pub unsafe extern "C" fn GetRegisterEventFunc() -> UnityRenderThreadEventData {
    register_textures
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn GetUnRegisterEventFunc() -> UnityRenderThreadEventData {
    unregister_textures
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn GetDrawEventFunc() -> UnityRenderThreadEventData {
    draw_frames
}

// #[unsafe(no_mangle)]
// pub unsafe extern "C" fn run_app(
//     media_path: *const std::ffi::c_char,
//     init_image_demo: unsafe extern "C" fn(
//         path: *const std::ffi::c_char,
//         width: std::ffi::c_int,
//         height: std::ffi::c_int,
//     ) -> *const std::ffi::c_void,
//     register_image_demo_texture: unsafe extern "C" fn(
//         tex_y_id: std::ffi::c_uint,
//         tex_uv_id: std::ffi::c_uint,
//         handle: *const std::ffi::c_void,
//     ) -> std::ffi::c_int,
//     draw_image_demo_texture: unsafe extern "C" fn(
//         handle: *const std::ffi::c_void,
//     ) -> std::ffi::c_int,
//     destroy_image_demo: unsafe extern "C" fn(handle: *const std::ffi::c_void),
// ) -> std::ffi::c_int {
//     if let Err(_) = elogger!(mock_image_window(
//         media_path,
//         width,
//         height,
//         init_image_demo,
//         register_image_demo_texture,
//         draw_image_demo_texture,
//         destroy_image_demo,
//     )) {
//         return -1;
//     }
//     0
// }

// fn init_player_worker(
//     id: std::ffi::c_int,
//     path: *const std::ffi::c_char,
// ) -> Result<*const std::ffi::c_void> {
//     let path_string = utils::c_str_to_string(path)?;
//     let res_video = GluResourceVideo::new(&path_string)?;
//     Ok(Arc::into_raw(res_img) as *const std::ffi::c_void)
// }

// #[unsafe(no_mangle)]
// pub unsafe extern "C" fn register_image_demo_texture(
//     tex_y_id: std::ffi::c_uint,
//     tex_uv_id: std::ffi::c_uint,
//     handle: *const std::ffi::c_void,
// ) -> std::ffi::c_int {
//     register_image_demo_texture_inner(tex_y_id, tex_uv_id, handle).unwrap_or(-1)
// }

// fn register_image_demo_texture_inner(
//     tex_y_id: std::ffi::c_uint,
//     tex_uv_id: std::ffi::c_uint,
//     handle: *const std::ffi::c_void,
// ) -> Result<i32> {
//     unsafe {
//         let res_img = Arc::from_raw(handle as *const Mutex<GluResourceImage>);
//         if let Ok(mut lock) = res_img.lock() {
//             lock.register(tex_y_id, tex_uv_id)?;
//         }
//         let _ = Arc::into_raw(res_img) as *const std::ffi::c_void;
//     }
//     Ok(0)
// }

// #[unsafe(no_mangle)]
// pub unsafe extern "C" fn draw_image_demo_texture(
//     handle: *const std::ffi::c_void,
// ) -> std::ffi::c_int {
//     draw_image_demo_texture_inner(handle).unwrap_or(-1)
// }

// fn draw_image_demo_texture_inner(handle: *const std::ffi::c_void) -> Result<i32> {
//     unsafe {
//         let res_img = Arc::from_raw(handle as *const Mutex<GluResourceImage>);
//         if let Ok(mut lock) = res_img.lock() {
//             lock.draw(None)?;
//         }
//         let _ = Arc::into_raw(res_img) as *const std::ffi::c_void;
//     }
//     Ok(0)
// }

// #[unsafe(no_mangle)]
// pub unsafe extern "C" fn destroy_image_demo(handle: *const std::ffi::c_void) {
//     let _ = unsafe { Arc::from_raw(handle as *const Mutex<GluResourceImage>) };
// }

// #[unsafe(no_mangle)]
// pub unsafe extern "C" fn run_image_app(
//     media_path: *const std::ffi::c_char,
//     width: i32,
//     height: i32,
//     init_image_demo: unsafe extern "C" fn(
//         path: *const std::ffi::c_char,
//         width: std::ffi::c_int,
//         height: std::ffi::c_int,
//     ) -> *const std::ffi::c_void,
//     register_image_demo_texture: unsafe extern "C" fn(
//         tex_y_id: std::ffi::c_uint,
//         tex_uv_id: std::ffi::c_uint,
//         handle: *const std::ffi::c_void,
//     ) -> std::ffi::c_int,
//     draw_image_demo_texture: unsafe extern "C" fn(handle: *const std::ffi::c_void) -> std::ffi::c_int,
//     destroy_image_demo: unsafe extern "C" fn(handle: *const std::ffi::c_void),
// ) -> std::ffi::c_int {
//     if let Err(_) = elogger!(mock_image_window(
//         media_path,
//         width,
//         height,
//         init_image_demo,
//         register_image_demo_texture,
//         draw_image_demo_texture,
//         destroy_image_demo,
//     )) {
//         return -1;
//     }
//     0
// }
