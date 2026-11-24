use image::codecs::bmp::BmpDecoder;
use image::codecs::png::{CompressionType, FilterType, PngDecoder, PngEncoder};
use image::{DynamicImage, ImageDecoder, ImageEncoder, RgbaImage};
use std::io::Cursor;

pub(super) const MAX_PNG: usize = 32 * 1024 * 1024;
pub(super) const MAX_PIXELS: u64 = 24 * 1024 * 1024;
pub(super) const MAX_DIB: usize = MAX_PIXELS as usize * 4 + 64 * 1024;

pub(super) fn valid_dimensions(width: u32, height: u32) -> bool {
    width > 0 && height > 0 && u64::from(width) * u64::from(height) <= MAX_PIXELS
}

pub(super) enum Format { Png, Dib }

fn le_u32(bytes: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(offset..offset + 4)?.try_into().ok()?))
}

fn decode_limited(decoder: impl ImageDecoder) -> Option<RgbaImage> {
    let (w, h) = decoder.dimensions();
    if !valid_dimensions(w, h) || decoder.total_bytes() > MAX_PIXELS * 8 { return None; }
    Some(DynamicImage::from_decoder(decoder).ok()?.into_rgba8())
}

fn decode_dib(bytes: &[u8]) -> Option<RgbaImage> {
    let header = le_u32(bytes, 0)? as usize;
    if !matches!(header, 108 | 124) || le_u32(bytes, 16)? != 3 {
        return decode_limited(BmpDecoder::new_without_file_header(Cursor::new(bytes)).ok()?);
    }
    // V4/V5 的位域掩码在头部内。image 0.25.10 的无文件头模式会额外跳过 12 字节，
    // 因而为这两种 DIB 加上显式记录像素偏移的 BMP 文件头。
    let colors = le_u32(bytes, 32)? as usize;
    let mut offset = header.checked_add(colors.checked_mul(4)?)?;
    if header == 124 && le_u32(bytes, 112)? as usize == offset {
        let profile_size = le_u32(bytes, 116)? as usize;
        offset = offset.checked_add(profile_size.checked_add(3)? & !3)?;
    }
    if offset >= bytes.len() { return None; }
    let mut bmp = Vec::with_capacity(bytes.len() + 14);
    bmp.extend_from_slice(b"BM");
    bmp.extend_from_slice(&((bytes.len() + 14) as u32).to_le_bytes());
    bmp.extend_from_slice(&[0; 4]);
    bmp.extend_from_slice(&((offset + 14) as u32).to_le_bytes());
    bmp.extend_from_slice(bytes);
    decode_limited(BmpDecoder::new(Cursor::new(bmp)).ok()?)
}

impl Format {
    pub(super) fn decode(&self, bytes: &[u8]) -> Option<RgbaImage> {
        match self {
            Self::Png if bytes.len() <= MAX_PNG => decode_limited(PngDecoder::new(Cursor::new(bytes)).ok()?),
            Self::Dib if bytes.len() <= MAX_DIB => decode_dib(bytes),
            _ => None,
        }
    }
}

/// 剪贴板可能来自任意应用，只缓存重新编码的像素，不带原始 EXIF 等元数据。
pub(super) fn png(image: &RgbaImage) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    PngEncoder::new_with_quality(&mut out, CompressionType::Fast, FilterType::Adaptive)
        .write_image(image.as_raw(), image.width(), image.height(), image::ExtendedColorType::Rgba8).ok()?;
    (out.len() <= MAX_PNG).then_some(out)
}

/// CF_DIBV5 使用自顶向下的 BGRA 位图。声明透明通道，避免 Windows 按 BGRX 解释。
pub(super) fn dibv5(image: &RgbaImage) -> Vec<u8> {
    let mut bytes = vec![0; 124];
    bytes.reserve(image.as_raw().len());
    bytes[0..4].copy_from_slice(&124_u32.to_le_bytes());
    bytes[4..8].copy_from_slice(&(image.width() as i32).to_le_bytes());
    bytes[8..12].copy_from_slice(&(-(image.height() as i32)).to_le_bytes());
    bytes[12..14].copy_from_slice(&1_u16.to_le_bytes());
    bytes[14..16].copy_from_slice(&32_u16.to_le_bytes());
    bytes[16..20].copy_from_slice(&3_u32.to_le_bytes()); // BI_BITFIELDS
    bytes[20..24].copy_from_slice(&(image.as_raw().len() as u32).to_le_bytes());
    for (offset, value) in [(40, 0x00ff0000_u32), (44, 0x0000ff00), (48, 0x000000ff), (52, 0xff000000), (56, 0x73524742)] {
        bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }
    for px in image.pixels() { bytes.extend_from_slice(&[px[2], px[1], px[0], px[3]]); }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clipboard_formats_preserve_pixels_and_reject_oversize_headers() {
        let original = RgbaImage::from_raw(1, 2, vec![77, 123, 53, 255, 1, 2, 3, 120]).unwrap();
        let mut dib = dibv5(&original);
        assert_eq!(Format::Dib.decode(&dib), Some(original.clone()));
        assert_eq!(Format::Png.decode(&png(&original).unwrap()), Some(original));
        dib[4..8].copy_from_slice(&i32::MAX.to_le_bytes());
        assert!(Format::Dib.decode(&dib).is_none());
    }

    #[test]
    fn standard_24bit_dib_honors_padding_and_bottom_up_rows() {
        let mut dib = vec![0; 40];
        dib[..4].copy_from_slice(&40_u32.to_le_bytes());
        dib[4..8].copy_from_slice(&1_i32.to_le_bytes());
        dib[8..12].copy_from_slice(&2_i32.to_le_bytes());
        dib[12..14].copy_from_slice(&1_u16.to_le_bytes());
        dib[14..16].copy_from_slice(&24_u16.to_le_bytes());
        dib.extend_from_slice(&[6, 5, 4, 0, 3, 2, 1, 0]);
        let image = Format::Dib.decode(&dib).unwrap();
        assert_eq!(image.as_raw(), &[1, 2, 3, 255, 4, 5, 6, 255]);
    }
}
