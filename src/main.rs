use glue::elogger;
// use glue::image::run_image_window;
use glue::video::run_video_window;

// fn generate_nv12(frame: usize) -> (Vec<u8>, Vec<u8>) {
//     let mut y = vec![0u8; (WIDTH * HEIGHT) as usize];
//     let mut uv = vec![0u8; (WIDTH * HEIGHT / 2) as usize];

//     // Create a simple test pattern
//     for row in 0..HEIGHT as usize {
//         for col in 0..WIDTH as usize {
//             let idx = row * WIDTH as usize + col;
//             // Create a gradient pattern
//             y[idx] = ((col as f32 / WIDTH as f32) * 255.0) as u8;
//         }
//     }

//     // Set UV to neutral gray (128, 128)
//     for i in (0..uv.len()).step_by(2) {
//         uv[i] = 128; // U
//         uv[i + 1] = (frame % 255) as u8; // V
//     }

//     (y, uv)
// }

fn main() {
    let _ = elogger!(run_video_window(0, 4));
    // let _ = elogger!(run_image_window());
}
