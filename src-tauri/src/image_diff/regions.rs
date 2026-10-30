//! Groups changed pixels into rectangular regions: the places worth a close-up crop.

use serde::{Deserialize, Serialize};

use super::pixelmatch::CHANGED;

/// Side of the grid cells changed pixels are bucketed in.
const CELL: usize = 16;
/// Cells within this many cells of each other (about 48 px) belong to one region.
const LINK: i64 = 3;
pub const MAX_REGIONS: usize = 30;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Region {
    pub id: u32,
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    pub changed_px: u64,
}

#[derive(Debug, Default, PartialEq)]
pub struct Regions {
    pub regions: Vec<Region>,
    /// Regions beyond [`MAX_REGIONS`] (the smallest ones), not listed.
    pub other: u32,
}

#[derive(Clone, Copy)]
struct Cell {
    x0: u32,
    y0: u32,
    x1: u32,
    y1: u32,
    px: u64,
}

impl Cell {
    const EMPTY: Cell = Cell { x0: u32::MAX, y0: u32::MAX, x1: 0, y1: 0, px: 0 };

    fn add_pixel(&mut self, x: u32, y: u32) {
        self.x0 = self.x0.min(x);
        self.y0 = self.y0.min(y);
        self.x1 = self.x1.max(x);
        self.y1 = self.y1.max(y);
        self.px += 1;
    }

    fn union(&mut self, o: &Cell) {
        self.x0 = self.x0.min(o.x0);
        self.y0 = self.y0.min(o.y0);
        self.x1 = self.x1.max(o.x1);
        self.y1 = self.y1.max(o.y1);
        self.px += o.px;
    }
}

fn find(parent: &mut [usize], mut i: usize) -> usize {
    while parent[i] != i {
        parent[i] = parent[parent[i]];
        i = parent[i];
    }
    i
}

/// Regions of the [`CHANGED`] pixels of a `w`×`h` mask (anti-aliased pixels don't count).
pub fn detect(mask: &[u8], w: usize, h: usize) -> Regions {
    if w == 0 || h == 0 {
        return Regions::default();
    }
    let (gw, gh) = (w.div_ceil(CELL), h.div_ceil(CELL));
    let mut cells = vec![Cell::EMPTY; gw * gh];
    for y in 0..h {
        let row = &mask[y * w..(y + 1) * w];
        for (x, m) in row.iter().enumerate() {
            if *m == CHANGED {
                cells[(y / CELL) * gw + x / CELL].add_pixel(x as u32, y as u32);
            }
        }
    }
    let mut parent: Vec<usize> = (0..cells.len()).collect();
    for cy in 0..gh {
        for cx in 0..gw {
            let i = cy * gw + cx;
            if cells[i].px == 0 {
                continue;
            }
            // Only look forward (down/right half of the window): the other half links back to us.
            for dy in 0..=LINK {
                for dx in -LINK..=LINK {
                    if dy == 0 && dx <= 0 {
                        continue;
                    }
                    let (nx, ny) = (cx as i64 + dx, cy as i64 + dy);
                    if nx < 0 || ny < 0 || nx >= gw as i64 || ny >= gh as i64 {
                        continue;
                    }
                    let j = ny as usize * gw + nx as usize;
                    if cells[j].px > 0 {
                        let (a, b) = (find(&mut parent, i), find(&mut parent, j));
                        if a != b {
                            parent[a] = b;
                        }
                    }
                }
            }
        }
    }
    let mut groups: Vec<Option<Cell>> = vec![None; cells.len()];
    for (i, cell) in cells.iter().enumerate() {
        if cell.px == 0 {
            continue;
        }
        let root = find(&mut parent, i);
        match &mut groups[root] {
            Some(g) => g.union(cell),
            slot => *slot = Some(*cell),
        }
    }
    let mut found: Vec<Cell> = groups.into_iter().flatten().collect();
    // The biggest first, so the cut drops the smallest.
    found.sort_by(|a, b| b.px.cmp(&a.px));
    let other = found.len().saturating_sub(MAX_REGIONS) as u32;
    found.truncate(MAX_REGIONS);
    // Reading order, ids follow it.
    found.sort_by_key(|c| (c.y0, c.x0));
    let regions = found
        .into_iter()
        .enumerate()
        .map(|(id, c)| Region {
            id: id as u32,
            x: c.x0,
            y: c.y0,
            w: c.x1 - c.x0 + 1,
            h: c.y1 - c.y0 + 1,
            changed_px: c.px,
        })
        .collect();
    Regions { regions, other }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image_diff::pixelmatch::{ANTI_ALIASED, SAME};

    fn mask_with(w: usize, h: usize, px: &[(usize, usize)]) -> Vec<u8> {
        let mut m = vec![SAME; w * h];
        for (x, y) in px {
            m[y * w + x] = CHANGED;
        }
        m
    }

    #[test]
    fn nothing_changed_has_no_region() {
        assert_eq!(detect(&vec![SAME; 100 * 100], 100, 100), Regions::default());
    }

    #[test]
    fn nearby_pixels_form_one_exact_bbox_and_far_ones_another() {
        let m = mask_with(400, 400, &[(10, 10), (30, 20), (350, 380)]);
        let r = detect(&m, 400, 400);
        assert_eq!(r.other, 0);
        assert_eq!(r.regions.len(), 2);
        assert_eq!(r.regions[0], Region { id: 0, x: 10, y: 10, w: 21, h: 11, changed_px: 2 });
        assert_eq!(r.regions[1], Region { id: 1, x: 350, y: 380, w: 1, h: 1, changed_px: 1 });
    }

    #[test]
    fn gaps_under_the_link_distance_merge() {
        // 40 px apart horizontally: 3 cells, within the link distance.
        let m = mask_with(300, 50, &[(10, 10), (50, 10)]);
        assert_eq!(detect(&m, 300, 50).regions.len(), 1);
        // 100 px apart: separate.
        let m = mask_with(300, 50, &[(10, 10), (110, 10)]);
        assert_eq!(detect(&m, 300, 50).regions.len(), 2);
    }

    #[test]
    fn anti_aliasing_does_not_make_a_region() {
        let mut m = vec![SAME; 50 * 50];
        m[25 * 50 + 25] = ANTI_ALIASED;
        assert!(detect(&m, 50, 50).regions.is_empty());
    }

    #[test]
    fn regions_are_capped_keeping_the_biggest() {
        // 40 isolated pixels, 200 px apart, plus a big blob that must survive the cut.
        let (w, h) = (8200, 220);
        let mut m = vec![SAME; w * h];
        for i in 0..40 {
            m[10 * w + i * 200] = CHANGED;
        }
        for y in 100..110 {
            for x in 1000..1010 {
                m[y * w + x] = CHANGED;
            }
        }
        let r = detect(&m, w, h);
        assert_eq!(r.regions.len(), MAX_REGIONS);
        assert_eq!(r.other, 11);
        assert!(r.regions.iter().any(|r| r.changed_px == 100));
        assert!(r.regions.windows(2).all(|p| (p[0].y, p[0].x) <= (p[1].y, p[1].x)));
    }
}
