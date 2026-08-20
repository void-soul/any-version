//! 剪贴板图片处理：CF_DIB → PNG 文件 + 缩略图

use std::path::Path;

/// 将 DIB 像素数据解码为 RGBA8 像素（bottom-up）
///
/// 支持：24bpp BI_RGB、32bpp BI_RGB / BI_BITFIELDS。
/// BI_BITFIELDS 时优先按掩码提取通道（DIBV5 的常见做法，如 RGBA 顺序），
/// 掩码缺失或全 0 时回退按 BGRA 处理。
/// 返回 (width, height, rgba)
#[cfg(windows)]
pub fn dib_to_rgba(
    width: i32,
    raw_height: i32,
    bpp: u16,
    compression: u32,
    masks: [u32; 4], // [R, G, B, A] 掩码，BI_BITFIELDS 时可能有效
    pixels: &[u8],
) -> Option<(u32, u32, Vec<u8>)> {
    let w = width.max(0) as usize;
    let h = raw_height.abs() as usize;
    if w == 0 || h == 0 {
        return None;
    }
    let top_down = raw_height < 0;
    let bytes_per_pixel = (bpp / 8) as usize;
    if bytes_per_pixel < 3 {
        return None;
    }
    let row_size = ((w * bytes_per_pixel + 3) / 4) * 4;
    let mut rgba = vec![0u8; w * h * 4];

    // BI_BITFIELDS=3：若能拿到有效掩码则按掩码提取通道
    let use_masks = compression == 3
        && masks[0] != 0
        && masks[1] != 0
        && masks[2] != 0
        && (masks[0] | masks[1] | masks[2] | masks[3]) == u32::MAX;

    for y in 0..h {
        let src_row = if top_down { y } else { h - 1 - y };
        let src_off = src_row * row_size;
        if src_off + w * bytes_per_pixel > pixels.len() {
            continue;
        }
        let dst_off = y * w * 4;
        for x in 0..w {
            let si = src_off + x * bytes_per_pixel;
            let di = dst_off + x * 4;
            if use_masks && bytes_per_pixel == 4 {
                let px = u32::from_le_bytes([pixels[si], pixels[si + 1], pixels[si + 2], pixels[si + 3]]);
                rgba[di] = channel(px, masks[0]);
                rgba[di + 1] = channel(px, masks[1]);
                rgba[di + 2] = channel(px, masks[2]);
                rgba[di + 3] = if masks[3] != 0 { channel(px, masks[3]) } else { 255 };
            } else {
                // BGR(A) -> RGBA
                rgba[di] = pixels[si + 2];
                rgba[di + 1] = pixels[si + 1];
                rgba[di + 2] = pixels[si];
                rgba[di + 3] = if bytes_per_pixel >= 4 { pixels[si + 3] } else { 255 };
            }
        }
    }
    Some((w as u32, h as u32, rgba))
}

/// 按掩码提取通道值（掩码为连续位，取出后缩放到 0..255）
fn channel(px: u32, mask: u32) -> u8 {
    if mask == 0 {
        return 0;
    }
    let shift = mask.trailing_zeros();
    let width = 32 - shift - mask.leading_zeros();
    let v = (px & mask) >> shift;
    if width >= 8 {
        (v >> (width - 8)) as u8
    } else {
        // 通道位宽不足 8 位时按位权放大
        ((v as u32 * 255 + ((1u32 << width) / 2)) / ((1u32 << width) - 1)).min(255) as u8
    }
}

/// 保存 RGBA 像素为 PNG，同时生成缩略图（最长边 max_thumb_px）
pub fn save_rgba_png(
    dir: &Path,
    stem: &str,
    width: u32,
    height: u32,
    rgba: &[u8],
    max_thumb_px: u32,
) -> Result<(String, String), String> {
    use image::{ImageBuffer, RgbaImage};

    let full_path = dir.join(format!("{}.png", stem));
    let thumb_path = dir.join(format!("{}_thumb.png", stem));

    let img: RgbaImage = ImageBuffer::from_raw(width, height, rgba.to_vec())
        .ok_or_else(|| "图片像素数据不合法".to_string())?;
    img.save(&full_path)
        .map_err(|e| format!("保存剪贴板图片失败: {}", e))?;

    // 缩略图：等比缩放
    let (tw, th) = if width > height {
        (max_thumb_px, (height as f32 * max_thumb_px as f32 / width as f32).round() as u32)
    } else {
        ((width as f32 * max_thumb_px as f32 / height as f32).round() as u32, max_thumb_px)
    };
    let tw = tw.max(1);
    let th = th.max(1);
    let thumb: RgbaImage = image::imageops::resize(&img, tw, th, image::imageops::FilterType::Lanczos3);
    thumb
        .save(&thumb_path)
        .map_err(|e| format!("保存剪贴板缩略图失败: {}", e))?;

    // 返回相对 data_dir/clipboard 的路径
    Ok((
        format!("images/{}", full_path.file_name().unwrap_or_default().to_string_lossy()),
        format!("images/{}", thumb_path.file_name().unwrap_or_default().to_string_lossy()),
    ))
}

/// 保存原始 PNG 字节（CopyQ 式：剪贴板里原本是 PNG 就原样保留），
/// 同时解码生成缩略图（最长边 max_thumb_px）
pub fn save_png_bytes(
    dir: &Path,
    stem: &str,
    png_bytes: &[u8],
    max_thumb_px: u32,
) -> Result<(String, String), String> {
    use image::RgbaImage;

    let full_path = dir.join(format!("{}.png", stem));
    let thumb_path = dir.join(format!("{}_thumb.png", stem));

    std::fs::write(&full_path, png_bytes)
        .map_err(|e| format!("保存剪贴板图片失败: {}", e))?;

    // 解码生成缩略图
    let img = image::load_from_memory(png_bytes)
        .map_err(|e| format!("解码剪贴板 PNG 失败: {}", e))?
        .to_rgba8();
    let (width, height) = img.dimensions();
    let (tw, th) = if width > height {
        (max_thumb_px, (height as f32 * max_thumb_px as f32 / width as f32).round() as u32)
    } else {
        ((width as f32 * max_thumb_px as f32 / height as f32).round() as u32, max_thumb_px)
    };
    let tw = tw.max(1);
    let th = th.max(1);
    let thumb: RgbaImage = image::imageops::resize(&img, tw, th, image::imageops::FilterType::Lanczos3);
    thumb
        .save(&thumb_path)
        .map_err(|e| format!("保存剪贴板缩略图失败: {}", e))?;

    Ok((
        format!("images/{}", full_path.file_name().unwrap_or_default().to_string_lossy()),
        format!("images/{}", thumb_path.file_name().unwrap_or_default().to_string_lossy()),
    ))
}

/// 生成随机文件名
pub fn new_stem() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("img_{:x}_{:08x}", nanos, rand_helper())
}

fn rand_helper() -> u32 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    t ^ (t << 13) ^ (t >> 7)
}
