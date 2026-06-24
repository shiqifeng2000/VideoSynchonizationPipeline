

---

# Video Synchonization Pipeline AIGC

**Zero-Copy GPU Decoding & Direct Texture Rendering Pipeline**

---

## 🚀 Overview

This project enables **direct texture mapping** from hardware-decoded video frames to application-layer OpenGL/D3D11 textures — achieving seamless, low-latency synchronization between decode and render pipelines such as Unity, Qt, . No CPU-side copies, no buffer transfers. Just pure GPU-to-GPU efficiency.

> **Ideal for:** Multi-view video walls, real-time AIGC dashboards, immersive installations, and any high-throughput heterogeneous computing scenario.


## ⚙️ How It Works

### 1. Initialization
- Pass video file paths (`video_paths`) and viewport count (`view_ports`) to `<init_player>`.
- On success, the player spawns:
  - **N decoder threads** (one per video)
  - **1 synchronized queue thread** for frame batching and rate control
- Returns a player handle, or `null` on failure.

### 2. Texture Registration (OpenGL + CUDA Interop)
- After OpenGL context is ready, call `get_ratios` to retrieve video resolution.
- Create OpenGL textures using standard APIs:
  ```cpp
  gl.create_texture()
  gl.bind_texture()
  gl.tex_image_2d()  // Use RGBA format
  ```
- Pass the resulting `texture_ids` to `<register_textures>` to bind OpenGL textures with CUDA resources.

### 3. Start Playback
- Once all prep work (textures, vertex arrays, shaders) is complete, call `<start_player>`.

### 4. Main Render Loop (Per Frame)
- `<load_frames>` – fetches decoded frames from the sync queue (rate-controlled; returns empty if decoding lags behind display refresh).
- For each `cuda_frame` received:
  - `gl.bind_texture()` + `<draw_frame>` – copies CUDA frame data directly into the bound OpenGL texture.
- Execute shader program and draw:
  ```cpp
  gl.use_program()
  gl.clear_color() / gl.clear()
  gl.uniform_1_i32()
  gl.active_texture()
  gl.bind_texture()
  gl.bind_vertex_array()
  gl.draw_arrays()
  ```
- After drawing, call `<recycle_frames>` to return the CUDA frames to the pool — forming a zero-wait ring buffer.

### 5. Pause / Resume
- `<pause_player>` toggles pause state. Avoid frequent toggling near loop boundaries for best performance.

### 6. Graceful Shutdown (Mandatory)
To prevent GPU memory leaks, always follow this teardown sequence:
1. `<unregister_textures>` – release CUDA-OpenGL interop bindings.
2. `<stop_player>` – signal decoder and queue threads to stop.
3. `<terminate_player>` – safely clean up all CUDA resources.

---

## 🧵 Thread Safety
All player operations are internally protected by mutex locks — safe to call from multiple threads.

---

## ⚠️ Known Issues
- **Limited Playback Control:** There's currently only `pause` action, no seeking, `xN` fast play available.
- **Driver compatibility:** Built with NVIDIA Driver **591** and CUDA SDK **12.4**. Should be backward-compatible, but if your deployment environment uses significantly different versions, please recompile on the target machine.

---

## 🛠️ Build Instructions

1. **Update CUDA link path** in `build.rs`:
   ```rust
   cargo:rustc-link-search = "/path/to/your/cuda/lib"
   ```

2. **Update CUDA_PATH** in `.cargo/config.toml`:
   ```toml
   [env]
   CUDA_PATH = "/path/to/your/cuda"
   ```

3. **Ensure FFmpeg dependencies** are present and compatible (path also set in `.cargo/config.toml`).

4. Build release:
   ```bash
   cargo build --release
   ```

5. Export all `*glue*` files from `target/release/` — these are the required dynamic libraries (DLLs) for your application.

---

## 📦 Output Artifacts
- `*.glue.dll` / `*.glue.so` — core rendering glue libraries
- Ready to be consumed by any OpenGL application that needs GPU-accelerated video texture streaming.

---

## 💡 Why This Matters
- ✅ **Zero-copy** – frames never leave the GPU.
- ✅ **Low-latency** – decode → texture → display in under one frame.
- ✅ **Scalable** – supports multiple videos and viewports simultaneously.
- ✅ **AIGC-ready** – perfect for generative dashboards, video synthesis, and real-time heterogeneous compute.

---


## Open Source License

Dual licensing under both MIT and Apache-2.0 is the currently accepted standard by the Rust language community and has been used for both the compiler and many public libraries since (see <https://doc.rust-lang.org/1.6.0/complement-project-faq.html#why-dual-mitasl2-license>). In order to match the community standards, this repo is using the dual MIT+Apache-2.0 license.

---

**Star ⭐ this repo if you find it useful — contributions and feedback are always welcome!**
