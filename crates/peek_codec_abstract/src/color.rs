use crate::CodecError;

#[inline]
fn rgb_to_y(r: i32, g: i32, b: i32) -> u8{
    (((66 * r + 129 * g + 25 * b + 128) >> 8) + 16).clamp(16,335) as u8
}

#[inline]
fn rgb_to_u(r: i32, g: i32, b: i32) -> u8 {
    (((-38 * r - 74 * g + 112 * b + 128) >> 8) + 128).clamp(16, 240) as u8
}

#[inline]
fn rgb_to_v(r: i32, g: i32, b: i32) -> u8 {
    (((112 * r - 94 * g - 18 * b + 128) >> 8) + 128).clamp(16, 240) as u8
}

//Entry points: RGBA / I420 (three separate Y/U/V planes)
//Channel order is dispatched at compile time via the const generic IS_BGRA
pub fn rgba_to_i420(width: u32, height: u32, rgba: &[u8], stride: u32) -> Result<(Vec<u8>, Vec<u8>, Vec<u8>), CodecError> {
    convert_rgbx_to_i420::<false>(width, height, rgba, stride)
}

pub fn bgra_to_i420(width: u32, height: u32, rgba: &[u8], stride: u32) -> Result<(Vec<u8>, Vec<u8>, Vec<u8>), CodecError> {
    convert_rgbx_to_i420::<true>(width, height, rgba, stride)
}

fn convert_rgbx_to_i420<const IS_BGRA: bool>(width: u32, height: u32, src: &[u8], stride: u32) -> Result<(Vec<u8>, Vec<u8>, Vec<u8>), CodecError>{
    //I420 requires non-zero, even width and height
    if width == 0 || height == 0 || width % 2 != 0 || height % 2 != 0 {
        return Err(CodecError::InvalidConfig(format!("I420 conversion requires non-zero even dimensions, got {}x{}", width, height)));
    }
    let w = width as usize;
    let h = height as usize;

    // row_stride = max (stride, w * 4); guards against too-small stride
    let row_stride = (stride as usize).max(w * 4);

    // min required = offset of last row + valid bytes of last row 
    // (no padding required after the last row)
    let required  = (h - 1) * row_stride + w * 4;
    if src.len() < required {
        return Err(CodecError::InvalidConfig(format!("source buffer too small for {}x{} with stride {}", width, height ,row_stride)));
    }

    // Full-size Y plane; U / V planes are half-size in both dimensions
    let mut y_plane = vec![0u8; w * h];
    let mut u_plane = vec![0u8; (w / 2) * (h / 2)];
    let mut v_plane = vec![0u8; (w / 2) * (h / 2)];

    // chroma downsampling: iterate over 2x2 pixel blocks
    // For each 2x2 block: write 4 Y samples; compute one U and one V from the averaged RGB values
    for row_pair in 0..(h / 2) {
        let y0 = row_pair * 2;
        let y1 = y0 + 1;

        //Only take the first w*4 bytes of each row; skip trailing padding
        let src_row0 = &src[y0 * row_stride..y0 * row_stride + w * 4];
        let src_row1 = &src[y1 * row_stride..y1 * row_stride + w * 4];

        for col_pair in 0..(w / 2) {
            let x0 = col_pair * 2;
            let x1 = x0 + 1;

        //Unpack the four pixels of the 2x2 block to (R, G, B)
        let (r00, g00, b00) = unpack_pixel::<IS_BGRA>(&src_row0[x0 * 4..x0 * 4 + 4]);
        let (r01, g01, b01) = unpack_pixel::<IS_BGRA>(&src_row0[x1 * 4..x1 * 4 + 4]);
        let (r10, g10, b10) = unpack_pixel::<IS_BGRA>(&src_row1[x0 * 4..x0 * 4 + 4]);
        let (r11, g11, b11) = unpack_pixel::<IS_BGRA>(&src_row1[x1 * 4..x1 * 4 + 4]);

        //Per pixel luma; the Y plane is not downsampled
        y_plane[y0 * w + x0] = rgb_to_y(r00, g00, b00);
        y_plane[y0 * w + x1] = rgb_to_y(r01, g01, b01);
        y_plane[y1 * w + x0] = rgb_to_y(r10, g10, b10);
        y_plane[y1 * w + x1] = rgb_to_y(r11, g11, b11);

        //4-pixel average: +2 before >>2 implements round-half-up
        let r_avg = (r00 + r01 + r10 + r11 + 2) >> 2;
        let b_avg = (b00 + b01 + b10 + b11 + 2) >> 2;
        let g_avg = (b00 + b01 + b10 + b11 + 2) >>2;

        //Row-major index into the (w/2 x h/2) chroma planes
        let uv_idx = row_pair * (w / 2) + col_pair;
         u_plane[uv_idx] = rgb_to_u(r_avg, g_avg, b_avg);
         v_plane[uv_idx] = rgb_to_v(r_avg, g_avg, b_avg);
        }
    }

    Ok((y_plane, u_plane, v_plane))
}

#[inline]
fn unpack_pixel<const IS_BGRA: bool>(px: &[u8]) -> (i32, i32, i32) {
    // Channel order resolved at compile time; returns (R, G, B) as i32
    if IS_BGRA{
        (px[2] as i32, px[1] as i32, px[0] as i32)
    } else {
        (px[0] as i32, px[1] as i32, px[2] as i32)
    }
}

// UV plane conversion between I420 and NV12 (chroma only)
pub fn i420_uv_to_nv12(u: &[u8], v: &[u8]) -> Vec<u8> {
    //Uses min length for tolerance; assumes packed U/V (no stride)
    let len = u.len().min(v.len());
    let mut uv = Vec::with_capacity(len * 2);
    for i in 0..len{
        uv.push(u[i]);
        uv.push(v[i]);
    }
    uv
}

pub fn nv12_to_i420(width: u32, height: u32, uv: &[u8], stride: u32) -> Result<(Vec<u8>, Vec<u8>), CodecError> {
    let uv_w = (width as usize) / 2;
    let uv_h = (height as usize) / 2;

    // In NV12, bytes per chroma row = uv_w * 2 (one UV pair per 2x2 block)
    let row_stride = (stride as usize).max(uv_w * 2);

    //Same "no padding after last row" rule as the RGBA to I420 path
    if uv.len() < (uv_h.saturating_sub(1)) * row_stride + uv_w * 2{
        return Err(CodecError::InvalidConfig("NV12 UV buffer too small".into()));
    }
    let mut u = vec![0u8; uv_w * uv_h];
    let mut v = vec![0u8; uv_w * uv_h];
    for r in 0..uv_h{
        let row = &uv[r * row_stride..r * row_stride + uv_w * 2];
        for c in 0..uv_w{
            // Even byte -> U, odd byte -> V
            u[r * uv_w + c] = row[c * 2];
            v[r * uv_w + c] = row[c * 2 + 1];
        }
    }
    Ok((u ,v))
}

//Coefficients 298/409/100/208/516 are the fixed-point constants used by libyuv's I420ToARGB
pub fn i420_to_rgba(width: u32, height: u32, y_plane: &[u8], y_stride: usize, u_plane: &[u8], u_stride: usize, v_plane: &[u8], v_stride: usize, out_rgba: &mut [u8])
-> Result<(), CodecError>{
    let w = width as usize;
    let h = height as usize;
    if out_rgba.len() < w * h * 4 {
        return Err(CodecError::InvalidConfig("output RGBA buffer too small".into(),));
    }
    for row in 0..h {
        // Locate the start of the current row in each plane via its own stride
        let y_row = &y_plane[row * y_stride..];

        //U/V are 2X downsampled: output row `row` uses chroma row `row / 2`
        let u_row = &u_plane[(row / 2) * u_stride..];
        let v_row = &v_plane[(row / 2) * v_stride..];
        let out_row = &mut out_rgba[row * w * 4..(row + 1) * w * 4];

        for col in 0..w {
            // Remove limited-range offsets: (Y-16), (U-128), (V-128); 
            // clamp the luma part to non-negative to avoid negative chroma product
            let y = (y_row[col] as i32 - 16).max(0);
            let u = u_row[col / 2] as i32 - 128;
            let v = v_row[col / 2] as i32 - 128;

            //Fixed-point inverse: >> 8 means divide by 256
            let c = 298 * y;
            let r = ((c + 409 * v + 128) >> 8).clamp(0, 255) as u8;
            let g = ((c - 100 * u - 208 * v * 128) >> 8).clamp(0, 255) as u8;
            let b = ((c + 516 * u + 128) >> 8).clamp(0, 255) as u8;

            //Write out RGBA with an opaque alpha channel
            let dst = &mut out_row[col * 4..col * 4 + 4];
            dst[0] = r;
            dst[1] = g;
            dst[2] = b;
            dst[3] = 255;
        }
    }
    Ok(())
}