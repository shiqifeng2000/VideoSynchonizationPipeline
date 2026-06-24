# 2026_Heterogeneous_Space_AIGC

### 该项目为解码直绘功能，将gpu解码出来的视频帧直接绘制到应用层提供的texture上，达到无缝同步的效果

### 实现流程如下:
- 确定要播放的视频组和播放的窗口数目
- 将播放视频的磁盘地址video_paths和窗口数view_ports传入<init_player>中，如果初始化失败，则返回null。初始化结束后，内部会生成video_paths个解码线程和1个同步队列线程。
- 应用层在必要的opengl准备工作后进行texture注册，如果应用层无法获取分辨率，可以用get_ratios获取宽高。获取宽高之后，需要执行[gl.create_texture] + [gl.bind_texture] + [gl.tex_image_2d]方法，图片类型RGBA进行texture的申请。申请之后获得的texture_ids用<register_textures>将opengl和cuda resource绑定
- 应用层准备工作(如上述工作和vertex array)都结束后，执行<start_player>启动工作流程
- 工作流程需要分解为如下几步:
  - 执行<load_frames>从同步队列中批量获取数据，该方法内部有帧率控制，如果为空可能是刷新频率高于解码频率，如果为空则不绘制
  - 从<load_frames>获取到cuda_frames之后，依次执行 [gl.bind_texture] + <draw_frame>，将帧数据拷贝到texture上
  - 启用着色器[gl.use_program]和必要的gl操作，最后[gl.draw_arrays]将texture绘制到画布上，可能需要的opengl api [gl.clear_color],[gl.clear],[gl.use_program], [gl.uniform_1_i32], [gl.active_texture], [gl.bind_texture], [gl.bind_vertex_array], [gl.draw_arrays]
  - 以上工作成功执行后，回收cuda_frames帧<recycle_frames>，如此形成一个环形队列
- 如果需要暂停，执行<pause_player>，再次执行解除暂停，不建议频繁操作
- 如果应用需要退出，需要保证如下流程，否则不保证不会显存泄漏
  - 执行<unregister_textures>，将opengl和cuda绑定资源解除
  - 执行<stop_player>，如此通知解码线程和队列线程停止
  - 执行<terminate_player>，将cuda资源进行安全回收
- 以上操作都是跨线程互斥的，内部有mutex锁控制

### 已知的问题
- 使用了nvidia驱动 591, cuda sdk 12.4进行编译，理论上向后兼容，如果部署机版本差异严重请考虑调整版本或者在新目标机上编译

### 编译步骤
- 调整 [build.rs]中 `cargo:rustc-link-searc` 链接库路径为本地cuda sdk路径
- 调整 [.cargo/config.toml]中, `CUDA_PATH` 链接库路径为本地cuda sdk路径
- 确保 [.cargo/config.toml]中, FFmpeg 依赖库存在, 兼容且有效
- 执行 `cargo build --release`, 并将 [target/release/*glue*] 所有文件导出，即为依赖的dll文件

