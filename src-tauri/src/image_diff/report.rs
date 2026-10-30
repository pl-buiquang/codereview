//! The AI report: what a model wrote to `report.json` after looking at the region crops, checked
//! against the manifest and turned into `report.final.json` plus one before/after/diff crop set
//! per finding (`report/<n>-{before,after,diff}.png`). The panel only ever reads the final file.

use std::path::Path;

use serde::{Deserialize, Serialize};

use super::job::{read_png, write_window, Raster};
use super::store::{self, Change, FileEntry, Manifest};
use crate::error::{AppError, AppResult};

pub const RAW: &str = "report.json";
pub const MAX_FINDINGS: usize = 60;
pub const KINDS: &[&str] = &["layout", "text", "color", "content", "noise"];

#[derive(Debug, Deserialize)]
struct RawFinding {
    file: String,
    #[serde(default)]
    bbox: Option<[i64; 4]>,
    #[serde(default)]
    region_ids: Vec<u32>,
    title: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    kind: String,
}

#[derive(Debug, Deserialize)]
struct RawReport {
    #[serde(default)]
    summary: String,
    #[serde(default)]
    findings: Vec<RawFinding>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Finding {
    /// Crops are `report/<n>-{before,after,diff}.png`.
    pub n: usize,
    pub file: String,
    /// The manifest file the finding is about (its `f<index>` directory).
    pub file_index: usize,
    /// x, y, w, h in the compared (padded) image.
    pub bbox: [u32; 4],
    pub region_ids: Vec<u32>,
    pub title: String,
    pub description: String,
    pub kind: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Report {
    pub summary: String,
    pub generated_at: String,
    pub findings: Vec<Finding>,
    /// Findings left out because they named an unknown file or were not about a compared pair.
    pub dropped: usize,
}

fn compared_size(f: &FileEntry) -> Option<(u32, u32)> {
    let (b, a) = (f.before?, f.after?);
    Some((b.width.max(a.width), b.height.max(a.height)))
}

/// The box of a finding, clamped into the compared image: the model's `bbox`, else the union of
/// its regions.
fn resolve_bbox(raw: &RawFinding, file: &FileEntry, (w, h): (u32, u32)) -> Option<[u32; 4]> {
    let union = || {
        let mut it = raw.region_ids.iter().filter_map(|id| file.regions.iter().find(|r| r.id == *id));
        let first = it.next()?;
        let (mut x0, mut y0, mut x1, mut y1) = (first.x, first.y, first.x + first.w, first.y + first.h);
        for r in it {
            x0 = x0.min(r.x);
            y0 = y0.min(r.y);
            x1 = x1.max(r.x + r.w);
            y1 = y1.max(r.y + r.h);
        }
        Some([x0, y0, x1 - x0, y1 - y0])
    };
    let [x, y, bw, bh] = match raw.bbox {
        Some([x, y, bw, bh]) if bw > 0 && bh > 0 => [x.max(0) as u32, y.max(0) as u32, bw as u32, bh as u32],
        _ => union()?,
    };
    let x = x.min(w - 1);
    let y = y.min(h - 1);
    Some([x, y, bw.min(w - x), bh.min(h - y)])
}

/// Check `dir/report.json` against the manifest, write the crops of every finding and the final
/// report, and return it.
pub fn finalize(dir: &Path) -> AppResult<Report> {
    let manifest = store::read_manifest(dir).ok_or_else(|| AppError::Other("the image diffs are gone".into()))?;
    let bytes = std::fs::read(dir.join(RAW)).map_err(|_| AppError::Other("the run did not write report.json".into()))?;
    let raw: RawReport = serde_json::from_slice(&bytes).map_err(|e| AppError::Other(format!("report.json is not valid: {e}")))?;
    let report = build(dir, &manifest, raw)?;
    store::write_json(dir, store::REPORT_FINAL, &report)?;
    Ok(report)
}

fn build(dir: &Path, manifest: &Manifest, raw: RawReport) -> AppResult<Report> {
    let out = dir.join("report");
    let _ = std::fs::remove_dir_all(&out);
    std::fs::create_dir_all(&out)?;
    let mut findings = Vec::new();
    let mut dropped = 0;
    // Full-size pictures are decoded once per file, and only when a crop can't be copied.
    let mut loaded: Option<(usize, Raster, Raster, Raster)> = None;
    for r in raw.findings.into_iter().take(MAX_FINDINGS * 2) {
        if findings.len() >= MAX_FINDINGS {
            dropped += 1;
            continue;
        }
        let file = manifest.files.iter().find(|f| f.path == r.file || f.old_path.as_deref() == Some(&r.file));
        let usable = file.filter(|f| f.skipped.is_none() && matches!(f.change, Change::Modified | Change::Renamed));
        let Some((file, size)) = usable.and_then(|f| Some((f, compared_size(f)?))) else {
            dropped += 1;
            continue;
        };
        let Some(bbox) = resolve_bbox(&r, file, size) else {
            dropped += 1;
            continue;
        };
        let n = findings.len();
        let fdir = dir.join(format!("f{}", file.index));
        // A finding that is exactly one region already has its crops.
        let single = match r.region_ids.as_slice() {
            [id] => file.regions.iter().find(|g| g.id == *id).filter(|g| [g.x, g.y, g.w, g.h] == bbox),
            _ => None,
        };
        for slot in ["before", "after", "diff"] {
            let target = out.join(format!("{n}-{slot}.png"));
            if let Some(g) = single {
                std::fs::copy(fdir.join(format!("crop-{}-{slot}.png", g.id)), &target)?;
                continue;
            }
            if loaded.as_ref().is_none_or(|(i, ..)| *i != file.index) {
                let read = |name: &str| read_png(&fdir.join(name)).map_err(AppError::Other);
                loaded = Some((file.index, read("before.png")?, read("after.png")?, read("diff.png")?));
            }
            let (_, before, after, diff) = loaded.as_ref().expect("just loaded");
            let img = match slot {
                "before" => before,
                "after" => after,
                _ => diff,
            };
            // The compared image is the larger of both sides, white where a side is smaller.
            let (w, h) = size;
            let padded;
            let rgba: &[u8] = if (img.w, img.h) == (w, h) {
                &img.rgba
            } else {
                padded = pad_white(img, w, h);
                &padded
            };
            write_window(&target, (bbox[0], bbox[1], bbox[2], bbox[3]), rgba, w, h).map_err(AppError::Other)?;
        }
        let kind = if KINDS.contains(&r.kind.as_str()) { r.kind } else { "content".into() };
        findings.push(Finding {
            n,
            file: file.path.clone(),
            file_index: file.index,
            bbox,
            region_ids: r.region_ids,
            title: r.title.trim().to_string(),
            description: r.description.trim().to_string(),
            kind,
        });
    }
    Ok(Report { summary: raw.summary.trim().to_string(), generated_at: chrono::Utc::now().to_rfc3339(), findings, dropped })
}

fn pad_white(img: &Raster, w: u32, h: u32) -> Vec<u8> {
    let mut out = vec![255u8; w as usize * h as usize * 4];
    for y in 0..img.h.min(h) {
        let n = img.w.min(w) as usize * 4;
        let src = y as usize * img.w as usize * 4;
        let dst = y as usize * w as usize * 4;
        out[dst..dst + n].copy_from_slice(&img.rgba[src..src + n]);
    }
    out
}

pub fn read_final(dir: &Path) -> Option<Report> {
    serde_json::from_slice(&std::fs::read(dir.join(store::REPORT_FINAL)).ok()?).ok()
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicBool;

    use super::*;
    use crate::image_diff::blob::{BlobSource, Side};
    use crate::image_diff::job::generate;
    use crate::image_diff::{store::MANIFEST_VERSION, ImageFile};

    struct Fake(Vec<(Side, Vec<u8>)>);

    impl BlobSource for Fake {
        fn read(&self, side: Side, _: &str) -> AppResult<Vec<u8>> {
            self.0.iter().find(|(s, _)| *s == side).map(|(_, b)| b.clone()).ok_or_else(|| AppError::Other("missing".into()))
        }
    }

    fn png(w: u32, h: u32, squares: &[(u32, u32, u32)]) -> Vec<u8> {
        let mut img = image::RgbaImage::from_pixel(w, h, image::Rgba([255, 255, 255, 255]));
        for (sx, sy, s) in squares {
            for y in *sy..sy + s {
                for x in *sx..sx + s {
                    img.put_pixel(x, y, image::Rgba([0, 0, 0, 255]));
                }
            }
        }
        let mut out = Vec::new();
        image::DynamicImage::ImageRgba8(img).write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
        out
    }

    /// A generated cache of one image with two far-apart squares (two regions).
    fn cache(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("cr-report-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let blobs = Fake(vec![(Side::Left, png(300, 200, &[])), (Side::Right, png(300, 200, &[(10, 10, 6), (250, 150, 6)]))]);
        let file = ImageFile { path: "a.png".into(), old_path: None, change: Change::Modified };
        let files = generate(&blobs, &[file], &dir, &AtomicBool::new(false), |_, _| {}).unwrap();
        assert_eq!(files[0].regions.len(), 2);
        let manifest = Manifest {
            version: MANIFEST_VERSION,
            review_id: 1,
            pin: "p".into(),
            base_sha: None,
            head_sha: None,
            generated_at: "t".into(),
            preview_width: 1000,
            auto: false,
            files,
        };
        store::write_json(&dir, store::MANIFEST, &manifest).unwrap();
        dir
    }

    fn write_raw(dir: &Path, json: &str) {
        std::fs::write(dir.join(RAW), json).unwrap();
    }

    #[test]
    fn a_finding_is_cropped_clamped_and_its_kind_checked() {
        let dir = cache("ok");
        write_raw(
            &dir,
            r#"{"summary":" two squares ","findings":[
              {"file":"a.png","region_ids":[0],"title":"Top left square","description":"A black square appears","kind":"content"},
              {"file":"a.png","bbox":[240,140,500,500],"region_ids":[1],"title":"Bottom right","kind":"sparkles"},
              {"file":"a.png","region_ids":[0,1],"title":"Both"}
            ]}"#,
        );
        let report = finalize(&dir).unwrap();
        assert_eq!(report.summary, "two squares");
        assert_eq!(report.dropped, 0);
        assert_eq!(report.findings.len(), 3);
        assert_eq!(report.findings[0].bbox, [10, 10, 6, 6], "taken from the region");
        assert_eq!(report.findings[1].bbox, [240, 140, 60, 60], "clamped into the 300x200 image");
        assert_eq!(report.findings[1].kind, "content", "unknown kinds fall back");
        assert_eq!(report.findings[2].bbox, [10, 10, 246, 146], "union of both regions");
        for n in 0..3 {
            for slot in ["before", "after", "diff"] {
                assert!(dir.join(format!("report/{n}-{slot}.png")).exists(), "{n}-{slot}");
            }
        }
        // The single-region finding reuses the region crop byte for byte.
        assert_eq!(
            std::fs::read(dir.join("report/0-diff.png")).unwrap(),
            std::fs::read(dir.join("f0/crop-0-diff.png")).unwrap()
        );
        assert_eq!(read_final(&dir).unwrap(), report);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn findings_about_unknown_files_or_nothing_locatable_are_dropped() {
        let dir = cache("drop");
        write_raw(
            &dir,
            r#"{"summary":"","findings":[
              {"file":"nope.png","region_ids":[0],"title":"x"},
              {"file":"a.png","title":"no box and no region"},
              {"file":"a.png","bbox":[0,0,0,0],"region_ids":[9],"title":"empty box, unknown region"},
              {"file":"a.png","bbox":[20,20,10,10],"title":"kept: the box alone is enough"}
            ]}"#,
        );
        let report = finalize(&dir).unwrap();
        assert_eq!((report.findings.len(), report.dropped), (1, 3));
        assert_eq!(report.findings[0].bbox, [20, 20, 10, 10]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_missing_or_broken_report_is_an_error_not_an_empty_report() {
        let dir = cache("bad");
        assert!(finalize(&dir).unwrap_err().to_string().contains("did not write report.json"));
        write_raw(&dir, "not json");
        assert!(finalize(&dir).unwrap_err().to_string().contains("not valid"));
        assert!(read_final(&dir).is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
