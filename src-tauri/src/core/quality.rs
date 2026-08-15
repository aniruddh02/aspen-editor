use image::{DynamicImage, GenericImageView, GrayImage};

use crate::core::dedupe::ImageRecord;
use crate::core::settings::{PerfProfile, SceneMode};

/// Laplacian variance sharpness (higher = sharper).
/// Uses the maximum of nine rule-of-thirds points so an off-center sharp
/// subject is not penalized just because the center is soft.
pub fn sharpness_laplacian(img: &DynamicImage) -> f64 {
    let (w, h) = img.dimensions();
    let cell_w = (w / 3).max(16);
    let cell_h = (h / 3).max(16);
    let step_x = (w.saturating_sub(cell_w)) / 2;
    let step_y = (h.saturating_sub(cell_h)) / 2;

    let mut sharpness_values: Vec<f64> = Vec::with_capacity(9);
    for row in 0..3 {
        for col in 0..3 {
            let x0 = col * step_x;
            let y0 = row * step_y;
            let region = crop_luma(img, x0, y0, cell_w, cell_h);
            sharpness_values.push(laplacian_variance(&region));
        }
    }

    // Take the top-3 average — captures the sharpest zone of the image
    // without being fooled by a single noise-spike or dead patch.
    sharpness_values.sort_by(|a, b| b.total_cmp(a));
    sharpness_values.iter().take(3).sum::<f64>() / 3.0
}

/// Detect if the subject (center of frame) is completely out-of-focus.
/// Returns a value in [0.0, 1.0] where 1.0 means subject is well-focused
/// and 0.0 means subject is severely blurred relative to background.
#[allow(dead_code)]
pub fn subject_focus_proxy(img: &DynamicImage) -> f64 {
    subject_focus_with_roi(img, &find_subject_roi(img))
}

fn subject_focus_with_roi(img: &DynamicImage, roi: &SubjectRoi) -> f64 {
    let center = laplacian_variance(&to_region_gray(img, Region::CenterHalf));
    let face_zone = laplacian_variance(&crop_luma(img, roi.x, roi.y, roi.w, roi.h));
    // Harmonic mean requires detail in both the likely face zone and broader
    // subject area. A single sharp boundary or detailed shirt cannot mask a
    // completely defocused face.
    let subject = 2.0 * face_zone * center / (face_zone + center + 1e-6);
    let border = border_sharpness(img);

    // Absolute subject detail is the primary signal. Comparing only with the
    // background wrongly rejects portraits containing textured borders or
    // watermarks, even when the subject is objectively sharp.
    let absolute = ((subject - 12.0) / 108.0).clamp(0.0, 1.0);

    // Relative focus remains useful for the classic failure mode where the
    // camera locks onto a detailed background instead of the subject.
    let ratio = subject / (border + 1e-6);
    let relative = ((ratio - 0.35) / 1.15).clamp(0.0, 1.0);

    (0.75 * absolute + 0.25 * relative).clamp(0.0, 1.0)
}

/// Average the four real border strips independently. Concatenating their
/// pixels creates fake seams, which massively inflates Laplacian variance.
fn border_sharpness(img: &DynamicImage) -> f64 {
    let (w, h) = img.dimensions();
    let strip_w = (w / 8).max(4).min(w);
    let strip_h = (h / 8).max(4).min(h);
    let strips = [
        crop_luma(img, 0, 0, w, strip_h),
        crop_luma(img, 0, h.saturating_sub(strip_h), w, strip_h),
        crop_luma(img, 0, 0, strip_w, h),
        crop_luma(img, w.saturating_sub(strip_w), 0, strip_w, h),
    ];
    strips.iter().map(laplacian_variance).sum::<f64>() / strips.len() as f64
}

/// Absolute blur detector. Returns 1.0 if image has any sharp region above the
/// usable threshold, 0.0 if the entire image is blurred (motion blur, defocus).
pub fn blur_confidence(img: &DynamicImage) -> f64 {
    let max_sharp = sharpness_laplacian(img);
    // Empirically: usable photos rarely have max regional sharpness below 40.
    // 20 = borderline usable, below 10 = motion-blurred / severely defocused.
    if max_sharp >= 60.0 {
        1.0
    } else if max_sharp >= 20.0 {
        ((max_sharp - 20.0) / 40.0).clamp(0.3, 1.0)
    } else {
        (max_sharp / 20.0 * 0.3).clamp(0.0, 0.3)
    }
}

/// Portrait-specific blur confidence. Global sharp regions can come from a
/// watermark, clothing, or background while the face itself is blurred, so
/// require usable detail in the likely face zone as well.
#[allow(dead_code)]
pub fn portrait_blur_confidence(img: &DynamicImage) -> f64 {
    portrait_blur_with_roi(img, &find_subject_roi(img))
}

fn portrait_blur_with_roi(img: &DynamicImage, roi: &SubjectRoi) -> f64 {
    let global = blur_confidence(img);
    let subject_detail = face_clarity_with_roi(img, roi);
    let subject = if subject_detail >= 180.0 {
        1.0
    } else if subject_detail >= 50.0 {
        0.3 + (subject_detail - 50.0) / 130.0 * 0.7
    } else {
        subject_detail / 50.0 * 0.3
    };
    global.min(subject.clamp(0.0, 1.0))
}

/// Face/expression proxy: sharpness of the adaptive subject ROI.
#[allow(dead_code)]
pub fn face_clarity_proxy(img: &DynamicImage) -> f64 {
    face_clarity_with_roi(img, &find_subject_roi(img))
}

fn face_clarity_with_roi(img: &DynamicImage, roi: &SubjectRoi) -> f64 {
    laplacian_variance(&crop_luma(img, roi.x, roi.y, roi.w, roi.h))
}

/// Exposure quality: 0.0 (bad) to 1.0 (ideal).
pub fn exposure_quality(img: &DynamicImage) -> f64 {
    exposure_quality_gray(&img.to_luma8())
}

/// Portrait exposure favors readable skin/face tones while retaining some
/// whole-frame context. This avoids rejecting a well-lit subject merely
/// because a sunset, doorway, or studio background is very bright or dark.
#[allow(dead_code)]
pub fn portrait_exposure_quality(img: &DynamicImage) -> f64 {
    portrait_exposure_with_roi(img, &find_subject_roi(img))
}

fn portrait_exposure_with_roi(img: &DynamicImage, roi: &SubjectRoi) -> f64 {
    let subject = exposure_quality_gray(&crop_luma(img, roi.x, roi.y, roi.w, roi.h));
    let global = exposure_quality(img);
    (0.7 * subject + 0.3 * global).clamp(0.0, 1.0)
}

fn exposure_quality_gray(gray: &GrayImage) -> f64 {
    let total = gray.pixels().count() as f64;
    if total == 0.0 {
        return 0.5;
    }

    let mut histogram = [0u32; 256];
    for px in gray.pixels() {
        histogram[px.0[0] as usize] += 1;
    }

    let clipped_dark: f64 = histogram[..6].iter().map(|&c| c as f64).sum::<f64>() / total;
    let clipped_bright: f64 = histogram[250..].iter().map(|&c| c as f64).sum::<f64>() / total;

    let mean: f64 = histogram
        .iter()
        .enumerate()
        .map(|(i, &c)| i as f64 * c as f64)
        .sum::<f64>()
        / total;
    let mean_penalty = 1.0 - ((mean - 128.0) / 128.0).abs().powi(2) * 0.5;

    let variance: f64 = histogram
        .iter()
        .enumerate()
        .map(|(i, &c)| {
            let diff = i as f64 - mean;
            diff * diff * c as f64
        })
        .sum::<f64>()
        / total;
    let std_dev = variance.sqrt();
    let spread_bonus = (std_dev / 60.0).clamp(0.3, 1.0);
    let clip_penalty = 1.0 - (clipped_dark + clipped_bright).clamp(0.0, 0.5) * 1.5;

    (mean_penalty * spread_bonus * clip_penalty).clamp(0.0, 1.0)
}

/// Dynamic range: ratio of usable histogram tones. Images with wider tonal range
/// (more "life" and detail in shadows/highlights) score higher.
pub fn dynamic_range_quality(img: &DynamicImage) -> f64 {
    let gray = img.to_luma8();
    let total = gray.pixels().count() as f64;
    if total == 0.0 {
        return 0.5;
    }

    let mut histogram = [0u32; 256];
    for px in gray.pixels() {
        histogram[px.0[0] as usize] += 1;
    }

    let occupied = histogram.iter().filter(|&&c| c > 0).count() as f64;
    let range_ratio = occupied / 256.0;

    // Percentile range (5th to 95th) for robust dynamic range
    let mut cumulative = 0.0;
    let mut p5 = 0usize;
    let mut p95 = 255usize;
    for (i, &c) in histogram.iter().enumerate() {
        cumulative += c as f64;
        if cumulative / total <= 0.05 {
            p5 = i;
        }
        if cumulative / total <= 0.95 {
            p95 = i;
        }
    }
    let percentile_range = (p95.saturating_sub(p5)) as f64 / 255.0;

    (0.4 * range_ratio + 0.6 * percentile_range).clamp(0.0, 1.0)
}

/// Color vibrancy: how colorful vs monotone the image is.
/// Expressive moments tend to have richer color variance.
pub fn color_vibrancy(img: &DynamicImage) -> f64 {
    let rgb = img.to_rgb8();
    let total = rgb.pixels().count() as f64;
    if total == 0.0 {
        return 0.5;
    }

    let mut sum_sat = 0.0f64;
    for px in rgb.pixels() {
        let r = px.0[0] as f64 / 255.0;
        let g = px.0[1] as f64 / 255.0;
        let b = px.0[2] as f64 / 255.0;
        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        let sat = if max > 1e-9 { (max - min) / max } else { 0.0 };
        sum_sat += sat;
    }
    let mean_sat = sum_sat / total;
    // Typical photo saturation: 0.1 to 0.5 range
    (mean_sat * 2.0).clamp(0.0, 1.0)
}

/// Expression proxy: images capturing "moments" tend to have more contrast
/// and gradient variation in the adaptive subject ROI.
#[allow(dead_code)]
pub fn expression_energy(img: &DynamicImage) -> f64 {
    expression_energy_with_roi(img, &find_subject_roi(img))
}

fn expression_energy_with_roi(img: &DynamicImage, roi: &SubjectRoi) -> f64 {
    let face_zone = crop_luma(img, roi.x, roi.y, roi.w, roi.h);
    let (w, h) = face_zone.dimensions();
    if w < 4 || h < 4 {
        return 0.0;
    }

    // Gradient magnitude map (Sobel-like approximation)
    let mut gradient_sum = 0.0f64;
    let mut n = 0u64;
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let gx = face_zone.get_pixel(x + 1, y).0[0] as f64
                - face_zone.get_pixel(x - 1, y).0[0] as f64;
            let gy = face_zone.get_pixel(x, y + 1).0[0] as f64
                - face_zone.get_pixel(x, y - 1).0[0] as f64;
            gradient_sum += (gx * gx + gy * gy).sqrt();
            n += 1;
        }
    }
    if n == 0 {
        return 0.0;
    }

    // Mean gradient magnitude in the face zone — richer expressions have
    // more texture variation (eyes open, smile, movement).
    gradient_sum / n as f64
}

/// Adaptive subject box used for portrait face metrics. Dependency-light:
/// searches candidate windows and ranks them by skin-tone likelihood, local
/// detail, and a soft portrait prior (favor upper/center placements).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SubjectRoi {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

/// Evidence behind the chosen ROI. Logged in benchmark mode so ROI placement
/// can be audited offline without access to the original photograph.
#[derive(Debug, Clone, Copy, Default)]
pub struct RoiEvidence {
    pub candidate_score: f64,
    pub skin_fraction: f64,
    pub detail: f64,
    pub prior: f64,
    pub used_fallback: bool,
    pub candidates_evaluated: usize,
}

/// Locate the most likely subject/face region without an ML face detector.
pub fn find_subject_roi(img: &DynamicImage) -> SubjectRoi {
    find_subject_roi_detailed(img).0
}

/// ROI search that also reports why the winning window was selected.
pub fn find_subject_roi_detailed(img: &DynamicImage) -> (SubjectRoi, RoiEvidence) {
    let (w, h) = img.dimensions();
    let fallback = default_upper_center_roi(w, h);
    if w < 32 || h < 32 {
        return (
            fallback,
            RoiEvidence {
                used_fallback: true,
                ..RoiEvidence::default()
            },
        );
    }

    let rgb = img.to_rgb8();
    let box_w = (w * 2 / 5).max(16).min(w);
    let box_h = (h * 2 / 5).max(16).min(h);
    let max_x = w.saturating_sub(box_w);
    let max_y = h.saturating_sub(box_h);

    // 5x4 search grid covers off-center portraits without becoming expensive.
    let x_steps = if max_x == 0 { 1 } else { 5u32 };
    let y_steps = if max_y == 0 { 1 } else { 4u32 };

    let mut best = fallback;
    let mut best_score = f64::NEG_INFINITY;
    let mut best_parts = RoiCandidateParts::default();
    let mut evaluated = 0usize;

    for yi in 0..y_steps {
        for xi in 0..x_steps {
            let x = if x_steps == 1 {
                0
            } else {
                xi * max_x / (x_steps - 1)
            };
            let y = if y_steps == 1 {
                0
            } else {
                yi * max_y / (y_steps - 1)
            };
            let parts = score_roi_candidate(&rgb, x, y, box_w, box_h, w, h);
            evaluated += 1;
            if parts.total > best_score {
                best_score = parts.total;
                best_parts = parts;
                best = SubjectRoi {
                    x,
                    y,
                    w: box_w,
                    h: box_h,
                };
            }
        }
    }

    // If the adaptive search finds almost nothing face-like, keep the legacy
    // upper-center prior so behavior stays stable on non-portrait content.
    let used_fallback = best_score < 0.08;
    let roi = if used_fallback { fallback } else { best };

    (
        roi,
        RoiEvidence {
            candidate_score: if best_score.is_finite() {
                best_score
            } else {
                0.0
            },
            skin_fraction: best_parts.skin_fraction,
            detail: best_parts.detail,
            prior: best_parts.prior,
            used_fallback,
            candidates_evaluated: evaluated,
        },
    )
}

fn default_upper_center_roi(w: u32, h: u32) -> SubjectRoi {
    let cw = (w * 2 / 5).max(8).min(w);
    let ch = (h * 2 / 5).max(8).min(h);
    SubjectRoi {
        x: (w.saturating_sub(cw)) / 2,
        y: h.saturating_sub(ch) * 3 / 10,
        w: cw,
        h: ch,
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct RoiCandidateParts {
    total: f64,
    skin_fraction: f64,
    detail: f64,
    prior: f64,
}

fn score_roi_candidate(
    rgb: &image::RgbImage,
    x: u32,
    y: u32,
    box_w: u32,
    box_h: u32,
    frame_w: u32,
    frame_h: u32,
) -> RoiCandidateParts {
    let mut skin = 0u64;
    let mut total = 0u64;
    let mut luma_sum = 0.0f64;
    let mut luma_sq = 0.0f64;

    // Subsample for speed; every 2nd pixel is enough for ranking windows.
    let step = 2u32;
    for py in (y..y + box_h).step_by(step as usize) {
        for px in (x..x + box_w).step_by(step as usize) {
            let p = rgb.get_pixel(px, py).0;
            let r = p[0] as f64;
            let g = p[1] as f64;
            let b = p[2] as f64;
            if looks_like_skin(r, g, b) {
                skin += 1;
            }
            let luma = 0.299 * r + 0.587 * g + 0.114 * b;
            luma_sum += luma;
            luma_sq += luma * luma;
            total += 1;
        }
    }
    if total == 0 {
        return RoiCandidateParts::default();
    }

    let skin_fraction = skin as f64 / total as f64;
    let mean = luma_sum / total as f64;
    let var = (luma_sq / total as f64 - mean * mean).max(0.0);
    // Local contrast stands in for facial detail without a Laplacian pass.
    let detail = (var.sqrt() / 64.0).clamp(0.0, 1.0);

    let cx = (x + box_w / 2) as f64 / frame_w as f64;
    let cy = (y + box_h / 2) as f64 / frame_h as f64;
    // Soft portrait prior: prefer upper-middle placements.
    let prior = (1.0 - (cx - 0.5).abs() * 1.4).clamp(0.0, 1.0)
        * (1.0 - (cy - 0.38).abs() * 1.6).clamp(0.0, 1.0);

    RoiCandidateParts {
        total: 0.50 * skin_fraction + 0.30 * detail + 0.20 * prior,
        skin_fraction,
        detail,
        prior,
    }
}

/// Lightweight skin-tone heuristic spanning common lighting conditions.
fn looks_like_skin(r: f64, g: f64, b: f64) -> bool {
    // Classic RGB rule covers many daylight and indoor skin tones.
    let rgb_hit = r > 95.0
        && g > 40.0
        && b > 20.0
        && r > g
        && r > b
        && (r - g).abs() > 15.0
        && r.max(g).max(b) - r.min(g).min(b) > 15.0;

    // HSV fallback helps backlit / cooler skin where RGB thresholds fail.
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let delta = max - min;
    if max < 1e-6 || delta < 1e-6 {
        return rgb_hit;
    }
    let mut h = if (max - r).abs() < 1e-6 {
        60.0 * (((g - b) / delta) % 6.0)
    } else if (max - g).abs() < 1e-6 {
        60.0 * (((b - r) / delta) + 2.0)
    } else {
        60.0 * (((r - g) / delta) + 4.0)
    };
    if h < 0.0 {
        h += 360.0;
    }
    let s = delta / max;
    let v = max / 255.0;
    let hsv_hit =
        ((h <= 50.0) || (h >= 340.0)) && (0.08..=0.75).contains(&s) && (0.18..=0.98).contains(&v);

    rgb_hit || hsv_hit
}

#[derive(Clone, Copy)]
enum Region {
    CenterHalf,
}

fn to_region_gray(img: &DynamicImage, region: Region) -> GrayImage {
    let (w, h) = img.dimensions();
    match region {
        Region::CenterHalf => {
            let cw = (w / 2).max(8);
            let ch = (h / 2).max(8);
            let x0 = (w.saturating_sub(cw)) / 2;
            let y0 = (h.saturating_sub(ch)) / 2;
            crop_luma(img, x0, y0, cw, ch)
        }
    }
}

fn crop_luma(img: &DynamicImage, x: u32, y: u32, w: u32, h: u32) -> GrayImage {
    let cropped = image::imageops::crop_imm(img, x, y, w, h).to_image();
    DynamicImage::ImageRgba8(cropped).to_luma8()
}

fn laplacian_variance(gray: &GrayImage) -> f64 {
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

#[derive(Debug, Clone)]
pub struct ScoredMember {
    pub index: usize,
    pub score: f64,
    pub sharpness: f64,
    pub focus: f64,
    pub face: f64,
    pub exposure: f64,
    pub expression: f64,
    pub vibrancy: f64,
    pub dynamic_range: f64,
    pub blur_confidence: f64,
}

impl ScoredMember {
    pub fn zero(index: usize) -> Self {
        Self {
            index,
            score: 0.0,
            sharpness: 0.0,
            focus: 0.0,
            face: 0.0,
            exposure: 0.0,
            expression: 0.0,
            vibrancy: 0.0,
            dynamic_range: 0.0,
            blur_confidence: 0.0,
        }
    }

    pub fn with_index(mut self, idx: usize) -> Self {
        self.index = idx;
        self
    }
}

/// Score an image with emphasis on expression/moment over raw sharpness.
/// Portrait: expression (face energy + face clarity) is the heaviest factor.
/// Landscape: dynamic range and exposure dominate, sharpness is secondary.
pub fn score_member(img: &DynamicImage, scene: SceneMode, perf: PerfProfile) -> ScoredMember {
    let subject_roi = find_subject_roi(img);
    let sharpness = sharpness_laplacian(img);
    let focus = subject_focus_with_roi(img, &subject_roi);
    let exposure = match scene {
        SceneMode::Portrait => portrait_exposure_with_roi(img, &subject_roi),
        SceneMode::Landscape => exposure_quality(img),
    };
    let dynamic_range = dynamic_range_quality(img);
    let vibrancy = color_vibrancy(img);
    let expression = expression_energy_with_roi(img, &subject_roi);
    let blur_conf = match scene {
        SceneMode::Portrait => portrait_blur_with_roi(img, &subject_roi),
        SceneMode::Landscape => blur_confidence(img),
    };
    let face = if perf.face_scoring() {
        face_clarity_with_roi(img, &subject_roi)
    } else {
        0.0
    };

    // Base score: expression/moment first, technical quality secondary.
    let base = match scene {
        SceneMode::Portrait if perf.face_scoring() => {
            0.25 * expression
                + 0.15 * face
                + 0.15 * exposure
                + 0.10 * dynamic_range
                + 0.10 * sharpness
                + 0.10 * (focus * sharpness)
                + 0.15 * vibrancy
        }
        SceneMode::Portrait => {
            0.30 * expression
                + 0.20 * exposure
                + 0.10 * dynamic_range
                + 0.15 * sharpness
                + 0.10 * (focus * sharpness)
                + 0.15 * vibrancy
        }
        SceneMode::Landscape => {
            0.25 * dynamic_range
                + 0.20 * exposure
                + 0.20 * sharpness
                + 0.15 * (focus * sharpness)
                + 0.20 * vibrancy
        }
    };

    // Multiplicative gates: an image where the subject is completely blurred
    // or the whole image is out-of-focus cannot win regardless of other metrics.
    // blur_confidence goes to ~0 for severely blurred; focus goes to ~0.05 for
    // out-of-focus subjects on sharp backgrounds. Multiplying makes these near-zero.
    let focus_gate = focus.clamp(0.05, 1.0);
    let blur_gate = blur_conf.clamp(0.05, 1.0);
    let score = base * blur_gate * focus_gate;

    ScoredMember {
        index: 0,
        score,
        sharpness,
        focus,
        face,
        exposure,
        expression,
        vibrancy,
        dynamic_range,
        blur_confidence: blur_conf,
    }
}

/// Every intermediate value behind a score, captured so thresholds and weights
/// can be re-derived offline from a benchmark log alone — without shipping the
/// customer's photographs off their machine.
#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct QualityDiagnostics {
    pub width: u32,
    pub height: u32,
    pub megapixels: f64,
    pub aspect_ratio: f64,

    pub roi_x: u32,
    pub roi_y: u32,
    pub roi_w: u32,
    pub roi_h: u32,
    pub roi_center_rel_x: f64,
    pub roi_center_rel_y: f64,
    pub roi_area_fraction: f64,
    pub roi_candidate_score: f64,
    pub roi_skin_fraction: f64,
    pub roi_detail: f64,
    pub roi_prior: f64,
    pub roi_used_fallback: bool,
    pub roi_candidates_evaluated: usize,

    pub region_sharpness: Vec<f64>,
    pub sharpness_top3: f64,
    pub sharpness_max_region: f64,
    pub sharpness_min_region: f64,
    pub border_sharpness: f64,
    pub center_laplacian: f64,
    pub roi_laplacian: f64,
    pub subject_harmonic: f64,
    pub focus_absolute_term: f64,
    pub focus_relative_term: f64,
    pub focus_subject_border_ratio: f64,

    pub blur_confidence_global: f64,
    pub blur_confidence_portrait: f64,

    pub exposure_global: f64,
    pub exposure_portrait: f64,
    pub frame_luma: LumaStats,
    pub roi_luma: LumaStats,

    pub saturation_mean: f64,
    pub dynamic_range_occupied_bins: usize,
    pub dynamic_range_p5: usize,
    pub dynamic_range_p95: usize,
}

/// Compact luminance summary. The 16-bin histogram keeps the log small while
/// still allowing exposure and contrast thresholds to be re-fit later.
#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LumaStats {
    pub mean: f64,
    pub std_dev: f64,
    pub clipped_dark: f64,
    pub clipped_bright: f64,
    pub histogram16: Vec<f64>,
}

fn luma_stats(gray: &GrayImage) -> LumaStats {
    let total = gray.pixels().count() as f64;
    if total == 0.0 {
        return LumaStats {
            histogram16: vec![0.0; 16],
            ..LumaStats::default()
        };
    }

    let mut histogram = [0u32; 256];
    for px in gray.pixels() {
        histogram[px.0[0] as usize] += 1;
    }

    let mean: f64 = histogram
        .iter()
        .enumerate()
        .map(|(i, &c)| i as f64 * c as f64)
        .sum::<f64>()
        / total;
    let variance: f64 = histogram
        .iter()
        .enumerate()
        .map(|(i, &c)| {
            let diff = i as f64 - mean;
            diff * diff * c as f64
        })
        .sum::<f64>()
        / total;

    let mut histogram16 = vec![0.0f64; 16];
    for (i, &c) in histogram.iter().enumerate() {
        histogram16[i / 16] += c as f64 / total;
    }

    LumaStats {
        mean,
        std_dev: variance.sqrt(),
        clipped_dark: histogram[..6].iter().map(|&c| c as f64).sum::<f64>() / total,
        clipped_bright: histogram[250..].iter().map(|&c| c as f64).sum::<f64>() / total,
        histogram16,
    }
}

/// The nine rule-of-thirds Laplacian variances behind `sharpness_laplacian`.
fn region_sharpness_grid(img: &DynamicImage) -> Vec<f64> {
    let (w, h) = img.dimensions();
    let cell_w = (w / 3).max(16);
    let cell_h = (h / 3).max(16);
    let step_x = (w.saturating_sub(cell_w)) / 2;
    let step_y = (h.saturating_sub(cell_h)) / 2;

    let mut values = Vec::with_capacity(9);
    for row in 0..3 {
        for col in 0..3 {
            let region = crop_luma(img, col * step_x, row * step_y, cell_w, cell_h);
            values.push(laplacian_variance(&region));
        }
    }
    values
}

fn dynamic_range_parts(gray: &GrayImage) -> (usize, usize, usize) {
    let total = gray.pixels().count() as f64;
    if total == 0.0 {
        return (0, 0, 0);
    }
    let mut histogram = [0u32; 256];
    for px in gray.pixels() {
        histogram[px.0[0] as usize] += 1;
    }
    let occupied = histogram.iter().filter(|&&c| c > 0).count();
    let mut cumulative = 0.0;
    let mut p5 = 0usize;
    let mut p95 = 255usize;
    for (i, &c) in histogram.iter().enumerate() {
        cumulative += c as f64;
        if cumulative / total <= 0.05 {
            p5 = i;
        }
        if cumulative / total <= 0.95 {
            p95 = i;
        }
    }
    (occupied, p5, p95)
}

/// Score an image and capture the full evidence trail. Used by benchmark
/// logging; the hot path stays on `score_member`.
pub fn score_member_detailed(
    img: &DynamicImage,
    scene: SceneMode,
    perf: PerfProfile,
) -> (ScoredMember, QualityDiagnostics) {
    let member = score_member(img, scene, perf);
    let (roi, evidence) = find_subject_roi_detailed(img);
    let (w, h) = img.dimensions();

    let region_sharpness = region_sharpness_grid(img);
    let center_laplacian = laplacian_variance(&to_region_gray(img, Region::CenterHalf));
    let roi_laplacian = laplacian_variance(&crop_luma(img, roi.x, roi.y, roi.w, roi.h));
    let subject_harmonic =
        2.0 * roi_laplacian * center_laplacian / (roi_laplacian + center_laplacian + 1e-6);
    let border = border_sharpness(img);
    let ratio = subject_harmonic / (border + 1e-6);

    let gray = img.to_luma8();
    let (occupied, p5, p95) = dynamic_range_parts(&gray);

    let diagnostics = QualityDiagnostics {
        width: w,
        height: h,
        megapixels: (w as f64 * h as f64) / 1_000_000.0,
        aspect_ratio: if h == 0 { 0.0 } else { w as f64 / h as f64 },

        roi_x: roi.x,
        roi_y: roi.y,
        roi_w: roi.w,
        roi_h: roi.h,
        roi_center_rel_x: if w == 0 {
            0.0
        } else {
            (roi.x + roi.w / 2) as f64 / w as f64
        },
        roi_center_rel_y: if h == 0 {
            0.0
        } else {
            (roi.y + roi.h / 2) as f64 / h as f64
        },
        roi_area_fraction: if w == 0 || h == 0 {
            0.0
        } else {
            (roi.w as f64 * roi.h as f64) / (w as f64 * h as f64)
        },
        roi_candidate_score: evidence.candidate_score,
        roi_skin_fraction: evidence.skin_fraction,
        roi_detail: evidence.detail,
        roi_prior: evidence.prior,
        roi_used_fallback: evidence.used_fallback,
        roi_candidates_evaluated: evidence.candidates_evaluated,

        sharpness_top3: member.sharpness,
        sharpness_max_region: region_sharpness.iter().copied().fold(0.0, f64::max),
        sharpness_min_region: region_sharpness
            .iter()
            .copied()
            .fold(f64::INFINITY, f64::min),
        region_sharpness,
        border_sharpness: border,
        center_laplacian,
        roi_laplacian,
        subject_harmonic,
        focus_absolute_term: ((subject_harmonic - 12.0) / 108.0).clamp(0.0, 1.0),
        focus_relative_term: ((ratio - 0.35) / 1.15).clamp(0.0, 1.0),
        focus_subject_border_ratio: ratio,

        blur_confidence_global: blur_confidence(img),
        blur_confidence_portrait: portrait_blur_with_roi(img, &roi),

        exposure_global: exposure_quality(img),
        exposure_portrait: portrait_exposure_with_roi(img, &roi),
        frame_luma: luma_stats(&gray),
        roi_luma: luma_stats(&crop_luma(img, roi.x, roi.y, roi.w, roi.h)),

        saturation_mean: color_vibrancy(img) / 2.0,
        dynamic_range_occupied_bins: occupied,
        dynamic_range_p5: p5,
        dynamic_range_p95: p95,
    };

    (member, diagnostics)
}

/// Pick exactly one winner. Prefer RAW/DNG only when sharpness is nearly tied.
pub fn pick_winner(group: &[usize], records: &[ImageRecord], scores: &[ScoredMember]) -> usize {
    assert!(!group.is_empty());
    let members: Vec<&ScoredMember> = scores.iter().filter(|s| group.contains(&s.index)).collect();
    assert!(!members.is_empty());

    let max_score = members
        .iter()
        .map(|m| m.score)
        .fold(f64::NEG_INFINITY, f64::max);

    // Prefer clear sharpness winners over RAW bias.
    let best = members
        .iter()
        .copied()
        .max_by(|a, b| {
            a.score
                .total_cmp(&b.score)
                .then_with(|| a.sharpness.total_cmp(&b.sharpness))
                .then_with(|| a.focus.total_cmp(&b.focus))
        })
        .unwrap_or(members[0]);

    let max_sharp = members
        .iter()
        .map(|m| m.sharpness)
        .fold(0.0f64, f64::max)
        .max(1e-9);
    let near_sharp: Vec<&ScoredMember> = members
        .iter()
        .copied()
        .filter(|m| (max_score - m.score) <= (max_score.abs() * 0.02 + 1e-9))
        .filter(|m| (max_sharp - m.sharpness) / max_sharp <= 0.02)
        .collect();

    let prefer_raw: Vec<&ScoredMember> = near_sharp
        .iter()
        .copied()
        .filter(|m| records[m.index].is_raw_or_dng)
        .collect();

    let candidates = if !prefer_raw.is_empty() {
        prefer_raw
    } else {
        vec![best]
    };

    candidates
        .into_iter()
        .max_by(|a, b| {
            let ra = &records[a.index];
            let rb = &records[b.index];
            a.score
                .total_cmp(&b.score)
                .then_with(|| a.sharpness.total_cmp(&b.sharpness))
                .then_with(|| (ra.preview_w * ra.preview_h).cmp(&(rb.preview_w * rb.preview_h)))
                .then_with(|| ra.size.cmp(&rb.size))
                .then_with(|| ra.path.cmp(&rb.path))
        })
        .map(|m| m.index)
        .unwrap_or(group[0])
}

pub fn normalize_group_scores(scores: &mut [ScoredMember], group: &[usize], scene: SceneMode) {
    let idxs: Vec<usize> = scores
        .iter()
        .enumerate()
        .filter(|(_, s)| group.contains(&s.index))
        .map(|(i, _)| i)
        .collect();
    if idxs.is_empty() {
        return;
    }

    fn group_max(scores: &[ScoredMember], idxs: &[usize], f: fn(&ScoredMember) -> f64) -> f64 {
        idxs.iter()
            .map(|&i| f(&scores[i]))
            .fold(0.0f64, f64::max)
            .max(1e-9)
    }

    let max_s = group_max(scores, &idxs, |s| s.sharpness);
    let max_focus = group_max(scores, &idxs, |s| s.focus);
    let max_f = group_max(scores, &idxs, |s| s.face);
    let max_e = group_max(scores, &idxs, |s| s.exposure);
    let max_expr = group_max(scores, &idxs, |s| s.expression);
    let max_vib = group_max(scores, &idxs, |s| s.vibrancy);
    let max_dr = group_max(scores, &idxs, |s| s.dynamic_range);

    let has_face = !idxs.iter().all(|&j| scores[j].face == 0.0);

    for &i in &idxs {
        let s = scores[i].sharpness / max_s;
        let focus = scores[i].focus / max_focus;
        let f = scores[i].face / max_f;
        let e = scores[i].exposure / max_e;
        let expr = scores[i].expression / max_expr;
        let vib = scores[i].vibrancy / max_vib;
        let dr = scores[i].dynamic_range / max_dr;

        let base = match scene {
            SceneMode::Portrait if has_face => {
                0.25 * expr
                    + 0.15 * f
                    + 0.15 * e
                    + 0.10 * dr
                    + 0.10 * s
                    + 0.10 * (focus * s)
                    + 0.15 * vib
            }
            SceneMode::Portrait => {
                0.30 * expr + 0.20 * e + 0.10 * dr + 0.15 * s + 0.10 * (focus * s) + 0.15 * vib
            }
            SceneMode::Landscape => {
                0.25 * dr + 0.20 * e + 0.20 * s + 0.15 * (focus * s) + 0.20 * vib
            }
        };

        // Apply the same absolute blur gates as score_member so severely
        // blurred images can't win even after in-group normalization.
        let focus_gate = scores[i].focus.clamp(0.05, 1.0);
        let blur_gate = scores[i].blur_confidence.clamp(0.05, 1.0);
        scores[i].score = base * blur_gate * focus_gate;
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

    fn well_exposed() -> DynamicImage {
        DynamicImage::ImageRgb8(ImageBuffer::from_fn(64, 64, |x, y| {
            let v = ((x * 4 + y * 3) % 200 + 28) as u8;
            Rgb([v, v, v])
        }))
    }

    fn overexposed() -> DynamicImage {
        DynamicImage::ImageRgb8(ImageBuffer::from_fn(64, 64, |_x, _y| {
            Rgb([250u8, 252, 255])
        }))
    }

    fn soft_center_sharp_border() -> DynamicImage {
        DynamicImage::ImageRgb8(ImageBuffer::from_fn(64, 64, |x, y| {
            let edge = x < 6 || y < 6 || x > 57 || y > 57;
            let v = if edge {
                if (x + y) % 2 == 0 {
                    0
                } else {
                    255
                }
            } else {
                128
            };
            Rgb([v, v, v])
        }))
    }

    fn sharp_center_soft_border() -> DynamicImage {
        DynamicImage::ImageRgb8(ImageBuffer::from_fn(64, 64, |x, y| {
            let center = x > 16 && x < 48 && y > 16 && y < 48;
            let v = if center {
                if (x + y) % 2 == 0 {
                    0
                } else {
                    255
                }
            } else {
                128
            };
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
    fn subject_focus_prefers_sharp_center() {
        let good = subject_focus_proxy(&sharp_center_soft_border());
        let bad = subject_focus_proxy(&soft_center_sharp_border());
        assert!(
            good > bad,
            "focused subject {good} should beat soft subject {bad}"
        );
    }

    #[test]
    fn exposure_prefers_well_exposed() {
        let good = exposure_quality(&well_exposed());
        let bad = exposure_quality(&overexposed());
        assert!(
            good > bad,
            "well-exposed {good} should beat overexposed {bad}"
        );
    }

    #[test]
    fn score_member_returns_all_metrics() {
        let img = sharpish();
        let scored = score_member(&img, SceneMode::Portrait, PerfProfile::Medium);
        assert!(scored.sharpness > 0.0);
        assert!(scored.exposure > 0.0);
        assert!(scored.focus > 0.0);
    }

    fn scored(
        index: usize,
        score: f64,
        sharpness: f64,
        focus: f64,
        face: f64,
        exposure: f64,
    ) -> ScoredMember {
        ScoredMember {
            score,
            sharpness,
            focus,
            face,
            exposure,
            blur_confidence: 1.0,
            ..ScoredMember::zero(index)
        }
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
            scored(0, 1.0, 1.0, 1.0, 1.0, 1.0),
            scored(1, 1.0, 1.0, 1.0, 1.0, 1.0),
        ];
        assert_eq!(pick_winner(&[0, 1], &records, &scores), 1);
    }

    #[test]
    fn pick_higher_score_over_softer_raw() {
        let records = vec![
            ImageRecord {
                path: PathBuf::from("soft.arw"),
                blake3: "x".into(),
                phash: Some(0),
                dhash: None,
                preview_w: 100,
                preview_h: 100,
                size: 5_000_000,
                is_raw_or_dng: true,
            },
            ImageRecord {
                path: PathBuf::from("expressive.jpg"),
                blake3: "x".into(),
                phash: Some(0),
                dhash: None,
                preview_w: 100,
                preview_h: 100,
                size: 2_000_000,
                is_raw_or_dng: false,
            },
        ];
        let scores = vec![
            scored(0, 0.55, 0.50, 0.8, 0.4, 0.7),
            scored(1, 0.95, 1.0, 1.2, 0.9, 0.9),
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
            scored(0, 5.0, 5.0, 1.0, 0.0, 0.8),
            scored(1, 5.0, 5.0, 1.0, 0.0, 0.8),
        ];
        assert_eq!(pick_winner(&[0, 1], &records, &scores), 1);
    }

    #[test]
    fn expression_energy_nonzero_for_gradient_image() {
        let img = sharp_center_soft_border();
        let energy = expression_energy(&img);
        assert!(
            energy > 0.0,
            "expression energy {energy} should be positive for gradient image"
        );
    }

    #[test]
    fn dynamic_range_nonzero_for_varied_image() {
        let dr = dynamic_range_quality(&well_exposed());
        assert!(
            dr > 0.3,
            "dynamic range {dr} should be substantial for well-exposed image"
        );
    }

    #[test]
    fn color_vibrancy_low_for_grayscale() {
        let gray = DynamicImage::ImageRgb8(ImageBuffer::from_pixel(64, 64, Rgb([128u8, 128, 128])));
        let v = color_vibrancy(&gray);
        assert!(v < 0.1, "grayscale vibrancy {v} should be near zero");
    }

    fn all_blurred() -> DynamicImage {
        DynamicImage::ImageRgb8(ImageBuffer::from_fn(256, 256, |x, y| {
            let v = ((x as f64 * 0.03).sin() * 30.0 + (y as f64 * 0.02).cos() * 20.0 + 128.0) as u8;
            Rgb([v, v, v])
        }))
    }

    fn detailed() -> DynamicImage {
        DynamicImage::ImageRgb8(ImageBuffer::from_fn(256, 256, |x, y| {
            let v = if ((x / 2) + (y / 2)) % 2 == 0 {
                20
            } else {
                235
            };
            Rgb([v, v, v])
        }))
    }

    #[test]
    fn blur_confidence_low_for_blurred_image() {
        let bc_blur = blur_confidence(&all_blurred());
        let bc_sharp = blur_confidence(&detailed());
        assert!(
            bc_sharp > bc_blur,
            "sharp {bc_sharp} should beat blurred {bc_blur}"
        );
        assert!(
            bc_blur < 0.5,
            "blurred confidence {bc_blur} should be < 0.5"
        );
    }

    #[test]
    fn score_gates_out_severely_blurred_subject() {
        let blurred_score = score_member(&all_blurred(), SceneMode::Portrait, PerfProfile::Medium);
        let sharp_score = score_member(&detailed(), SceneMode::Portrait, PerfProfile::Medium);
        assert!(
            sharp_score.score > blurred_score.score * 1.5,
            "sharp {} should be much better than blurred {}",
            sharp_score.score,
            blurred_score.score
        );
    }

    #[test]
    fn subject_focus_penalizes_defocused_center() {
        // Center is uniform (defocused subject), border has detail (sharp background)
        let img = DynamicImage::ImageRgb8(ImageBuffer::from_fn(200, 200, |x, y| {
            let center = (50..150).contains(&x) && (30..150).contains(&y);
            let v = if center {
                128
            } else if (x + y) % 2 == 0 {
                20
            } else {
                235
            };
            Rgb([v, v, v])
        }));
        let focus = subject_focus_proxy(&img);
        assert!(
            focus < 0.55,
            "defocused-subject focus {focus} should be low"
        );
    }

    #[test]
    fn sample_image_quality_regression() {
        let samples = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../example-images");
        let mut scores = Vec::new();
        for number in 1..=10 {
            let path = samples.join(format!("sample{number}.png"));
            let img = image::open(&path)
                .unwrap_or_else(|error| panic!("failed to load {}: {error}", path.display()));
            let score = score_member(&img, SceneMode::Portrait, PerfProfile::High);
            println!(
                "sample{number}: score={:.4} sharp={:.2} focus={:.3} face={:.2} exposure={:.3} expression={:.2} vibrancy={:.3} range={:.3} blur={:.3}",
                score.score,
                score.sharpness,
                score.focus,
                score.face,
                score.exposure,
                score.expression,
                score.vibrancy,
                score.dynamic_range,
                score.blur_confidence,
            );
            scores.push(score.with_index(number - 1));
        }

        for sample in [1usize, 8, 10] {
            assert!(
                scores[sample - 1].blur_confidence >= 0.9,
                "sample{sample} is a visibly focused keeper"
            );
        }
        assert!(
            scores[2].blur_confidence < 0.5,
            "sample3 has obvious subject motion blur"
        );
        for sample in [7usize, 9] {
            assert!(
                scores[sample - 1].blur_confidence < 0.2,
                "sample{sample} is severely blurred"
            );
        }
        let sample6_global_exposure = exposure_quality(
            &image::open(samples.join("sample6.png")).expect("sample6 should load"),
        );
        assert!(
            scores[5].exposure > sample6_global_exposure,
            "portrait exposure should judge sample6's face, not its bright background"
        );

        let group: Vec<usize> = (0..10).collect();
        normalize_group_scores(&mut scores, &group, SceneMode::Portrait);
        scores.sort_by(|a, b| b.score.total_cmp(&a.score));
        println!(
            "normalized ranking: {}",
            scores
                .iter()
                .map(|score| format!("sample{}={:.3}", score.index + 1, score.score))
                .collect::<Vec<_>>()
                .join(", ")
        );
        for keeper in [1usize, 6, 8, 10] {
            let keeper_score = scores
                .iter()
                .find(|score| score.index + 1 == keeper)
                .unwrap()
                .score;
            for reject in [3usize, 7, 9] {
                let reject_score = scores
                    .iter()
                    .find(|score| score.index + 1 == reject)
                    .unwrap()
                    .score;
                assert!(
                    keeper_score > reject_score,
                    "clear keeper sample{keeper} should outrank blurred sample{reject}"
                );
            }
        }
        let bottom_three: std::collections::HashSet<usize> = scores
            .iter()
            .rev()
            .take(3)
            .map(|score| score.index + 1)
            .collect();
        assert_eq!(
            bottom_three,
            std::collections::HashSet::from([3, 7, 9]),
            "motion-blurred portraits should rank at the bottom"
        );
    }

    #[test]
    fn group_normalization_respects_landscape_mode() {
        let portrait_texture = ScoredMember {
            index: 0,
            expression: 100.0,
            face: 100.0,
            sharpness: 20.0,
            focus: 1.0,
            exposure: 0.3,
            vibrancy: 0.2,
            dynamic_range: 0.2,
            blur_confidence: 1.0,
            score: 0.0,
        };
        let landscape_quality = ScoredMember {
            index: 1,
            expression: 5.0,
            face: 5.0,
            sharpness: 100.0,
            focus: 1.0,
            exposure: 1.0,
            vibrancy: 1.0,
            dynamic_range: 1.0,
            blur_confidence: 1.0,
            score: 0.0,
        };

        let mut landscape_scores = vec![portrait_texture.clone(), landscape_quality.clone()];
        normalize_group_scores(&mut landscape_scores, &[0, 1], SceneMode::Landscape);
        assert!(
            landscape_scores[1].score > landscape_scores[0].score,
            "landscape mode should prioritize range, exposure, sharpness, and color"
        );

        let mut portrait_scores = vec![portrait_texture, landscape_quality];
        normalize_group_scores(&mut portrait_scores, &[0, 1], SceneMode::Portrait);
        assert!(
            portrait_scores[0].score > landscape_scores[0].score,
            "portrait mode should retain expression and face contributions"
        );
    }

    #[test]
    fn adaptive_roi_prefers_off_center_skin_region() {
        // Left third has skin-like tones with facial detail; right side is textured foliage.
        let img = DynamicImage::ImageRgb8(ImageBuffer::from_fn(240, 240, |x, y| {
            if x < 90 && (40..160).contains(&y) {
                let detail = if (x + y) % 3 == 0 { 18 } else { 0 };
                Rgb([
                    190u8.saturating_add(detail),
                    140u8.saturating_add(detail / 2),
                    110u8,
                ])
            } else if (x + y) % 2 == 0 {
                Rgb([20, 90, 30])
            } else {
                Rgb([40, 140, 50])
            }
        }));
        let roi = find_subject_roi(&img);
        assert!(
            roi.x + roi.w / 2 < 120,
            "ROI center should land on the left subject, got {:?}",
            roi
        );
    }

    #[test]
    fn sample6_roi_improves_exposure_over_global() {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../example-images/sample6.png");
        let img = image::open(&path).expect("sample6 should load");
        let roi = find_subject_roi(&img);
        let subject = portrait_exposure_with_roi(&img, &roi);
        let global = exposure_quality(&img);
        assert!(
            subject > global,
            "adaptive ROI exposure {subject} should beat global {global} for backlit sample6 (roi={roi:?})"
        );
    }

    #[test]
    fn detailed_scoring_matches_fast_path_and_fills_diagnostics() {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../example-images/sample1.png");
        let img = image::open(&path).expect("sample1 should load");

        let fast = score_member(&img, SceneMode::Portrait, PerfProfile::High);
        let (detailed, diag) = score_member_detailed(&img, SceneMode::Portrait, PerfProfile::High);

        assert_eq!(fast.score, detailed.score);
        assert_eq!(fast.expression, detailed.expression);

        assert_eq!(diag.region_sharpness.len(), 9);
        assert_eq!(diag.frame_luma.histogram16.len(), 16);
        assert_eq!(diag.roi_luma.histogram16.len(), 16);
        assert!((diag.frame_luma.histogram16.iter().sum::<f64>() - 1.0).abs() < 1e-6);
        assert_eq!(diag.sharpness_top3, fast.sharpness);
        assert!(diag.roi_w > 0 && diag.roi_h > 0);
        assert!(diag.roi_area_fraction > 0.0 && diag.roi_area_fraction <= 1.0);
        assert!(diag.dynamic_range_p95 >= diag.dynamic_range_p5);
        assert!(diag.roi_candidates_evaluated > 1);
    }

    #[test]
    fn diagnostics_reproduce_the_focus_terms_that_drove_the_score() {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../example-images/sample3.png");
        let img = image::open(&path).expect("sample3 should load");
        let (member, diag) = score_member_detailed(&img, SceneMode::Portrait, PerfProfile::High);

        // The log must let us recompute `focus` without the original pixels.
        let rebuilt =
            (0.75 * diag.focus_absolute_term + 0.25 * diag.focus_relative_term).clamp(0.0, 1.0);
        assert!(
            (rebuilt - member.focus).abs() < 1e-9,
            "focus {} should be reconstructible from diagnostics, got {rebuilt}",
            member.focus
        );
        assert_eq!(diag.blur_confidence_portrait, member.blur_confidence);
    }
}
