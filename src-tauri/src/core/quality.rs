use image::{DynamicImage, GenericImageView, GrayImage};

use crate::core::dedupe::ImageRecord;
use crate::core::settings::{PerfProfile, SceneMode};

/// Laplacian variance sharpness (higher = sharper).
pub fn sharpness_laplacian(img: &DynamicImage) -> f64 {
    let gray = to_center_gray(img);
    let (w, h) = gray.dimensions();
    if w < 3 || h < 3 {
        return 0.0;
    }

    let mut sum = 0.0f64;
    let mut sum_sq = 0.0f64;
    let mut n = 0u64;

    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let c = gray.get_pixel(x, y).0[0] as f64;
            let up = gray.get_pixel(x, y - 1).0[0] as f64;
            let dn = gray.get_pixel(x, y + 1).0[0] as f64;
            let lf = gray.get_pixel(x - 1, y).0[0] as f64;
            let rt = gray.get_pixel(x + 1, y).0[0] as f64;
            let lap = up + dn + lf + rt - 4.0 * c;
            sum += lap;
            sum_sq += lap * lap;
            n += 1;
        }
    }
    if n == 0 {
        return 0.0;
    }
    let mean = sum / n as f64;
    (sum_sq / n as f64) - (mean * mean)
}

fn to_center_gray(img: &DynamicImage) -> GrayImage {
    let (w, h) = img.dimensions();
    let cw = (w * 2 / 3).max(8);
    let ch = (h * 2 / 3).max(8);
    let x0 = (w.saturating_sub(cw)) / 2;
    let y0 = (h.saturating_sub(ch)) / 2;
    let cropped = image::imageops::crop_imm(img, x0, y0, cw, ch).to_image();
    DynamicImage::ImageRgba8(cropped).to_luma8()
}

/// Face clarity proxy: upper-center sharpness (Vision can replace later).
pub fn face_clarity_proxy(img: &DynamicImage) -> f64 {
    let (w, h) = img.dimensions();
    let cw = (w / 2).max(8);
    let ch = (h / 2).max(8);
    let x0 = (w.saturating_sub(cw)) / 2;
    let y0 = (h.saturating_sub(ch)) / 3;
    let cropped = image::imageops::crop_imm(img, x0, y0, cw, ch).to_image();
    let sub = DynamicImage::ImageRgba8(cropped);
    sharpness_laplacian(&sub)
}

#[derive(Debug, Clone)]
pub struct ScoredMember {
    pub index: usize,
    pub score: f64,
    pub sharpness: f64,
    pub face: f64,
}

pub fn score_member(img: &DynamicImage, scene: SceneMode, perf: PerfProfile) -> (f64, f64, f64) {
    let s = sharpness_laplacian(img);
    let f = if scene == SceneMode::Portrait && perf.face_scoring() {
        face_clarity_proxy(img)
    } else {
        0.0
    };
    let score = match scene {
        SceneMode::Portrait if perf.face_scoring() => 0.45 * s + 0.55 * f,
        _ => s,
    };
    (score, s, f)
}

/// Pick exactly one winner. Prefer RAW/DNG when scores are close.
pub fn pick_winner(group: &[usize], records: &[ImageRecord], scores: &[ScoredMember]) -> usize {
    assert!(!group.is_empty());
    let members: Vec<&ScoredMember> = scores
        .iter()
        .filter(|s| group.contains(&s.index))
        .collect();
    assert!(!members.is_empty());

    let max_score = members
        .iter()
        .map(|m| m.score)
        .fold(f64::NEG_INFINITY, f64::max);
    let min_score = members
        .iter()
        .map(|m| m.score)
        .fold(f64::INFINITY, f64::min);
    let score_range = (max_score - min_score).max(1e-9);
    let epsilon = score_range * 0.05;

    let near_top: Vec<&ScoredMember> = members
        .iter()
        .copied()
        .filter(|m| (max_score - m.score) <= epsilon)
        .collect();

    let prefer_raw: Vec<&ScoredMember> = near_top
        .iter()
        .copied()
        .filter(|m| records[m.index].is_raw_or_dng)
        .collect();

    let candidates = if !prefer_raw.is_empty() {
        prefer_raw
    } else {
        near_top
    };

    candidates
        .into_iter()
        .max_by(|a, b| {
            let ra = &records[a.index];
            let rb = &records[b.index];
            a.score
                .partial_cmp(&b.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| (ra.preview_w * ra.preview_h).cmp(&(rb.preview_w * rb.preview_h)))
                .then_with(|| ra.size.cmp(&rb.size))
                .then_with(|| ra.path.cmp(&rb.path))
        })
        .map(|m| m.index)
        .unwrap_or(group[0])
}

pub fn normalize_group_scores(scores: &mut [ScoredMember], group: &[usize]) {
    let idxs: Vec<usize> = scores
        .iter()
        .enumerate()
        .filter(|(_, s)| group.contains(&s.index))
        .map(|(i, _)| i)
        .collect();
    if idxs.is_empty() {
        return;
    }
    let max_s = idxs
        .iter()
        .map(|&i| scores[i].sharpness)
        .fold(0.0f64, f64::max)
        .max(1e-9);
    let max_f = idxs
        .iter()
        .map(|&i| scores[i].face)
        .fold(0.0f64, f64::max)
        .max(1e-9);

    let all_face_zero = idxs.iter().all(|&j| scores[j].face == 0.0);

    for &i in &idxs {
        let s = scores[i].sharpness / max_s;
        let f = scores[i].face / max_f;
        if all_face_zero {
            scores[i].score = s;
        } else {
            scores[i].score = 0.45 * s + 0.55 * f;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Rgb};
    use std::path::PathBuf;

    fn blurry() -> DynamicImage {
        DynamicImage::ImageRgb8(ImageBuffer::from_pixel(64, 64, Rgb([128u8, 128, 128])))
    }

    fn sharpish() -> DynamicImage {
        DynamicImage::ImageRgb8(ImageBuffer::from_fn(64, 64, |x, y| {
            let v = if (x + y) % 2 == 0 { 0 } else { 255 };
            Rgb([v, v, v])
        }))
    }

    #[test]
    fn sharp_beats_blurry() {
        let s = sharpness_laplacian(&sharpish());
        let b = sharpness_laplacian(&blurry());
        assert!(s > b, "sharp {s} should exceed blurry {b}");
    }

    #[test]
    fn pick_one_when_equal_prefers_raw() {
        let records = vec![
            ImageRecord {
                path: PathBuf::from("a.jpg"),
                blake3: "x".into(),
                phash: Some(0),
                dhash: None,
                preview_w: 100,
                preview_h: 100,
                size: 1000,
                is_raw_or_dng: false,
            },
            ImageRecord {
                path: PathBuf::from("b.arw"),
                blake3: "x".into(),
                phash: Some(0),
                dhash: None,
                preview_w: 100,
                preview_h: 100,
                size: 1000,
                is_raw_or_dng: true,
            },
        ];
        let scores = vec![
            ScoredMember {
                index: 0,
                score: 1.0,
                sharpness: 1.0,
                face: 1.0,
            },
            ScoredMember {
                index: 1,
                score: 1.0,
                sharpness: 1.0,
                face: 1.0,
            },
        ];
        assert_eq!(pick_winner(&[0, 1], &records, &scores), 1);
    }

    #[test]
    fn pick_exactly_one_from_identical() {
        let records = vec![
            ImageRecord {
                path: PathBuf::from("a.jpg"),
                blake3: "x".into(),
                phash: Some(0),
                dhash: None,
                preview_w: 50,
                preview_h: 50,
                size: 100,
                is_raw_or_dng: false,
            },
            ImageRecord {
                path: PathBuf::from("b.jpg"),
                blake3: "x".into(),
                phash: Some(0),
                dhash: None,
                preview_w: 50,
                preview_h: 50,
                size: 200,
                is_raw_or_dng: false,
            },
        ];
        let scores = vec![
            ScoredMember {
                index: 0,
                score: 5.0,
                sharpness: 5.0,
                face: 0.0,
            },
            ScoredMember {
                index: 1,
                score: 5.0,
                sharpness: 5.0,
                face: 0.0,
            },
        ];
        assert_eq!(pick_winner(&[0, 1], &records, &scores), 1);
    }
}
