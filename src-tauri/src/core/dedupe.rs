use image::DynamicImage;
use rustdct::DctPlanner;
use std::collections::HashMap;
use std::path::PathBuf;

/// Compute 64-bit perceptual hash (DCT pHash).
pub fn phash(img: &DynamicImage) -> u64 {
    let gray = img.to_luma8();
    let resized = image::imageops::resize(&gray, 32, 32, image::imageops::FilterType::Triangle);
    let mut samples: Vec<f64> = resized.pixels().map(|p| p.0[0] as f64).collect();

    let mut planner = DctPlanner::new();
    let dct = planner.plan_dct2(32);

    // Row-wise DCT
    for row in 0..32 {
        let start = row * 32;
        dct.process_dct2(&mut samples[start..start + 32]);
    }
    // Column-wise DCT (transpose dance)
    let mut cols = vec![0.0f64; 32 * 32];
    for r in 0..32 {
        for c in 0..32 {
            cols[c * 32 + r] = samples[r * 32 + c];
        }
    }
    for col in 0..32 {
        let start = col * 32;
        dct.process_dct2(&mut cols[start..start + 32]);
    }
    // Back to row-major low-freq 8x8 (skip DC at 0,0 for average)
    let mut vals = Vec::with_capacity(64);
    for r in 0..8 {
        for c in 0..8 {
            vals.push(cols[c * 32 + r]);
        }
    }
    let avg: f64 = vals[1..].iter().sum::<f64>() / 63.0;
    let mut hash = 0u64;
    for (i, v) in vals.iter().enumerate() {
        if *v > avg {
            hash |= 1u64 << i;
        }
    }
    hash
}

pub fn hamming(a: u64, b: u64) -> u32 {
    (a ^ b).count_ones()
}

/// Difference hash for High-mode confirmation.
pub fn dhash(img: &DynamicImage) -> u64 {
    let gray = img.to_luma8();
    let resized = image::imageops::resize(&gray, 9, 8, image::imageops::FilterType::Triangle);
    let mut hash = 0u64;
    let mut bit = 0;
    for y in 0..8 {
        for x in 0..8 {
            let left = resized.get_pixel(x, y).0[0];
            let right = resized.get_pixel(x + 1, y).0[0];
            if left > right {
                hash |= 1u64 << bit;
            }
            bit += 1;
        }
    }
    hash
}

#[derive(Debug, Clone)]
pub struct ImageRecord {
    pub path: PathBuf,
    pub blake3: String,
    pub phash: Option<u64>,
    pub dhash: Option<u64>,
    pub preview_w: u32,
    pub preview_h: u32,
    pub size: u64,
    pub is_raw_or_dng: bool,
}

/// Union-Find for clustering.
struct UnionFind {
    parent: Vec<usize>,
    rank: Vec<u8>,
}

impl UnionFind {
    fn new(n: usize) -> Self {
        Self {
            parent: (0..n).collect(),
            rank: vec![0; n],
        }
    }

    fn find(&mut self, x: usize) -> usize {
        if self.parent[x] != x {
            self.parent[x] = self.find(self.parent[x]);
        }
        self.parent[x]
    }

    fn union(&mut self, a: usize, b: usize) {
        let mut ra = self.find(a);
        let mut rb = self.find(b);
        if ra == rb {
            return;
        }
        if self.rank[ra] < self.rank[rb] {
            std::mem::swap(&mut ra, &mut rb);
        }
        self.parent[rb] = ra;
        if self.rank[ra] == self.rank[rb] {
            self.rank[ra] += 1;
        }
    }
}

/// Cluster by exact blake3 then perceptual hash.
pub fn cluster_duplicates(
    records: &[ImageRecord],
    hamming_threshold: u32,
    confirm_dhash: bool,
) -> Vec<Vec<usize>> {
    let n = records.len();
    if n == 0 {
        return vec![];
    }
    let mut uf = UnionFind::new(n);

    // Exact duplicates
    let mut by_hash: HashMap<&str, Vec<usize>> = HashMap::new();
    for (i, r) in records.iter().enumerate() {
        by_hash.entry(r.blake3.as_str()).or_default().push(i);
    }
    for idxs in by_hash.values() {
        for w in idxs.windows(2) {
            uf.union(w[0], w[1]);
        }
    }

    // Perceptual near-duplicates
    let with_phash: Vec<(usize, u64)> = records
        .iter()
        .enumerate()
        .filter_map(|(i, r)| r.phash.map(|h| (i, h)))
        .collect();

    for i in 0..with_phash.len() {
        for j in (i + 1)..with_phash.len() {
            let (ia, ha) = with_phash[i];
            let (ib, hb) = with_phash[j];
            let dist = hamming(ha, hb);
            if dist <= hamming_threshold {
                if confirm_dhash {
                    if let (Some(da), Some(db)) = (records[ia].dhash, records[ib].dhash) {
                        if hamming(da, db) > hamming_threshold + 2 {
                            continue;
                        }
                    }
                }
                uf.union(ia, ib);
            }
        }
    }

    let mut groups: HashMap<usize, Vec<usize>> = HashMap::new();
    for i in 0..n {
        groups.entry(uf.find(i)).or_default().push(i);
    }
    groups.into_values().filter(|g| g.len() >= 2).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageBuffer, Rgb};

    fn solid(r: u8, g: u8, b: u8) -> DynamicImage {
        DynamicImage::ImageRgb8(ImageBuffer::from_pixel(64, 64, Rgb([r, g, b])))
    }

    #[test]
    fn identical_images_same_phash() {
        let a = solid(100, 120, 140);
        let b = solid(100, 120, 140);
        assert_eq!(phash(&a), phash(&b));
        assert_eq!(hamming(phash(&a), phash(&b)), 0);
    }

    #[test]
    fn different_images_differ() {
        let a = solid(0, 0, 0);
        let b = solid(255, 255, 255);
        assert!(hamming(phash(&a), phash(&b)) > 0);
    }

    #[test]
    fn exact_blake3_clusters() {
        let records = vec![
            ImageRecord {
                path: PathBuf::from("a.jpg"),
                blake3: "aaa".into(),
                phash: Some(0),
                dhash: None,
                preview_w: 10,
                preview_h: 10,
                size: 1,
                is_raw_or_dng: false,
            },
            ImageRecord {
                path: PathBuf::from("b.jpg"),
                blake3: "aaa".into(),
                phash: Some(0),
                dhash: None,
                preview_w: 10,
                preview_h: 10,
                size: 1,
                is_raw_or_dng: false,
            },
            ImageRecord {
                path: PathBuf::from("c.jpg"),
                blake3: "bbb".into(),
                phash: Some(u64::MAX),
                dhash: None,
                preview_w: 10,
                preview_h: 10,
                size: 1,
                is_raw_or_dng: false,
            },
        ];
        let groups = cluster_duplicates(&records, 3, false);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].len(), 2);
    }
}
