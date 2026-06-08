//! Pure re-anchoring primitives: parse one file's unified diff and remap a
//! RIGHT-side (new) line from the OLD revision to the NEW revision.

#[derive(Debug, PartialEq, Eq)]
pub enum Remap {
    Shifted(i64),
    Lost,
}

#[derive(Debug)]
struct Hunk {
    old_start: i64,
    old_len: i64,
    new_start: i64,
    new_len: i64,
    lines: Vec<u8>,
}

#[derive(Debug, Default)]
pub struct FileHunks {
    hunks: Vec<Hunk>,
}

pub fn parse_file_patch(patch: &str) -> FileHunks {
    let mut hunks: Vec<Hunk> = Vec::new();
    let mut current: Option<Hunk> = None;

    for line in patch.lines() {
        if line.starts_with("@@") {
            if let Some(h) = current.take() {
                hunks.push(h);
            }
            if let Some(h) = parse_hunk_header(line) {
                current = Some(h);
            }
            continue;
        }

        let Some(hunk) = current.as_mut() else {
            continue;
        };

        match line.as_bytes().first() {
            Some(b' ') => hunk.lines.push(b' '),
            Some(b'-') => hunk.lines.push(b'-'),
            Some(b'+') => hunk.lines.push(b'+'),
            // Anything else ends the body: file headers, "\ No newline", etc.
            _ => {
                hunks.push(current.take().unwrap());
            }
        }
    }

    if let Some(h) = current.take() {
        hunks.push(h);
    }

    FileHunks { hunks }
}

fn parse_hunk_header(line: &str) -> Option<Hunk> {
    let rest = line.strip_prefix("@@")?;
    let close = rest.find("@@")?;
    let spec = rest[..close].trim();

    let mut parts = spec.split_whitespace();
    let old = parts.next()?.strip_prefix('-')?;
    let new = parts.next()?.strip_prefix('+')?;

    let (old_start, old_len) = parse_range(old)?;
    let (new_start, new_len) = parse_range(new)?;

    Some(Hunk {
        old_start,
        old_len,
        new_start,
        new_len,
        lines: Vec::new(),
    })
}

fn parse_range(range: &str) -> Option<(i64, i64)> {
    match range.split_once(',') {
        Some((start, len)) => Some((start.parse().ok()?, len.parse().ok()?)),
        None => Some((range.parse().ok()?, 1)),
    }
}

pub fn remap_right_line(line: i64, hunks: &FileHunks) -> Remap {
    let mut delta: i64 = 0;

    for hunk in &hunks.hunks {
        if line < hunk.old_start {
            return Remap::Shifted(line + delta);
        }

        let old_end = hunk.old_start + hunk.old_len - 1;
        if hunk.old_len > 0 && line <= old_end {
            let mut old_ln = hunk.old_start;
            let mut new_ln = hunk.new_start;
            for &kind in &hunk.lines {
                match kind {
                    b' ' => {
                        if old_ln == line {
                            return Remap::Shifted(new_ln);
                        }
                        old_ln += 1;
                        new_ln += 1;
                    }
                    b'-' => {
                        if old_ln == line {
                            return Remap::Lost;
                        }
                        old_ln += 1;
                    }
                    b'+' => new_ln += 1,
                    _ => {}
                }
            }
            return Remap::Lost;
        }

        delta += hunk.new_len - hunk.old_len;
    }

    Remap::Shifted(line + delta)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_empty_patch() {
        let hunks = parse_file_patch("");
        assert_eq!(remap_right_line(80, &hunks), Remap::Shifted(80));
    }

    #[test]
    fn insertion_above() {
        let patch = "@@ -50,0 +50,3 @@\n+a\n+b\n+c\n";
        let hunks = parse_file_patch(patch);
        assert_eq!(remap_right_line(80, &hunks), Remap::Shifted(83));
    }

    #[test]
    fn deletion_above() {
        let patch = "@@ -50,2 +50,0 @@\n-x\n-y\n";
        let hunks = parse_file_patch(patch);
        assert_eq!(remap_right_line(80, &hunks), Remap::Shifted(78));
    }

    #[test]
    fn inside_replaced_is_lost() {
        // Replace old line 50 with a new line: delete then add at the same spot.
        let patch = "@@ -50,1 +50,1 @@\n-old\n+new\n";
        let hunks = parse_file_patch(patch);
        assert_eq!(remap_right_line(50, &hunks), Remap::Lost);
    }

    #[test]
    fn context_inside_hunk() {
        // Hunk inserts two adds, then a context line at old line 50.
        // old 50 (context) -> new 52.
        let patch = "@@ -50,1 +50,3 @@\n+a\n+b\n ctx\n";
        let hunks = parse_file_patch(patch);
        assert_eq!(remap_right_line(50, &hunks), Remap::Shifted(52));
    }

    #[test]
    fn multi_hunk_delta() {
        // Two hunks, each adding one line, both before line 200.
        let patch = "@@ -10,1 +10,2 @@\n ctx\n+added\n@@ -100,1 +101,2 @@\n ctx\n+added\n";
        let hunks = parse_file_patch(patch);
        assert_eq!(remap_right_line(200, &hunks), Remap::Shifted(202));
    }

    #[test]
    fn past_all_hunks() {
        let patch = "@@ -5,1 +5,2 @@\n ctx\n+added\n";
        let hunks = parse_file_patch(patch);
        assert_eq!(remap_right_line(500, &hunks), Remap::Shifted(501));
    }

    #[test]
    fn multi_line_range_caller_treats_as_lost() {
        // start_line in an untouched gap, line inside a replacement: caller marks Lost.
        let patch = "@@ -100,1 +100,1 @@\n-old\n+new\n";
        let hunks = parse_file_patch(patch);
        let start = remap_right_line(50, &hunks);
        let end = remap_right_line(100, &hunks);
        assert_eq!(start, Remap::Shifted(50));
        assert_eq!(end, Remap::Lost);
        let lost = matches!(start, Remap::Lost) || matches!(end, Remap::Lost);
        assert!(lost);
    }

    #[test]
    fn parses_full_git_diff_headers() {
        let patch = "diff --git a/foo.rs b/foo.rs\n\
index abc123..def456 100644\n\
--- a/foo.rs\n\
+++ b/foo.rs\n\
@@ -50,0 +50,3 @@\n\
+a\n+b\n+c\n\
\\ No newline at end of file\n";
        let hunks = parse_file_patch(patch);
        assert_eq!(remap_right_line(80, &hunks), Remap::Shifted(83));
    }

    #[test]
    fn missing_hunk_length_defaults_to_one() {
        // No comma in either range -> length 1 each. Pure context line at old 50.
        let patch = "@@ -50 +60 @@\n ctx\n";
        let hunks = parse_file_patch(patch);
        assert_eq!(remap_right_line(50, &hunks), Remap::Shifted(60));
    }
}
