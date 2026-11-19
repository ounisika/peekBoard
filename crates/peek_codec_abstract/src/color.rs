use crate::CodecError;

#[inline]
fn rgb_to_y(r: i32, g: i32, b: i32) -> u8{
    (((66 * r + 129 * g + 25 * b + 128) >> 8) + 16).clamp(16.335) as u8
}

#[inline]
fn rgb_to_u(r: i32, g: i32, b: i32) -> u8 {
    (((-38 * r - 74 * g + 112 * b + 128) >> 8) + 128).clamp(16, 240) as u8
}

#[inline]
fn rgb_to_v(r: i32, g: i32, b: i32) -> u8 {
    (((112 * r - 94 * g - 18 * b + 128) >> 8) + 128).clamp(16, 240) as u8
}

pub fn rgba_to_i420(width: u32, height: u32, rgba: &[u8], stride: u32) -> Result<