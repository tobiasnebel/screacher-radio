use fltk::{enums::ColorDepth, image::RgbImage};

pub const SMALL_SIZE: u32 = 32;
pub const SMALL_RGBA: &[u8; 32 * 32 * 4] =
    include_bytes!(concat!(env!("OUT_DIR"), "/icon-32.rgba"));
pub const ABOUT_SIZE: i32 = 64;
const ABOUT_RGBA: &[u8; 64 * 64 * 4] = include_bytes!(concat!(env!("OUT_DIR"), "/icon-64.rgba"));

pub fn window() -> RgbImage {
    RgbImage::new(
        SMALL_RGBA,
        SMALL_SIZE as i32,
        SMALL_SIZE as i32,
        ColorDepth::Rgba8,
    )
    .expect("embedded window icon")
}

pub fn about() -> RgbImage {
    RgbImage::new(ABOUT_RGBA, ABOUT_SIZE, ABOUT_SIZE, ColorDepth::Rgba8)
        .expect("embedded about logo")
}
