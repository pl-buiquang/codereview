//! A port of pixelmatch (YIQ perceptual delta + anti-aliasing detection), the algorithm behind the
//! `snapshot-diff` skill. Unlike the skill, anti-aliased pixels are told apart from real changes
//! (yellow, counted apart) so they can be left out of the regions.

const THRESHOLD: f64 = 0.1;
const ALPHA: f64 = 0.1;
const MAX_DELTA: f64 = 35215.0 * THRESHOLD * THRESHOLD;

/// `mask` values.
pub const SAME: u8 = 0;
pub const CHANGED: u8 = 1;
pub const ANTI_ALIASED: u8 = 2;

pub struct Compared {
    /// Real differences (red in `diff`).
    pub changed: u64,
    /// Anti-aliasing differences (yellow in `diff`).
    pub anti_aliased: u64,
    /// RGBA: unchanged pixels faded to grey, changes red, anti-aliasing yellow.
    pub diff: Vec<u8>,
    /// One byte per pixel: [`SAME`], [`CHANGED`] or [`ANTI_ALIASED`].
    pub mask: Vec<u8>,
}

fn blend(c: f64, a: f64) -> f64 {
    255.0 + (c - 255.0) * a
}

fn rgb2y(r: f64, g: f64, b: f64) -> f64 {
    r * 0.29889531 + g * 0.58662247 + b * 0.11448223
}

fn rgb2i(r: f64, g: f64, b: f64) -> f64 {
    r * 0.59597799 - g * 0.27417610 - b * 0.32180189
}

fn rgb2q(r: f64, g: f64, b: f64) -> f64 {
    r * 0.21147017 - g * 0.52261711 + b * 0.31114694
}

/// Perceptual distance of pixel `k` of `img1` and pixel `m` of `img2`, negative when `img1` is the
/// lighter one, as in pixelmatch (`y_only` returns just the brightness difference).
fn color_delta(img1: &[u8], img2: &[u8], k: usize, m: usize, y_only: bool) -> f64 {
    let (mut r1, mut g1, mut b1, a1) = (img1[k] as f64, img1[k + 1] as f64, img1[k + 2] as f64, img1[k + 3] as f64);
    let (mut r2, mut g2, mut b2, a2) = (img2[m] as f64, img2[m + 1] as f64, img2[m + 2] as f64, img2[m + 3] as f64);
    if a1 == a2 && r1 == r2 && g1 == g2 && b1 == b2 {
        return 0.0;
    }
    if a1 < 255.0 {
        let a = a1 / 255.0;
        r1 = blend(r1, a);
        g1 = blend(g1, a);
        b1 = blend(b1, a);
    }
    if a2 < 255.0 {
        let a = a2 / 255.0;
        r2 = blend(r2, a);
        g2 = blend(g2, a);
        b2 = blend(b2, a);
    }
    let y1 = rgb2y(r1, g1, b1);
    let y2 = rgb2y(r2, g2, b2);
    let y = y1 - y2;
    if y_only {
        return y;
    }
    let i = rgb2i(r1, g1, b1) - rgb2i(r2, g2, b2);
    let q = rgb2q(r1, g1, b1) - rgb2q(r2, g2, b2);
    let delta = 0.5053 * y * y + 0.299 * i * i + 0.1957 * q * q;
    if y1 > y2 {
        -delta
    } else {
        delta
    }
}

fn has_many_siblings(img: &[u8], x1: usize, y1: usize, w: usize, h: usize) -> bool {
    let x0 = x1.saturating_sub(1);
    let y0 = y1.saturating_sub(1);
    let x2 = (x1 + 1).min(w - 1);
    let y2 = (y1 + 1).min(h - 1);
    let pos = (y1 * w + x1) * 4;
    let mut zeroes = usize::from(x1 == x0 || x1 == x2 || y1 == y0 || y1 == y2);
    for x in x0..=x2 {
        for y in y0..=y2 {
            if x == x1 && y == y1 {
                continue;
            }
            let pos2 = (y * w + x) * 4;
            if img[pos..pos + 4] == img[pos2..pos2 + 4] {
                zeroes += 1;
            }
        }
    }
    zeroes > 2
}

/// Whether the pixel at (x1, y1) of `img` looks like anti-aliasing: it sits on a brightness
/// gradient and its darkest/lightest neighbour has identical siblings in both images.
fn antialiased(img: &[u8], x1: usize, y1: usize, w: usize, h: usize, img2: &[u8]) -> bool {
    let x0 = x1.saturating_sub(1);
    let y0 = y1.saturating_sub(1);
    let x2 = (x1 + 1).min(w - 1);
    let y2 = (y1 + 1).min(h - 1);
    let pos = (y1 * w + x1) * 4;
    let mut zeroes = usize::from(x1 == x0 || x1 == x2 || y1 == y0 || y1 == y2);
    let (mut min, mut max) = (0.0f64, 0.0f64);
    let (mut min_xy, mut max_xy) = ((0, 0), (0, 0));
    for x in x0..=x2 {
        for y in y0..=y2 {
            if x == x1 && y == y1 {
                continue;
            }
            let delta = color_delta(img, img, pos, (y * w + x) * 4, true);
            if delta == 0.0 {
                zeroes += 1;
                if zeroes > 2 {
                    return false;
                }
            } else if delta < min {
                min = delta;
                min_xy = (x, y);
            } else if delta > max {
                max = delta;
                max_xy = (x, y);
            }
        }
    }
    if min == 0.0 || max == 0.0 {
        return false;
    }
    (has_many_siblings(img, min_xy.0, min_xy.1, w, h) && has_many_siblings(img2, min_xy.0, min_xy.1, w, h))
        || (has_many_siblings(img, max_xy.0, max_xy.1, w, h) && has_many_siblings(img2, max_xy.0, max_xy.1, w, h))
}

/// Compare two RGBA images of the same `w`×`h`.
pub fn compare(img1: &[u8], img2: &[u8], w: usize, h: usize) -> Compared {
    assert_eq!(img1.len(), w * h * 4);
    assert_eq!(img2.len(), w * h * 4);
    let mut diff = vec![0u8; w * h * 4];
    let mut mask = vec![SAME; w * h];
    let (mut changed, mut anti_aliased) = (0u64, 0u64);
    for y in 0..h {
        for x in 0..w {
            let n = y * w + x;
            let pos = n * 4;
            let delta = if img1[pos..pos + 4] == img2[pos..pos + 4] {
                0.0
            } else {
                color_delta(img1, img2, pos, pos, false)
            };
            if delta.abs() > MAX_DELTA {
                if antialiased(img1, x, y, w, h, img2) || antialiased(img2, x, y, w, h, img1) {
                    mask[n] = ANTI_ALIASED;
                    anti_aliased += 1;
                    diff[pos..pos + 4].copy_from_slice(&[255, 255, 0, 255]);
                } else {
                    mask[n] = CHANGED;
                    changed += 1;
                    diff[pos..pos + 4].copy_from_slice(&[255, 0, 0, 255]);
                }
            } else {
                let a = img1[pos + 3] as f64 / 255.0;
                let y = rgb2y(img1[pos] as f64, img1[pos + 1] as f64, img1[pos + 2] as f64);
                let v = blend(y, ALPHA * a).clamp(0.0, 255.0) as u8;
                diff[pos..pos + 4].copy_from_slice(&[v, v, v, 255]);
            }
        }
    }
    Compared { changed, anti_aliased, diff, mask }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(w: usize, h: usize, rgba: [u8; 4]) -> Vec<u8> {
        rgba.iter().copied().cycle().take(w * h * 4).collect()
    }

    #[test]
    fn identical_images_have_no_difference() {
        let a = solid(4, 4, [10, 20, 30, 255]);
        let r = compare(&a, &a.clone(), 4, 4);
        assert_eq!((r.changed, r.anti_aliased), (0, 0));
        assert!(r.mask.iter().all(|m| *m == SAME));
    }

    #[test]
    fn one_changed_pixel_is_counted_and_painted_red() {
        let a = solid(5, 5, [255, 255, 255, 255]);
        let mut b = a.clone();
        let pos = (2 * 5 + 2) * 4;
        b[pos..pos + 4].copy_from_slice(&[0, 0, 0, 255]);
        let r = compare(&a, &b, 5, 5);
        assert_eq!(r.changed, 1);
        assert_eq!(r.mask[2 * 5 + 2], CHANGED);
        assert_eq!(&r.diff[pos..pos + 4], &[255, 0, 0, 255]);
    }

    #[test]
    fn a_tiny_colour_shift_stays_under_the_threshold() {
        let a = solid(3, 3, [100, 100, 100, 255]);
        let b = solid(3, 3, [101, 100, 100, 255]);
        assert_eq!(compare(&a, &b, 3, 3).changed, 0);
    }

    #[test]
    fn a_pixel_on_an_edge_gradient_is_anti_aliasing_not_a_change() {
        // Left half black, right half white; the middle column differs by a grey step in `b`.
        let (w, h) = (6, 6);
        let mut a = solid(w, h, [0, 0, 0, 255]);
        let mut b = a.clone();
        for y in 0..h {
            for x in 3..w {
                let p = (y * w + x) * 4;
                a[p..p + 3].copy_from_slice(&[255, 255, 255]);
                b[p..p + 3].copy_from_slice(&[255, 255, 255]);
            }
        }
        // In `a` the edge pixel (2,3) is mid grey; in `b` it is black.
        let p = (3 * w + 2) * 4;
        a[p..p + 3].copy_from_slice(&[128, 128, 128]);
        let r = compare(&a, &b, w, h);
        assert_eq!(r.changed, 0);
        assert_eq!(r.anti_aliased, 1);
        assert_eq!(r.mask[3 * w + 2], ANTI_ALIASED);
    }
}
