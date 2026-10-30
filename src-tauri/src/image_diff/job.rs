//! Generates the diffs of a list of image files into a cache directory, one file at a time (a
//! screenshot of 40 megapixels is 160 MB per decoded copy).

use std::fs::File;
use std::io::{BufWriter, Cursor};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

use image::codecs::png::{CompressionType, FilterType, PngEncoder};
use image::{ExtendedColorType, ImageEncoder};

use super::blob::{BlobSource, Side};
use super::pixelmatch;
use super::regions::{self, Region};
use super::store::{Change, Dim, FileEntry};
use super::{is_svg_path, probe_dimensions, ImageFile};
use crate::error::{AppError, AppResult};

pub const PREVIEW_WIDTH: u32 = 1000;
/// Bigger images are skipped: three decoded copies would take close to 1 GB.
pub const MAX_PIXELS: u64 = 80_000_000;
/// SVGs are rasterized at their own size, scaled down to this.
const MAX_SVG_PIXELS: f64 = 40_000_000.0;
/// Context around a region in its crop.
const CROP_PADDING: u32 = 64;
/// Crops bigger than this are scaled down, so a model reading them doesn't lose the detail.
const CROP_MAX_WIDTH: u32 = 1600;
const CROP_MAX_HEIGHT: u32 = 2400;

pub struct Raster {
    pub w: u32,
    pub h: u32,
    pub rgba: Vec<u8>,
}

/// An image read from a side: the pixels, and the original bytes when they already are a PNG.
struct Loaded {
    raster: Raster,
    png: Option<Vec<u8>>,
}

fn io(e: std::io::Error) -> String {
    e.to_string()
}

fn write_png(path: &Path, rgba: &[u8], w: u32, h: u32) -> Result<(), String> {
    let out = BufWriter::new(File::create(path).map_err(io)?);
    PngEncoder::new_with_quality(out, CompressionType::Fast, FilterType::Sub)
        .write_image(rgba, w, h, ExtendedColorType::Rgba8)
        .map_err(|e| e.to_string())
}

fn resize(rgba: &[u8], w: u32, h: u32, nw: u32, nh: u32) -> Vec<u8> {
    let view = image::ImageBuffer::<image::Rgba<u8>, &[u8]>::from_raw(w, h, rgba).expect("rgba matches its size");
    image::imageops::resize(&view, nw.max(1), nh.max(1), image::imageops::FilterType::Triangle).into_raw()
}

/// A `cw`×`ch` window of an image, white where it falls outside.
fn crop(rgba: &[u8], w: u32, h: u32, x: u32, y: u32, cw: u32, ch: u32) -> Vec<u8> {
    let mut out = vec![255u8; cw as usize * ch as usize * 4];
    for row in 0..ch {
        let sy = y + row;
        if sy >= h || x >= w {
            continue;
        }
        let n = cw.min(w - x) as usize * 4;
        let src = (sy as usize * w as usize + x as usize) * 4;
        let dst = row as usize * cw as usize * 4;
        out[dst..dst + n].copy_from_slice(&rgba[src..src + n]);
    }
    out
}

/// `rgba` on a white `nw`×`nh` canvas (top-left aligned), as the skill does for different sizes.
fn pad(r: Raster, nw: u32, nh: u32) -> Vec<u8> {
    if r.w == nw && r.h == nh {
        return r.rgba;
    }
    crop(&r.rgba, r.w, r.h, 0, 0, nw, nh)
}

fn demultiply(premultiplied: &mut [u8]) {
    for px in premultiplied.chunks_exact_mut(4) {
        let a = px[3] as u32;
        if a != 0 && a != 255 {
            for c in &mut px[..3] {
                *c = ((*c as u32 * 255 + a / 2) / a).min(255) as u8;
            }
        }
    }
}

fn rasterize_svg(bytes: &[u8], opt: &resvg::usvg::Options) -> Result<Raster, String> {
    let tree = resvg::usvg::Tree::from_data(bytes, opt).map_err(|e| format!("invalid SVG: {e}"))?;
    let size = tree.size();
    let (w, h) = (size.width() as f64, size.height() as f64);
    let scale = if w * h > MAX_SVG_PIXELS { (MAX_SVG_PIXELS / (w * h)).sqrt() } else { 1.0 };
    let (pw, ph) = (((w * scale).ceil() as u32).max(1), ((h * scale).ceil() as u32).max(1));
    let mut pixmap = resvg::tiny_skia::Pixmap::new(pw, ph).ok_or("SVG has no drawable size")?;
    resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(scale as f32, scale as f32), &mut pixmap.as_mut());
    let mut rgba = pixmap.take();
    demultiply(&mut rgba);
    Ok(Raster { w: pw, h: ph, rgba })
}

fn decode(bytes: Vec<u8>, path: &str, svg: &mut Option<resvg::usvg::Options<'static>>) -> Result<Loaded, String> {
    if let Some((w, h)) = probe_dimensions(&bytes, path) {
        if w as u64 * h as u64 > MAX_PIXELS && !is_svg_path(path) {
            return Err(format!("too large to compare ({} MP, limit {} MP)", w as u64 * h as u64 / 1_000_000, MAX_PIXELS / 1_000_000));
        }
    }
    if is_svg_path(path) {
        let opt = svg.get_or_insert_with(|| {
            let mut opt = resvg::usvg::Options::default();
            opt.fontdb_mut().load_system_fonts();
            opt
        });
        return Ok(Loaded { raster: rasterize_svg(&bytes, opt)?, png: None });
    }
    let is_png = bytes.starts_with(&[0x89, b'P', b'N', b'G']);
    let img = image::ImageReader::new(Cursor::new(&bytes))
        .with_guessed_format()
        .map_err(io)?
        .decode()
        .map_err(|e| format!("cannot decode: {e}"))?
        .into_rgba8();
    let (w, h) = img.dimensions();
    Ok(Loaded { raster: Raster { w, h, rgba: img.into_raw() }, png: is_png.then_some(bytes) })
}

fn load(
    blobs: &dyn BlobSource,
    side: Side,
    path: &str,
    svg: &mut Option<resvg::usvg::Options<'static>>,
) -> Result<Loaded, String> {
    let which = if side == Side::Left { "before" } else { "after" };
    let bytes = blobs.read(side, path).map_err(|e| format!("cannot read the {which} image: {e}"))?;
    decode(bytes, path, svg).map_err(|e| format!("{which}: {e}"))
}

/// Full-size copy and (when wide) the preview of one side.
fn write_side(fdir: &Path, name: &str, loaded: &Loaded) -> Result<Dim, String> {
    let r = &loaded.raster;
    let full = fdir.join(format!("{name}.png"));
    match &loaded.png {
        Some(bytes) => std::fs::write(&full, bytes).map_err(io)?,
        None => write_png(&full, &r.rgba, r.w, r.h)?,
    }
    write_preview(fdir, name, &r.rgba, r.w, r.h)?;
    Ok(Dim { width: r.w, height: r.h })
}

fn write_preview(fdir: &Path, name: &str, rgba: &[u8], w: u32, h: u32) -> Result<(), String> {
    if w <= PREVIEW_WIDTH {
        return Ok(());
    }
    let nh = (h as u64 * PREVIEW_WIDTH as u64 / w as u64).max(1) as u32;
    write_png(&fdir.join(format!("preview-{name}.png")), &resize(rgba, w, h, PREVIEW_WIDTH, nh), PREVIEW_WIDTH, nh)
}

fn write_crop(fdir: &Path, region: &Region, name: &str, rgba: &[u8], w: u32, h: u32) -> Result<(), String> {
    let x = region.x.saturating_sub(CROP_PADDING);
    let y = region.y.saturating_sub(CROP_PADDING);
    let x1 = (region.x + region.w + CROP_PADDING).min(w);
    let y1 = (region.y + region.h + CROP_PADDING).min(h);
    let (cw, ch) = (x1 - x, y1 - y);
    let window = crop(rgba, w, h, x, y, cw, ch);
    let scale = (CROP_MAX_WIDTH as f64 / cw as f64).min(CROP_MAX_HEIGHT as f64 / ch as f64).min(1.0);
    let path = fdir.join(format!("crop-{}-{name}.png", region.id));
    if scale < 1.0 {
        let (nw, nh) = (((cw as f64 * scale) as u32).max(1), ((ch as f64 * scale) as u32).max(1));
        write_png(&path, &resize(&window, cw, ch, nw, nh), nw, nh)
    } else {
        write_png(&path, &window, cw, ch)
    }
}

fn process(
    index: usize,
    f: &ImageFile,
    blobs: &dyn BlobSource,
    dir: &Path,
    svg: &mut Option<resvg::usvg::Options<'static>>,
) -> FileEntry {
    let mut e = FileEntry {
        index,
        path: f.path.clone(),
        old_path: f.old_path.clone(),
        change: f.change,
        before: None,
        after: None,
        mismatch_px: 0,
        aa_px: 0,
        diff_pct: 0.0,
        regions: vec![],
        other_regions: 0,
        skipped: None,
    };
    let fdir = dir.join(format!("f{index}"));
    let mut run = |e: &mut FileEntry| -> Result<(), String> {
        std::fs::create_dir_all(&fdir).map_err(io)?;
        let before = (f.change != Change::Added).then(|| load(blobs, Side::Left, f.left_path(), svg)).transpose()?;
        let after = (f.change != Change::Deleted).then(|| load(blobs, Side::Right, &f.path, svg)).transpose()?;
        if let Some(b) = &before {
            e.before = Some(write_side(&fdir, "before", b)?);
        }
        if let Some(a) = &after {
            e.after = Some(write_side(&fdir, "after", a)?);
        }
        let (Some(before), Some(after)) = (before, after) else { return Ok(()) };
        let (w, h) = (before.raster.w.max(after.raster.w), before.raster.h.max(after.raster.h));
        let (a, b) = (pad(before.raster, w, h), pad(after.raster, w, h));
        let cmp = pixelmatch::compare(&a, &b, w as usize, h as usize);
        let found = regions::detect(&cmp.mask, w as usize, h as usize);
        e.mismatch_px = cmp.changed;
        e.aa_px = cmp.anti_aliased;
        e.diff_pct = cmp.changed as f64 / (w as f64 * h as f64) * 100.0;
        write_png(&fdir.join("diff.png"), &cmp.diff, w, h)?;
        write_preview(&fdir, "diff", &cmp.diff, w, h)?;
        for r in &found.regions {
            write_crop(&fdir, r, "before", &a, w, h)?;
            write_crop(&fdir, r, "after", &b, w, h)?;
            write_crop(&fdir, r, "diff", &cmp.diff, w, h)?;
        }
        e.regions = found.regions;
        e.other_regions = found.other;
        Ok(())
    };
    if let Err(msg) = run(&mut e) {
        log::warn!(target: "image_diff", "{}: {msg}", f.path);
        e.skipped = Some(msg);
    }
    e
}

/// Diff every file into `dir` (a fresh directory), reporting `(done, total)` after each. A file
/// that can't be read or decoded is recorded as skipped; only cancelling fails the whole run.
pub fn generate(
    blobs: &dyn BlobSource,
    files: &[ImageFile],
    dir: &Path,
    cancel: &AtomicBool,
    mut on_progress: impl FnMut(usize, usize),
) -> AppResult<Vec<FileEntry>> {
    std::fs::create_dir_all(dir)?;
    let mut svg = None;
    let mut entries = Vec::with_capacity(files.len());
    for (i, f) in files.iter().enumerate() {
        if cancel.load(Ordering::Relaxed) {
            return Err(AppError::Other("cancelled".into()));
        }
        entries.push(process(i, f, blobs, dir, &mut svg));
        on_progress(i + 1, files.len());
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    #[derive(Default)]
    struct Fake(HashMap<(bool, String), Vec<u8>>);

    impl Fake {
        fn put(&mut self, side: Side, path: &str, bytes: Vec<u8>) {
            self.0.insert((side == Side::Left, path.to_string()), bytes);
        }
    }

    impl BlobSource for Fake {
        fn read(&self, side: Side, path: &str) -> AppResult<Vec<u8>> {
            self.0
                .get(&(side == Side::Left, path.to_string()))
                .cloned()
                .ok_or_else(|| AppError::Other("missing".into()))
        }
    }

    /// A white PNG with black squares at the given (x, y, size).
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
        image::DynamicImage::ImageRgba8(img).write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
        out
    }

    fn file(path: &str, change: Change) -> ImageFile {
        ImageFile { path: path.into(), old_path: None, change }
    }

    fn tmp(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("cr-imgjob-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn a_modified_image_gets_counts_regions_and_files() {
        let mut blobs = Fake::default();
        blobs.put(Side::Left, "a.png", png(80, 60, &[]));
        blobs.put(Side::Right, "a.png", png(80, 60, &[(10, 10, 5)]));
        let dir = tmp("mod");
        let mut seen = vec![];
        let out = generate(&blobs, &[file("a.png", Change::Modified)], &dir, &AtomicBool::new(false), |d, t| seen.push((d, t))).unwrap();
        let e = &out[0];
        assert_eq!(e.skipped, None);
        assert_eq!(e.mismatch_px, 25);
        assert_eq!(e.regions.len(), 1);
        assert_eq!((e.regions[0].x, e.regions[0].y, e.regions[0].w, e.regions[0].h), (10, 10, 5, 5));
        assert_eq!(e.before, Some(Dim { width: 80, height: 60 }));
        assert!((e.diff_pct - 25.0 / 4800.0 * 100.0).abs() < 1e-9);
        for f in ["before", "after", "diff"] {
            assert!(dir.join(format!("f0/{f}.png")).exists(), "{f}.png");
            assert!(dir.join(format!("f0/crop-0-{f}.png")).exists(), "crop {f}");
            assert!(!dir.join(format!("f0/preview-{f}.png")).exists(), "narrow images need no preview");
        }
        assert_eq!(seen, vec![(1, 1)]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn wide_images_get_previews_and_unchanged_ones_no_region() {
        let mut blobs = Fake::default();
        blobs.put(Side::Left, "w.png", png(1200, 40, &[]));
        blobs.put(Side::Right, "w.png", png(1200, 40, &[]));
        let dir = tmp("wide");
        let out = generate(&blobs, &[file("w.png", Change::Modified)], &dir, &AtomicBool::new(false), |_, _| {}).unwrap();
        assert_eq!((out[0].mismatch_px, out[0].regions.len()), (0, 0));
        for f in ["before", "after", "diff"] {
            let (w, _) = image::image_dimensions(dir.join(format!("f0/preview-{f}.png"))).unwrap();
            assert_eq!(w, PREVIEW_WIDTH);
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn different_sizes_are_padded_with_white() {
        let mut blobs = Fake::default();
        blobs.put(Side::Left, "s.png", png(20, 20, &[]));
        blobs.put(Side::Right, "s.png", png(20, 30, &[(0, 22, 4)]));
        let dir = tmp("size");
        let out = generate(&blobs, &[file("s.png", Change::Modified)], &dir, &AtomicBool::new(false), |_, _| {}).unwrap();
        assert_eq!(out[0].mismatch_px, 16, "only the black square differs from the white padding");
        assert_eq!(image::image_dimensions(dir.join("f0/diff.png")).unwrap(), (20, 30));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn added_and_deleted_files_keep_one_side() {
        let mut blobs = Fake::default();
        blobs.put(Side::Right, "new.png", png(10, 10, &[]));
        blobs.put(Side::Left, "old.png", png(12, 8, &[]));
        let dir = tmp("addel");
        let out = generate(
            &blobs,
            &[file("new.png", Change::Added), file("old.png", Change::Deleted)],
            &dir,
            &AtomicBool::new(false),
            |_, _| {},
        )
        .unwrap();
        assert_eq!((out[0].before, out[0].after), (None, Some(Dim { width: 10, height: 10 })));
        assert_eq!((out[1].before, out[1].after), (Some(Dim { width: 12, height: 8 }), None));
        assert!(dir.join("f0/after.png").exists() && !dir.join("f0/diff.png").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn unreadable_or_undecodable_files_are_skipped_not_fatal() {
        let mut blobs = Fake::default();
        blobs.put(Side::Left, "bad.png", b"not a png".to_vec());
        blobs.put(Side::Right, "bad.png", png(4, 4, &[]));
        blobs.put(Side::Left, "ok.png", png(4, 4, &[]));
        blobs.put(Side::Right, "ok.png", png(4, 4, &[(0, 0, 2)]));
        let dir = tmp("skip");
        let out = generate(
            &blobs,
            &[file("bad.png", Change::Modified), file("gone.png", Change::Modified), file("ok.png", Change::Modified)],
            &dir,
            &AtomicBool::new(false),
            |_, _| {},
        )
        .unwrap();
        assert!(out[0].skipped.as_deref().unwrap().starts_with("before: cannot decode"));
        assert!(out[1].skipped.as_deref().unwrap().contains("cannot read the before image"));
        assert_eq!((out[2].skipped.as_deref(), out[2].mismatch_px), (None, 4));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn cancelling_stops_the_run() {
        let blobs = Fake::default();
        let dir = tmp("cancel");
        let err = generate(&blobs, &[file("a.png", Change::Added)], &dir, &AtomicBool::new(true), |_, _| {}).unwrap_err();
        assert!(err.to_string().contains("cancelled"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn svgs_are_rasterized_and_compared() {
        let svg = |fill: &str| format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="10"><rect width="10" height="10" fill="{fill}"/></svg>"#).into_bytes();
        let mut blobs = Fake::default();
        blobs.put(Side::Left, "l.svg", svg("red"));
        blobs.put(Side::Right, "l.svg", svg("blue"));
        let dir = tmp("svg");
        let out = generate(&blobs, &[file("l.svg", Change::Modified)], &dir, &AtomicBool::new(false), |_, _| {}).unwrap();
        assert_eq!(out[0].skipped, None);
        assert_eq!(out[0].before, Some(Dim { width: 20, height: 10 }));
        assert_eq!(out[0].mismatch_px, 100, "the 10x10 square changed colour");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn crops_are_cut_with_context_and_clamped() {
        let rgba = vec![7u8; 10 * 10 * 4];
        let c = crop(&rgba, 10, 10, 8, 8, 4, 4);
        assert_eq!(c.len(), 4 * 4 * 4);
        assert_eq!(&c[..4], &[7, 7, 7, 7], "inside");
        assert_eq!(&c[(3 * 4 + 3) * 4..(3 * 4 + 3) * 4 + 4], &[255, 255, 255, 255], "outside is white");
    }
}
