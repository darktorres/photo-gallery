// NOTE: This file MUST contain only one test.
use std::thread;

use fast_image_resize::images::Image;
use fast_image_resize::{PixelType, ResizeAlg, ResizeOptions, Resizer};

#[cfg(feature = "rayon")]
#[test]
/// Test the case then `resample_nearest()` get stuck in lock if the number of threads less than 2.
fn run_resize_nearest_in_another_thread_without_global_rayon_pool() {
    // "Disable" global rayon's thread-pool (set number of threads to 1).
    rayon::ThreadPoolBuilder::new()
        .num_threads(1)
        .use_current_thread()
        .build_global()
        .unwrap();

    let handle = thread::spawn(|| {
        let src_image = Image::new(1366, 768, PixelType::U8x3);
        let mut dst_image = Image::new(256, 256, src_image.pixel_type());
        let options = ResizeOptions::new().resize_alg(ResizeAlg::Nearest);
        let mut resizer = Resizer::new();
        resizer
            .resize(&src_image, &mut dst_image, &options)
            .unwrap();
    });
    handle.join().unwrap();
}
