//! A review's image diffs as one self-contained HTML page: the AI report (when there is one), then
//! every changed image as before / after / diff with its regions. The PNGs are inlined as base64
//! data URIs, previews for pictures wider than the preview width, so the file stands alone.

use std::fmt::Write as _;
use std::path::Path;

use base64::Engine as _;

use super::report::Report;
use super::store::{Change, Dim, FileEntry, Manifest};

const SLOTS: [&str; 3] = ["before", "after", "diff"];

fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// `<img>` of a cached PNG, inlined; a placeholder when the file is gone.
fn img(dir: &Path, rel: &str, alt: &str, class: &str) -> String {
    match std::fs::read(dir.join(rel)) {
        Ok(bytes) => format!(
            r#"<img class="{class}" alt="{}" src="data:image/png;base64,{}">"#,
            esc(alt),
            base64::engine::general_purpose::STANDARD.encode(bytes)
        ),
        Err(_) => format!(r#"<div class="missing">{} (missing)</div>"#, esc(alt)),
    }
}

fn diff_dim(f: &FileEntry) -> Option<Dim> {
    let (b, a) = (f.before?, f.after?);
    Some(Dim { width: b.width.max(a.width), height: b.height.max(a.height) })
}

/// The same rule as the panel's `imageAssetRel`: the preview when the picture is wider than it.
fn asset_rel(f: &FileEntry, slot: &str, preview_width: u32) -> String {
    let dim = match slot {
        "diff" => diff_dim(f),
        "before" => f.before,
        _ => f.after,
    };
    let wide = dim.is_some_and(|d| d.width > preview_width);
    format!("f{}/{}{slot}.png", f.index, if wide { "preview-" } else { "" })
}

fn change_label(c: Change) -> &'static str {
    match c {
        Change::Added => "NEW",
        Change::Deleted => "DEL",
        Change::Modified => "modified",
        Change::Renamed => "renamed",
    }
}

/// The same as the panel's `fileBadge`.
fn badge(f: &FileEntry) -> String {
    if f.skipped.is_some() {
        return "skipped".into();
    }
    match f.change {
        Change::Added | Change::Deleted => change_label(f.change).into(),
        _ if f.mismatch_px == 0 => "same".into(),
        _ if f.diff_pct < 0.1 => format!("{:.3}%", f.diff_pct),
        _ => format!("{:.1}%", f.diff_pct),
    }
}

fn short(sha: Option<&str>) -> &str {
    sha.map(|s| &s[..s.len().min(8)]).unwrap_or("?")
}

fn size(d: Option<Dim>) -> String {
    d.map(|d| format!("{}×{}", d.width, d.height)).unwrap_or_else(|| "—".into())
}

fn display_path(f: &FileEntry) -> String {
    match &f.old_path {
        Some(old) => format!("{} → {}", esc(old), esc(&f.path)),
        None => esc(&f.path),
    }
}

fn crops(out: &mut String, dir: &Path, rel: impl Fn(&str) -> String, label: &str) {
    out.push_str(r#"<div class="crops">"#);
    for slot in SLOTS {
        let _ = write!(out, "<figure>{}<figcaption>{slot}</figcaption></figure>", img(dir, &rel(slot), &format!("{slot} {label}"), "crop"));
    }
    out.push_str("</div>");
}

fn report_section(out: &mut String, dir: &Path, report: &Report) {
    out.push_str(r#"<section id="report"><h2>AI report</h2>"#);
    if !report.summary.is_empty() {
        let _ = write!(out, r#"<p class="summary">{}</p>"#, esc(&report.summary));
    }
    if report.findings.is_empty() {
        out.push_str(r#"<p class="muted">The report lists no changes.</p>"#);
    }
    for f in &report.findings {
        let _ = write!(
            out,
            r##"<article id="finding-{n}" class="finding{noise}"><header><strong>{title}</strong><span class="chip">{kind}</span><a href="#file-{fi}">{file}</a></header>"##,
            n = f.n,
            noise = if f.kind == "noise" { " noise" } else { "" },
            title = esc(&f.title),
            kind = esc(&f.kind),
            fi = f.file_index,
            file = esc(&f.file),
        );
        if !f.description.is_empty() {
            let _ = write!(out, "<p>{}</p>", esc(&f.description));
        }
        crops(out, dir, |slot| format!("report/{}-{slot}.png", f.n), &f.title);
        out.push_str("</article>");
    }
    if report.dropped > 0 {
        let _ = write!(out, r#"<p class="muted">{} finding(s) left out: they named no compared image.</p>"#, report.dropped);
    }
    out.push_str("</section>");
}

fn file_section(out: &mut String, dir: &Path, f: &FileEntry, preview_width: u32) {
    let _ = write!(
        out,
        r#"<section class="file" id="file-{}"><header><span class="path">{}</span><span class="chip">{}</span>"#,
        f.index,
        display_path(f),
        change_label(f.change)
    );
    let dim = diff_dim(f);
    if f.skipped.is_none() && dim.is_some() {
        let _ = write!(
            out,
            r#"<span class="muted">{} → {} · {} px differ ({:.3}%){}</span>"#,
            size(f.before),
            size(f.after),
            f.mismatch_px,
            f.diff_pct,
            if f.aa_px > 0 { format!(" · {} px anti-aliasing", f.aa_px) } else { String::new() }
        );
    }
    out.push_str("</header>");
    if let Some(why) = &f.skipped {
        let _ = write!(out, r#"<p class="muted">Not compared: {}</p></section>"#, esc(why));
        return;
    }
    let slots: &[&str] = match f.change {
        Change::Added => &["after"],
        Change::Deleted => &["before"],
        _ => &SLOTS,
    };
    let _ = write!(out, r#"<div class="cols" style="grid-template-columns: repeat({}, minmax(0, 1fr))">"#, slots.len());
    for slot in slots {
        let _ = write!(out, r#"<div class="col"><h4>{slot}</h4><div class="frame">"#);
        out.push_str(&img(dir, &asset_rel(f, slot, preview_width), &format!("{slot} {}", f.path), "full"));
        if *slot == "diff" {
            if let Some(d) = dim {
                for r in &f.regions {
                    let pct = |v: u32, total: u32| v as f64 / total.max(1) as f64 * 100.0;
                    let _ = write!(
                        out,
                        r##"<span class="box" title="#{} · {}×{}" style="left:{:.3}%;top:{:.3}%;width:{:.3}%;height:{:.3}%"></span>"##,
                        r.id,
                        r.w,
                        r.h,
                        pct(r.x, d.width),
                        pct(r.y, d.height),
                        pct(r.w, d.width),
                        pct(r.h, d.height)
                    );
                }
            }
        }
        out.push_str("</div></div>");
    }
    out.push_str("</div>");
    if !f.regions.is_empty() {
        let other = if f.other_regions > 0 { format!(" + {} smaller", f.other_regions) } else { String::new() };
        let _ = write!(out, "<details><summary>Regions ({}{other}) · before / after / diff crops</summary>", f.regions.len());
        for r in &f.regions {
            let _ = write!(
                out,
                r#"<div class="region"><div class="muted">#{} · {}×{} at {},{} · {} px</div>"#,
                r.id, r.w, r.h, r.x, r.y, r.changed_px
            );
            crops(out, dir, |slot| format!("f{}/crop-{}-{slot}.png", f.index, r.id), &format!("#{}", r.id));
            out.push_str("</div>");
        }
        out.push_str("</details>");
    }
    out.push_str("</section>");
}

const STYLE: &str = r#"
body { font: 14px/1.45 -apple-system, BlinkMacSystemFont, "Segoe UI", Helvetica, Arial, sans-serif; color: #1f2328; background: #fff; margin: 0 auto; padding: 24px; max-width: 1600px; }
h1 { font-size: 22px; margin: 0 0 4px; } h2 { font-size: 18px; border-bottom: 1px solid #d0d7de; padding-bottom: 4px; margin-top: 32px; }
h4 { margin: 0 0 4px; font-size: 12px; text-transform: uppercase; color: #59636e; }
a { color: #0969da; text-decoration: none; } a:hover { text-decoration: underline; }
.muted { color: #59636e; } .missing { color: #cf222e; padding: 8px; border: 1px dashed #cf222e; }
.chip { display: inline-block; font-size: 11px; padding: 0 6px; border: 1px solid #d0d7de; border-radius: 10px; color: #59636e; }
nav ul { columns: 2; padding-left: 18px; } nav li { break-inside: avoid; }
.badge { font-size: 11px; color: #59636e; margin-left: 6px; } .badge.changed { color: #cf222e; }
.legend { display: flex; gap: 16px; margin: 8px 0; font-size: 12px; } .swatch { display: inline-block; width: 10px; height: 10px; margin-right: 4px; vertical-align: middle; }
.summary { font-size: 15px; }
.finding, .file { border: 1px solid #d0d7de; border-radius: 6px; padding: 12px; margin: 12px 0; }
.finding.noise { opacity: 0.55; }
.finding header, .file header { display: flex; flex-wrap: wrap; gap: 8px; align-items: baseline; margin-bottom: 8px; }
.path { font-family: ui-monospace, SFMono-Regular, Menlo, monospace; font-weight: 600; word-break: break-all; }
.cols { display: grid; gap: 12px; }
.frame { position: relative; line-height: 0; border: 1px solid #d0d7de; background: repeating-conic-gradient(#eee 0 25%, #fff 0 50%) 0 0 / 16px 16px; }
img.full { width: 100%; height: auto; display: block; }
.box { position: absolute; border: 2px solid #0969da; box-sizing: border-box; }
.crops { display: flex; flex-wrap: wrap; gap: 12px; } figure { margin: 0; } figcaption { font-size: 12px; color: #59636e; }
img.crop { max-width: 420px; max-height: 320px; border: 1px solid #d0d7de; display: block; }
details { margin-top: 10px; } summary { cursor: pointer; color: #59636e; } .region { margin: 10px 0; }
"#;

/// The page for a review's generated image diffs read from `dir`.
pub fn render(dir: &Path, manifest: &Manifest, report: Option<&Report>) -> String {
    let mut out = String::new();
    let _ = write!(
        out,
        r#"<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><title>Image diffs · review {id}</title><style>{STYLE}</style></head><body>"#,
        id = manifest.review_id
    );
    let _ = write!(
        out,
        r#"<h1>Image diffs</h1><p class="muted">Review {} · {}…{} · generated {} · {} image(s)</p>"#,
        manifest.review_id,
        esc(short(manifest.base_sha.as_deref())),
        esc(short(manifest.head_sha.as_deref())),
        esc(&manifest.generated_at),
        manifest.files.len()
    );
    out.push_str("<nav><ul>");
    if let Some(r) = report {
        let _ = write!(out, "<li><a href=\"#report\">AI report</a> ({} finding(s))</li>", r.findings.len());
    }
    for f in &manifest.files {
        let changed = if f.mismatch_px > 0 { " changed" } else { "" };
        let _ = write!(
            out,
            r##"<li><a href="#file-{}">{}</a><span class="badge{changed}">{}</span></li>"##,
            f.index,
            display_path(f),
            esc(&badge(f))
        );
    }
    out.push_str("</ul></nav>");
    if let Some(r) = report {
        report_section(&mut out, dir, r);
    }
    out.push_str(r#"<h2>Images</h2><div class="legend muted"><span><i class="swatch" style="background:#ff0000"></i>changed</span><span><i class="swatch" style="background:#ffff00"></i>anti-aliasing</span><span><i class="swatch" style="background:rgba(128,128,128,0.4)"></i>unchanged (faded)</span></div>"#);
    for f in &manifest.files {
        file_section(&mut out, dir, f, manifest.preview_width);
    }
    out.push_str("</body></html>\n");
    out
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicBool;

    use super::*;
    use crate::error::{AppError, AppResult};
    use crate::image_diff::blob::{BlobSource, Side};
    use crate::image_diff::job::generate;
    use crate::image_diff::report::Finding;
    use crate::image_diff::store::MANIFEST_VERSION;
    use crate::image_diff::ImageFile;

    struct Fake(Vec<(Side, Vec<u8>)>);

    impl BlobSource for Fake {
        fn read(&self, side: Side, _: &str) -> AppResult<Vec<u8>> {
            self.0.iter().find(|(s, _)| *s == side).map(|(_, b)| b.clone()).ok_or_else(|| AppError::Other("missing".into()))
        }
    }

    fn png(w: u32, h: u32, square: bool) -> Vec<u8> {
        let mut img = image::RgbaImage::from_pixel(w, h, image::Rgba([255, 255, 255, 255]));
        if square {
            for y in 10..16 {
                for x in 10..16 {
                    img.put_pixel(x, y, image::Rgba([0, 0, 0, 255]));
                }
            }
        }
        let mut out = Vec::new();
        image::DynamicImage::ImageRgba8(img).write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
        out
    }

    fn cache(name: &str, w: u32) -> (std::path::PathBuf, Manifest) {
        let dir = std::env::temp_dir().join(format!("cr-html-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let blobs = Fake(vec![(Side::Left, png(w, 100, false)), (Side::Right, png(w, 100, true))]);
        let file = ImageFile { path: "e2e/a.png".into(), old_path: None, change: Change::Modified };
        let files = generate(&blobs, &[file], &dir, &AtomicBool::new(false), |_, _| {}).unwrap();
        let manifest = Manifest {
            version: MANIFEST_VERSION,
            review_id: 7,
            pin: "p".into(),
            base_sha: Some("0123456789abcdef".into()),
            head_sha: None,
            generated_at: "t".into(),
            preview_width: 1000,
            auto: false,
            files,
        };
        (dir, manifest)
    }

    #[test]
    fn the_page_inlines_every_picture_and_escapes_the_ai_text() {
        let (dir, manifest) = cache("report", 300);
        std::fs::create_dir_all(dir.join("report")).unwrap();
        for slot in SLOTS {
            std::fs::copy(dir.join(format!("f0/crop-0-{slot}.png")), dir.join(format!("report/0-{slot}.png"))).unwrap();
        }
        let report = Report {
            summary: "One <b>square</b>".into(),
            generated_at: "t".into(),
            findings: vec![Finding {
                n: 0,
                file: "e2e/a.png".into(),
                file_index: 0,
                bbox: [10, 10, 6, 6],
                region_ids: vec![0],
                title: "<script>alert(1)</script>".into(),
                description: "A square & more".into(),
                kind: "content".into(),
            }],
            dropped: 2,
        };
        let html = render(&dir, &manifest, Some(&report));
        assert!(html.contains("&lt;script&gt;alert(1)&lt;/script&gt;") && !html.contains("<script>"));
        assert!(html.contains("One &lt;b&gt;square&lt;/b&gt;") && html.contains("A square &amp; more"));
        assert!(html.contains(r#"id="finding-0""#) && html.contains(r#"id="file-0""#));
        assert!(html.contains("2 finding(s) left out"));
        assert!(html.contains("01234567…?"), "short shas");
        assert!(!html.contains("crimg://") && !html.contains("(missing)"));
        // 3 full pictures + 3 region crops + 3 finding crops.
        assert_eq!(html.matches("data:image/png;base64,").count(), 9);
        assert_eq!(html.matches(r#"class="box""#).count(), 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn without_a_report_only_the_images_and_wide_ones_as_previews() {
        let (dir, manifest) = cache("wide", 1200);
        assert_eq!(asset_rel(&manifest.files[0], "diff", 1000), "f0/preview-diff.png");
        let html = render(&dir, &manifest, None);
        assert!(!html.contains(r#"id="report""#));
        assert_eq!(html.matches("data:image/png;base64,").count(), 6);
        assert!(!html.contains("(missing)"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn badge_matches_the_panel() {
        let (dir, manifest) = cache("badge", 300);
        let mut f = manifest.files[0].clone();
        f.diff_pct = 0.0123;
        assert_eq!(badge(&f), "0.012%");
        f.diff_pct = 0.51;
        assert_eq!(badge(&f), "0.5%");
        f.change = Change::Added;
        assert_eq!(badge(&f), "NEW");
        f.skipped = Some("too big".into());
        assert_eq!(badge(&f), "skipped");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
