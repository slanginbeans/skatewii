//! Ported from SK8-ENGINE/skate-3-rust-engine crates/skate-data/src/ocean_pca.rs (GPL-3.0).
//!
//! The built-in PCA ocean/water animation table (cPCAWaterAnimationData) from
//! an unpacked retail executable, written as `assets/private/ocean-pca.json`.
//!
//! TU3 keeps the 30 frame means at 0x830118D8 and the weights right after them
//! at 0x83011A40 (tools/asset_pipeline/ocean_pca.py). Other builds place the
//! table elsewhere, so it is located the way the code reaches it: a `lis`
//! followed by a D-form use of the same register forms each address, and
//! cPCAWaterAnimationData::Init forms both within a few instructions.
use crate::xex::XexImage;

const FRAMES: usize = 30;
const MEANS: usize = FRAMES * 3;
const WEIGHTS: usize = FRAMES * 24;
const WEIGHTS_OFFSET: u32 = (MEANS * 4) as u32; // 0x168
/// Instructions between `lis` and the instruction completing an address.
const ADDRESS_WINDOW: usize = 8;
/// Instructions between the two address loads.
const PAIR_WINDOW: usize = 32;

pub struct OceanPca {
    pub means_address: u32,
    /// Per frame: mean row then six weight rows, in shader order, scaled /255.
    pub frames: Vec<[[f32; 4]; 7]>,
}

pub fn extract(image: &XexImage) -> Result<OceanPca, String> {
    let candidates = locate(image);
    let means_address = match candidates.as_slice() {
        [one] => *one,
        [] => return Err("Ocean PCA table not found in the executable".into()),
        many => {
            return Err(format!(
                "Ocean PCA table is ambiguous: {} candidates ({:08X?})",
                many.len(),
                many
            ));
        }
    };
    let means = floats(image, means_address, MEANS).ok_or("Ocean PCA means out of range")?;
    let weights = floats(image, means_address + WEIGHTS_OFFSET, WEIGHTS)
        .ok_or("Ocean PCA weights out of range")?;
    let frames = (0..FRAMES)
        .map(|frame| {
            let mean = &means[frame * 3..frame * 3 + 3];
            let weight = &weights[frame * 24..frame * 24 + 24];
            let mut rows = [[0.; 4]; 7];
            // Shader model order: mean X/Z/Y and weight pairs R/B/G.
            rows[0] = [mean[0] / 255., mean[2] / 255., mean[1] / 255., 0.];
            for (row, start) in rows[1..].iter_mut().zip([0, 4, 16, 20, 8, 12]) {
                *row = core::array::from_fn(|i| weight[start + i] / 255.);
            }
            rows
        })
        .collect();
    Ok(OceanPca {
        means_address,
        frames,
    })
}

impl OceanPca {
    /// Pack entry `xex/ocean_pca`: 30 frames (played at 30 Hz) x 7 rows x 4 big-endian f32.
    pub fn to_bytes(&self) -> Vec<u8> {
        self.frames
            .iter()
            .flatten()
            .flatten()
            .flat_map(|v| v.to_be_bytes())
            .collect()
    }
}

fn floats(image: &XexImage, address: u32, count: usize) -> Option<Vec<f32>> {
    let bytes = image.at(address, count * 4)?;
    let values: Vec<f32> = bytes
        .chunks_exact(4)
        .map(|b| f32::from_be_bytes([b[0], b[1], b[2], b[3]]))
        .collect();
    values.iter().all(|v| v.is_finite()).then_some(values)
}

/// A table holds 30 normal-like means (0..255 components) and nonzero weights.
fn plausible(image: &XexImage, address: u32) -> bool {
    let (Some(means), Some(weights)) = (
        floats(image, address, MEANS),
        floats(image, address + WEIGHTS_OFFSET, WEIGHTS),
    ) else {
        return false;
    };
    means.iter().all(|v| (0.0..=255.0).contains(v))
        && means.iter().any(|v| *v != 0.)
        && weights.iter().all(|v| v.abs() <= 255.)
        && weights.iter().filter(|v| **v != 0.).count() > WEIGHTS / 2
}

fn locate(image: &XexImage) -> Vec<u32> {
    let words: Vec<u32> = image
        .image
        .chunks_exact(4)
        .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
        .collect();
    // (instruction index, address) for every lis + D-form completion.
    let mut formed: Vec<(usize, u32)> = Vec::new();
    for (i, &w) in words.iter().enumerate() {
        let (op, rd, ra) = (w >> 26, (w >> 21) & 31, (w >> 16) & 31);
        if op != 15 || ra != 0 {
            continue;
        }
        let high = (w & 0xffff) << 16;
        for (j, &u) in words.iter().enumerate().skip(i + 1).take(ADDRESS_WINDOW) {
            let (uop, urd, ura) = (u >> 26, (u >> 21) & 31, (u >> 16) & 31);
            // addi and the integer/float loads and stores.
            if ura == rd && matches!(uop, 14 | 32 | 34 | 36 | 40 | 44 | 48 | 50 | 52 | 54) {
                let low = (u & 0xffff) as u16 as i16 as i32;
                formed.push((j, high.wrapping_add(low as u32)));
            }
            if uop == 15 && urd == rd {
                break;
            }
        }
    }
    let mut found: Vec<u32> = Vec::new();
    for (k, &(i, address)) in formed.iter().enumerate() {
        let pair = formed[k.saturating_sub(64)..(k + 64).min(formed.len())]
            .iter()
            .any(|&(j, other)| {
                other == address.wrapping_add(WEIGHTS_OFFSET) && i.abs_diff(j) <= PAIR_WINDOW
            });
        if pair && !found.contains(&address) && plausible(image, address) {
            found.push(address);
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put(image: &mut [u8], at: usize, w: u32) {
        image[at..at + 4].copy_from_slice(&w.to_be_bytes());
    }

    /// Synthetic image: code forms the means and weights addresses with lis + addi, the table follows.
    #[test]
    fn finds_the_table_through_the_code_that_reads_it() {
        let base = 0x8200_0000u32;
        let mut image = vec![0u8; 0x2000];
        put(&mut image, 0, (15 << 26) | (3 << 21) | 0x8200); // lis r3, 0x8200
        put(&mut image, 4, (14 << 26) | (3 << 21) | (3 << 16) | 0x1000); // addi r3, r3, 0x1000
        put(&mut image, 8, (15 << 26) | (4 << 21) | 0x8200); // lis r4, 0x8200
        put(&mut image, 12, (14 << 26) | (4 << 21) | (4 << 16) | 0x1168); // addi r4, r4, 0x1168
        for i in 0..MEANS {
            put(&mut image, 0x1000 + i * 4, (i as f32).to_bits());
        }
        for i in 0..WEIGHTS {
            put(
                &mut image,
                0x1168 + i * 4,
                (-(i as f32) / 10.0 - 1.0).to_bits(),
            );
        }
        let pca = extract(&XexImage {
            base_address: base,
            image,
        })
        .unwrap();
        assert_eq!(pca.means_address, base + 0x1000);
        assert_eq!(pca.frames.len(), FRAMES);
        // Frame 1 mean is (3, 4, 5) stored as X/Z/Y.
        assert_eq!(pca.frames[1][0], [3. / 255., 5. / 255., 4. / 255., 0.]);
        assert_eq!(pca.to_bytes().len(), FRAMES * 7 * 4 * 4);
    }

    #[test]
    fn reports_a_missing_table() {
        assert!(
            extract(&XexImage {
                base_address: 0x8200_0000,
                image: vec![0; 64]
            })
            .is_err()
        );
    }
}
