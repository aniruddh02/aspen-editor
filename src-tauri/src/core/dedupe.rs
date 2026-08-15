use image::DynamicImage;
use rustdct::DctPlanner;
use std::collections::HashMap;
use std::path::PathBuf;

/// Compute 64-bit perceptual hash (DCT pHash).
pub fn phash(img: &DynamicImage) -> u64 {
    let gray = normalize_luma(img.to_luma8());
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
    let gray = normalize_luma(img.to_luma8());
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

/// Stretch the useful luminance range before perceptual hashing. This makes
/// hashes robust to exposure-only variants while clipping the extreme 1% tails
/// so a few hot or dead pixels do not control the transform.
fn normalize_luma(mut gray: image::GrayImage) -> image::GrayImage {
    let total = gray.pixels().count() as u64;
    if total == 0 {
        return gray;
    }

    let mut histogram = [0u64; 256];
    for pixel in gray.pixels() {
        histogram[pixel.0[0] as usize] += 1;
    }

    let tail = (total / 100).max(1);
    let mut cumulative = 0u64;
    let mut low = 0usize;
    for (value, count) in histogram.iter().enumerate() {
        cumulative += count;
        if cumulative >= tail {
            low = value;
            break;
        }
    }

    cumulative = 0;
    let mut high = 255usize;
    for (value, count) in histogram.iter().enumerate().rev() {
        cumulative += count;
        if cumulative >= tail {
            high = value;
            break;
        }
    }

    if high <= low {
        return gray;
    }

    let scale = 255.0 / (high - low) as f64;
    for pixel in gray.pixels_mut() {
        pixel.0[0] = (((pixel.0[0] as usize).clamp(low, high) - low) as f64 * scale).round() as u8;
    }
    gray
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

    fn members_of(&mut self, n: usize, idx: usize) -> Vec<usize> {
        let root = self.find(idx);
        (0..n).filter(|&i| self.find(i) == root).collect()
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

    // Perceptual near-duplicates. Merge nearest pairs first so a borderline
    // match cannot claim one burst member and then block the rest of the burst.
    let with_phash: Vec<(usize, u64)> = records
        .iter()
        .enumerate()
        .filter_map(|(i, r)| r.phash.map(|h| (i, h)))
        .collect();

    let mut edges: Vec<(u32, usize, usize)> = Vec::new();
    for i in 0..with_phash.len() {
        for j in (i + 1)..with_phash.len() {
            let (ia, ha) = with_phash[i];
            let (ib, hb) = with_phash[j];
            if pair_is_near(records, ia, ib, hamming_threshold, confirm_dhash) {
                edges.push((hamming(ha, hb), ia, ib));
            }
        }
    }
    edges.sort_unstable_by_key(|(dist, ia, ib)| (*dist, *ia, *ib));

    for (_, ia, ib) in edges {
        // Complete-linkage: merge only when every member of both clusters
        // is within the threshold. Single-linkage union-find chained
        // A~B and B~C into {A,B,C} even when A and C were different
        // frames (client run e2159946: diameter 15 at threshold 7).
        if clusters_within_threshold(
            &mut uf,
            records,
            n,
            ia,
            ib,
            hamming_threshold,
            confirm_dhash,
        ) {
            uf.union(ia, ib);
        }
    }

    let mut groups: HashMap<usize, Vec<usize>> = HashMap::new();
    for i in 0..n {
        groups.entry(uf.find(i)).or_default().push(i);
    }
    groups.into_values().filter(|g| g.len() >= 2).collect()
}

fn pair_is_near(
    records: &[ImageRecord],
    ia: usize,
    ib: usize,
    hamming_threshold: u32,
    confirm_dhash: bool,
) -> bool {
    let (Some(ha), Some(hb)) = (records[ia].phash, records[ib].phash) else {
        return false;
    };
    if hamming(ha, hb) > hamming_threshold {
        return false;
    }
    if confirm_dhash {
        if let (Some(da), Some(db)) = (records[ia].dhash, records[ib].dhash) {
            // dHash is more sensitive than pHash to clipped shadows and
            // highlights. Keep it as a confirmation signal, but allow a
            // small exposure tolerance after pHash matched.
            if hamming(da, db) > hamming_threshold + 4 {
                return false;
            }
        }
    }
    true
}

fn clusters_within_threshold(
    uf: &mut UnionFind,
    records: &[ImageRecord],
    n: usize,
    ia: usize,
    ib: usize,
    hamming_threshold: u32,
    confirm_dhash: bool,
) -> bool {
    if uf.find(ia) == uf.find(ib) {
        return true;
    }
    let a_members = uf.members_of(n, ia);
    let b_members = uf.members_of(n, ib);
    a_members.iter().all(|&a| {
        b_members
            .iter()
            .all(|&b| pair_is_near(records, a, b, hamming_threshold, confirm_dhash))
    })
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

    fn hashed(name: &str, blake: &str, phash: u64) -> ImageRecord {
        ImageRecord {
            path: PathBuf::from(name),
            blake3: blake.into(),
            phash: Some(phash),
            dhash: None,
            preview_w: 10,
            preview_h: 10,
            size: 1,
            is_raw_or_dng: true,
        }
    }

    fn sorted_groups(records: &[ImageRecord], threshold: u32) -> Vec<Vec<String>> {
        let mut groups: Vec<Vec<String>> = cluster_duplicates(records, threshold, false)
            .into_iter()
            .map(|g| {
                let mut names: Vec<String> = g
                    .into_iter()
                    .map(|i| records[i].path.display().to_string())
                    .collect();
                names.sort();
                names
            })
            .collect();
        groups.sort();
        groups
    }

    #[test]
    fn chain_of_near_matches_does_not_merge_distant_endpoints() {
        // A-B=5, B-C=5, A-C=10. Single-linkage would make one group of three.
        let records = vec![
            hashed("a.arw", "a", 0),
            hashed("b.arw", "b", 0b1_1111),
            hashed("c.arw", "c", 0b1_1111 | (0b1_1111 << 5)),
        ];
        assert_eq!(
            hamming(records[0].phash.unwrap(), records[1].phash.unwrap()),
            5
        );
        assert_eq!(
            hamming(records[1].phash.unwrap(), records[2].phash.unwrap()),
            5
        );
        assert_eq!(
            hamming(records[0].phash.unwrap(), records[2].phash.unwrap()),
            10
        );
        let groups = sorted_groups(&records, 7);
        assert_eq!(groups.len(), 1);
        assert!(
            groups[0] == ["a.arw", "b.arw"] || groups[0] == ["b.arw", "c.arw"],
            "expected a tight pair, got {groups:?}"
        );
    }

    #[test]
    fn mutual_near_matches_still_form_one_group() {
        let records = vec![
            hashed("a.arw", "a", 0),
            hashed("b.arw", "b", 0b111),
            hashed("c.arw", "c", 0b1_1000),
        ];
        assert_eq!(hamming(0, 0b111), 3);
        assert_eq!(hamming(0, 0b1_1000), 2);
        assert_eq!(hamming(0b111, 0b1_1000), 5);
        let groups = sorted_groups(&records, 7);
        assert_eq!(groups, vec![vec!["a.arw", "b.arw", "c.arw"]]);
    }

    #[test]
    fn client_burst_does_not_absorb_the_sharper_different_frame() {
        // pHashes from run e2159946 group 0. Balanced threshold 7 used to
        // chain _ANI7997 onto the 7998-8000 burst (diameter 10) so the
        // sharper unrelated frame won.
        let records = vec![
            hashed("_ANI7997.ARW", "7997", 0x4402_8d16_73e9_06f1),
            hashed("_ANI7998.ARW", "7998", 0x4400_8d16_13c9_02fd),
            hashed("_ANI7999.ARW", "7999", 0x4000_4d16_13c9_02fd),
            hashed("_ANI8000.ARW", "8000", 0x0400_0d16_13e9_027d),
        ];
        let groups = sorted_groups(&records, 7);
        assert_eq!(
            groups,
            vec![vec![
                "_ANI7998.ARW".to_string(),
                "_ANI7999.ARW".to_string(),
                "_ANI8000.ARW".to_string()
            ]]
        );
    }
}
