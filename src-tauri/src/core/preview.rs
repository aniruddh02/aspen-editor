use image::{DynamicImage, GenericImageView, ImageFormat};
use std::io::Cursor;
use std::path::Path;
use std::process::Command;

use crate::core::discover::is_raw_ext;

/// Load a downscaled RGB preview for hashing/scoring.
pub fn load_preview(path: &Path, max_long_edge: u32) -> anyhow::Result<DynamicImage> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    let img = if is_raw_ext(&ext) {
        load_raw_preview(path)?
    } else if matches!(ext.as_str(), "heic" | "heif") {
        load_heic_via_sips(path)?
    } else {
        image::open(path)?
    };

    Ok(downscale(img, max_long_edge))
}

fn downscale(img: DynamicImage, max_long_edge: u32) -> DynamicImage {
    let (w, h) = img.dimensions();
    let long = w.max(h);
    if long <= max_long_edge {
        return img;
    }
    let scale = max_long_edge as f32 / long as f32;
    let nw = ((w as f32) * scale).round().max(1.0) as u32;
    let nh = ((h as f32) * scale).round().max(1.0) as u32;
    img.resize(nw, nh, image::imageops::FilterType::Triangle)
}

/// Extract largest embedded JPEG from RAW by scanning SOI/EOI markers.
fn load_raw_preview(path: &Path) -> anyhow::Result<DynamicImage> {
    let data = std::fs::read(path)?;
    if let Some(jpeg) = find_largest_jpeg(&data) {
        // Tiny thumbnails are poor for focus scoring; prefer ImageIO/sips when possible.
        if jpeg.len() > 32_768 {
            if let Ok(img) = image::load_from_memory(jpeg) {
                return Ok(img);
            }
        }
    }
    load_via_sips(path).or_else(|sips_err| {
        if let Some(jpeg) = find_largest_jpeg(&data) {
            image::load_from_memory(jpeg).map_err(|e| {
                anyhow::anyhow!(
                    "RAW preview failed (sips: {sips_err}; embedded jpeg: {e}) for {}",
                    path.display()
                )
            })
        } else {
            Err(anyhow::anyhow!(
                "no usable RAW preview for {} ({sips_err})",
                path.display()
            ))
        }
    })
}

fn find_largest_jpeg(data: &[u8]) -> Option<&[u8]> {
    let mut best: Option<&[u8]> = None;
    let mut i = 0;
    while i + 1 < data.len() {
        if data[i] == 0xFF && data[i + 1] == 0xD8 {
            if let Some(end) = find_eoi(data, i + 2) {
                let slice = &data[i..=end];
                if slice.len() > best.map(|b| b.len()).unwrap_or(0) && slice.len() > 128 {
                    best = Some(slice);
                }
                i = end + 1;
                continue;
            }
        }
        i += 1;
    }
    best
}

fn find_eoi(data: &[u8], start: usize) -> Option<usize> {
    let mut j = start;
    while j + 1 < data.len() {
        if data[j] == 0xFF && data[j + 1] == 0xD9 {
            return Some(j + 1);
        }
        j += 1;
    }
    None
}

fn load_heic_via_sips(path: &Path) -> anyhow::Result<DynamicImage> {
    load_via_sips(path)
}

fn load_via_sips(path: &Path) -> anyhow::Result<DynamicImage> {
    let tmp = tempfile::Builder::new().suffix(".jpg").tempfile()?;
    let status = Command::new("sips")
        .args(["-s", "format", "jpeg", path.to_str().unwrap_or(""), "--out"])
        .arg(tmp.path())
        .status()?;
    if !status.success() {
        anyhow::bail!("sips failed converting {}", path.display());
    }
    Ok(image::open(tmp.path())?)
}

/// Encode a DynamicImage to JPEG bytes (for tests / debugging).
#[allow(dead_code)]
pub fn to_jpeg_bytes(img: &DynamicImage) -> anyhow::Result<Vec<u8>> {
    let mut buf = Cursor::new(Vec::new());
    img.write_to(&mut buf, ImageFormat::Jpeg)?;
    Ok(buf.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Rgb};

    #[test]
    fn find_jpeg_in_buffer() {
        let img: ImageBuffer<Rgb<u8>, Vec<u8>> =
            ImageBuffer::from_fn(32, 32, |x, y| Rgb([(x * 7) as u8, (y * 5) as u8, 80]));
        let dyn_img = DynamicImage::ImageRgb8(img);
        let jpeg = to_jpeg_bytes(&dyn_img).unwrap();
        let mut wrapped = vec![0u8; 100];
        wrapped.extend_from_slice(&jpeg);
        wrapped.extend_from_slice(&[0, 1, 2]);
        let found = find_largest_jpeg(&wrapped).unwrap();
        assert!(found.len() >= jpeg.len() - 10);
        assert_eq!(&found[0..2], &[0xFF, 0xD8]);
    }

    #[test]
    fn downscale_respects_max() {
        let img: ImageBuffer<Rgb<u8>, Vec<u8>> =
            ImageBuffer::from_pixel(800, 600, Rgb([10u8, 20, 30]));
        let out = downscale(DynamicImage::ImageRgb8(img), 200);
        let (w, h) = out.dimensions();
        assert!(w.max(h) <= 200);
    }
}
