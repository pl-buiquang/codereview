//! Pure re-anchoring primitives: parse one file's unified diff and remap a line
//! valid in the patch's OLD revision onto the NEW revision. The same mapping
//! serves both comment sides: RIGHT comments map across a head→head patch, LEFT
//! comments across a base→base patch — in both cases the stored line lives on the
//! patch's old side (see `remap_line`).

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

/// Map `line` — valid in the patch's OLD revision — onto the NEW revision.
/// The input is matched against each hunk's `old_start`/`old_len`, so the caller
/// must always fetch the diff in the direction `old → new` (pin → current).
/// Used for RIGHT comments through a head→head patch and LEFT comments through a
/// base→base patch; "old"/"new" here are the patch's two sides, not a comment's
/// PR side. A line on an untouched gap shifts by the net delta of prior hunks; a
/// deleted/replaced line is `Lost`.
pub fn remap_line(line: i64, hunks: &FileHunks) -> Remap {
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

/// Line `line` would land on if every hunk fully above it applied: a cheap guess used to
/// break ties between equally good content matches.
pub fn predict_line(line: i64, hunks: &FileHunks) -> i64 {
    let mut delta = 0;
    for hunk in &hunks.hunks {
        if hunk.old_start + hunk.old_len.max(0) > line {
            break;
        }
        delta += hunk.new_len - hunk.old_len;
    }
    line + delta
}

/// Context lines compared on each side of the commented block.
const CONTEXT: usize = 3;

fn trivial(line: &str) -> bool {
    line.len() < 3 || !line.chars().any(char::is_alphanumeric)
}

/// Fallback for a line `remap_line` lost: find the commented block `old[start..=end]` (1-based,
/// inclusive) in `new` by content. Lines compare trimmed, so re-indentation and CRLF don't matter.
/// Returns the block's new first line, or `Lost` unless there is exactly one best match: the most
/// matching context lines wins, then the one nearest `predicted`. A block of only trivial lines
/// (blank, `}`) with no matching context is never relocated.
pub fn relocate(old: &[&str], new: &[&str], start: i64, end: i64, predicted: Option<i64>) -> Remap {
    if start < 1 || end < start || end as usize > old.len() {
        return Remap::Lost;
    }
    let (s, e) = (start as usize - 1, end as usize);
    let block: Vec<&str> = old[s..e].iter().map(|l| l.trim()).collect();
    if block.len() > new.len() {
        return Remap::Lost;
    }
    let matches = |a: Option<&&str>, b: Option<&&str>| matches!((a, b), (Some(a), Some(b)) if a.trim() == b.trim());

    let mut best: Vec<(usize, usize)> = Vec::new(); // (score, 0-based index in new)
    let mut best_score = 0;
    for i in 0..=(new.len() - block.len()) {
        if !block.iter().enumerate().all(|(k, l)| new[i + k].trim() == *l) {
            continue;
        }
        let mut score = 0;
        for k in 1..=CONTEXT {
            if s >= k && i >= k && matches(old.get(s - k), new.get(i - k)) {
                score += 1;
            }
            if matches(old.get(e + k - 1), new.get(i + block.len() + k - 1)) {
                score += 1;
            }
        }
        if best.is_empty() || score > best_score {
            best_score = score;
            best.clear();
        }
        if score == best_score {
            best.push((score, i));
        }
    }
    if best.is_empty() || (best_score == 0 && block.iter().all(|l| trivial(l))) {
        return Remap::Lost;
    }
    if best.len() > 1 {
        let Some(p) = predicted else { return Remap::Lost };
        let dist = |&(_, i): &(usize, usize)| (i as i64 + 1 - p).abs();
        best.sort_by_key(dist);
        if dist(&best[0]) == dist(&best[1]) {
            return Remap::Lost;
        }
    }
    Remap::Shifted(best[0].1 as i64 + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_empty_patch() {
        let hunks = parse_file_patch("");
        assert_eq!(remap_line(80, &hunks), Remap::Shifted(80));
    }

    #[test]
    fn insertion_above() {
        let patch = "@@ -50,0 +50,3 @@\n+a\n+b\n+c\n";
        let hunks = parse_file_patch(patch);
        assert_eq!(remap_line(80, &hunks), Remap::Shifted(83));
    }

    #[test]
    fn deletion_above() {
        let patch = "@@ -50,2 +50,0 @@\n-x\n-y\n";
        let hunks = parse_file_patch(patch);
        assert_eq!(remap_line(80, &hunks), Remap::Shifted(78));
    }

    #[test]
    fn inside_replaced_is_lost() {
        // Replace old line 50 with a new line: delete then add at the same spot.
        let patch = "@@ -50,1 +50,1 @@\n-old\n+new\n";
        let hunks = parse_file_patch(patch);
        assert_eq!(remap_line(50, &hunks), Remap::Lost);
    }

    #[test]
    fn context_inside_hunk() {
        // Hunk inserts two adds, then a context line at old line 50.
        // old 50 (context) -> new 52.
        let patch = "@@ -50,1 +50,3 @@\n+a\n+b\n ctx\n";
        let hunks = parse_file_patch(patch);
        assert_eq!(remap_line(50, &hunks), Remap::Shifted(52));
    }

    #[test]
    fn multi_hunk_delta() {
        // Two hunks, each adding one line, both before line 200.
        let patch = "@@ -10,1 +10,2 @@\n ctx\n+added\n@@ -100,1 +101,2 @@\n ctx\n+added\n";
        let hunks = parse_file_patch(patch);
        assert_eq!(remap_line(200, &hunks), Remap::Shifted(202));
    }

    #[test]
    fn past_all_hunks() {
        let patch = "@@ -5,1 +5,2 @@\n ctx\n+added\n";
        let hunks = parse_file_patch(patch);
        assert_eq!(remap_line(500, &hunks), Remap::Shifted(501));
    }

    #[test]
    fn multi_line_range_caller_treats_as_lost() {
        // start_line in an untouched gap, line inside a replacement: caller marks Lost.
        let patch = "@@ -100,1 +100,1 @@\n-old\n+new\n";
        let hunks = parse_file_patch(patch);
        let start = remap_line(50, &hunks);
        let end = remap_line(100, &hunks);
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
        assert_eq!(remap_line(80, &hunks), Remap::Shifted(83));
    }

    #[test]
    fn old_to_new_direction_worked_example() {
        // B1 -> B2 inserts 3 lines above old line 50; a LEFT comment at old line
        // 80 must shift to 83 (insertion above pushes it down).
        let forward = parse_file_patch("@@ -50,0 +50,3 @@\n+a\n+b\n+c\n");
        assert_eq!(remap_line(80, &forward), Remap::Shifted(83));

        // The trap: fetching the diff reversed (B2 -> B1) reads three deletions,
        // so line 80 would map to 77 and new lines 50-52 would falsely be Lost.
        // This documents why the caller must always diff pin -> current.
        let reversed = parse_file_patch("@@ -50,3 +50,0 @@\n-a\n-b\n-c\n");
        assert_eq!(remap_line(80, &reversed), Remap::Shifted(77));
        assert_eq!(remap_line(50, &reversed), Remap::Lost);
    }

    #[test]
    fn missing_hunk_length_defaults_to_one() {
        // No comma in either range -> length 1 each. Pure context line at old 50.
        let patch = "@@ -50 +60 @@\n ctx\n";
        let hunks = parse_file_patch(patch);
        assert_eq!(remap_line(50, &hunks), Remap::Shifted(60));
    }

    fn lines(s: &str) -> Vec<&str> {
        s.lines().collect()
    }

    #[test]
    fn relocate_finds_line_in_rewritten_hunk() {
        let old = lines("a\nb\nneedle()\nc\nd");
        let new = lines("a\nb2\nb3\nneedle()\nc\nd");
        assert_eq!(relocate(&old, &new, 3, 3, None), Remap::Shifted(4));
    }

    #[test]
    fn relocate_survives_reindent_and_crlf() {
        let old = lines("x\n  do_thing(1)\ny");
        let new = lines("x\n\t\tdo_thing(1)\r\ny");
        assert_eq!(relocate(&old, &new, 2, 2, None), Remap::Shifted(2));
    }

    #[test]
    fn relocate_moved_block_range() {
        let old = lines("h\nfoo(1)\nfoo(2)\nt\nz");
        let new = lines("z\nq\nq\nq\nh\nfoo(1)\nfoo(2)\nt");
        assert_eq!(relocate(&old, &new, 2, 3, None), Remap::Shifted(6));
    }

    #[test]
    fn relocate_missing_is_lost() {
        let old = lines("a\nneedle()\nb");
        let new = lines("a\nother()\nb");
        assert_eq!(relocate(&old, &new, 2, 2, None), Remap::Lost);
    }

    #[test]
    fn relocate_duplicate_uses_context_then_prediction() {
        // Same line twice; context picks the second.
        let old = lines("p\nq\nval = 1\nr\ns");
        let new = lines("x\nval = 1\ny\np\nq\nval = 1\nr\ns");
        assert_eq!(relocate(&old, &new, 3, 3, None), Remap::Shifted(6));
        // No context anywhere: ambiguous without a prediction, resolved by a nearer one.
        let old = lines("val = 1");
        let new = lines("val = 1\nm\nval = 1");
        assert_eq!(relocate(&old, &new, 1, 1, None), Remap::Lost);
        assert_eq!(relocate(&old, &new, 1, 1, Some(3)), Remap::Shifted(3));
        assert_eq!(relocate(&old, &new, 1, 1, Some(2)), Remap::Lost);
    }

    #[test]
    fn relocate_refuses_trivial_line_without_context() {
        let old = lines("a\n}\nb");
        let new = lines("q\n}\nw");
        assert_eq!(relocate(&old, &new, 2, 2, None), Remap::Lost);
        // With matching context it is fine.
        let new = lines("q\na\n}\nb");
        assert_eq!(relocate(&old, &new, 2, 2, None), Remap::Shifted(3));
    }

    #[test]
    fn relocate_out_of_bounds_is_lost() {
        let old = lines("a");
        assert_eq!(relocate(&old, &old, 2, 2, None), Remap::Lost);
        assert_eq!(relocate(&old, &old, 0, 1, None), Remap::Lost);
    }

    #[test]
    fn predict_line_counts_hunks_above_only() {
        let hunks = parse_file_patch("@@ -10,1 +10,3 @@\n ctx\n+a\n+b\n@@ -100,2 +102,0 @@\n-x\n-y\n");
        assert_eq!(predict_line(50, &hunks), 52);
        assert_eq!(predict_line(100, &hunks), 102);
        assert_eq!(predict_line(200, &hunks), 200);
    }
}
