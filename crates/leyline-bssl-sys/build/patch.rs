use std::fs;
use std::io;
use std::path::{Path, PathBuf};

struct Hunk<'a> {
    old_start: usize,
    old_count: usize,
    lines: Vec<(u8, &'a str)>,
    new_no_newline: bool,
}

struct FilePatch<'a> {
    path: PathBuf,
    hunks: Vec<Hunk<'a>>,
}

pub(crate) fn apply_file(root: &Path, patch_path: &Path, strip: usize) -> io::Result<()> {
    let text = fs::read_to_string(patch_path)?;
    let patches = parse_patch(&text, strip).map_err(|msg| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{}: {msg}", patch_path.display()),
        )
    })?;
    if patches.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("{}: no file patches found", patch_path.display()),
        ));
    }
    for file_patch in patches {
        apply_to_target(root, patch_path, &file_patch)?;
    }
    Ok(())
}

fn split_lines(raw: &str) -> (&'static str, bool, Vec<String>) {
    let eol = if raw.contains("\r\n") { "\r\n" } else { "\n" };
    let had_trailing_nl = raw.ends_with('\n');
    let mut lines: Vec<String> = raw
        .split('\n')
        .map(|l| l.strip_suffix('\r').unwrap_or(l).to_owned())
        .collect();
    if had_trailing_nl {
        lines.pop();
    }
    (eol, had_trailing_nl, lines)
}

fn apply_to_target(root: &Path, patch_path: &Path, file_patch: &FilePatch<'_>) -> io::Result<()> {
    let target = root.join(&file_patch.path);
    let raw = fs::read_to_string(&target).map_err(|e| {
        io::Error::new(
            e.kind(),
            format!(
                "{}: cannot read {}: {e}",
                patch_path.display(),
                target.display()
            ),
        )
    })?;
    let (eol, mut had_trailing_nl, mut lines) = split_lines(&raw);
    let mut delta: isize = 0;

    for (idx, hunk) in file_patch.hunks.iter().enumerate() {
        let pos = find_hunk_pos(&lines, hunk, delta).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "{}: hunk #{} (line {}) failed to apply to {}",
                    patch_path.display(),
                    idx + 1,
                    hunk.old_start,
                    file_patch.path.display()
                ),
            )
        })?;
        let replacement: Vec<String> = hunk
            .lines
            .iter()
            .filter(|(tag, _)| *tag == b' ' || *tag == b'+')
            .map(|(_, line)| (*line).to_owned())
            .collect();
        let at_eof = pos + hunk.old_count == lines.len();
        delta += replacement.len() as isize - hunk.old_count as isize;
        lines.splice(pos..pos + hunk.old_count, replacement);
        if at_eof && hunk.new_no_newline {
            had_trailing_nl = false;
        }
    }

    let mut out = lines.join(eol);
    if had_trailing_nl && !lines.is_empty() {
        out.push_str(eol);
    }
    fs::write(&target, out)
}

fn find_hunk_pos(lines: &[String], hunk: &Hunk<'_>, delta: isize) -> Option<usize> {
    let old_lines: Vec<&str> = hunk
        .lines
        .iter()
        .filter(|(tag, _)| *tag == b' ' || *tag == b'-')
        .map(|(_, l)| *l)
        .collect();
    if old_lines.len() > lines.len() {
        return None;
    }
    let max_start = lines.len() - old_lines.len();
    let base =
        (hunk.old_start.saturating_sub(1) as isize + delta).clamp(0, max_start as isize) as usize;
    let matches_at = |start: usize| -> bool {
        old_lines
            .iter()
            .enumerate()
            .all(|(i, expected)| lines[start + i] == *expected)
    };
    if matches_at(base) {
        return Some(base);
    }
    let max_offset = base.max(max_start - base);
    for offset in 1..=max_offset {
        if base + offset <= max_start && matches_at(base + offset) {
            return Some(base + offset);
        }
        if offset <= base && matches_at(base - offset) {
            return Some(base - offset);
        }
    }
    None
}

fn parse_patch(text: &str, strip: usize) -> Result<Vec<FilePatch<'_>>, String> {
    let raw_lines: Vec<&str> = text
        .split('\n')
        .map(|l| l.strip_suffix('\r').unwrap_or(l))
        .collect();
    let mut patches = Vec::new();
    let mut i = 0;
    while i < raw_lines.len() {
        if let Some(old_hdr) = raw_lines[i].strip_prefix("--- ") {
            let new_hdr = raw_lines
                .get(i + 1)
                .and_then(|l| l.strip_prefix("+++ "))
                .ok_or_else(|| format!("line {}: missing +++ header", i + 1))?;
            let path = strip_path(new_hdr, strip)
                .or_else(|| strip_path(old_hdr, strip))
                .ok_or_else(|| format!("line {}: invalid patch path '{new_hdr}'", i + 2))?;
            i += 2;
            let (hunks, next_i) = parse_hunks(&raw_lines, i)?;
            patches.push(FilePatch { path, hunks });
            i = next_i;
        } else {
            i += 1;
        }
    }
    Ok(patches)
}

fn parse_hunks<'a>(raw_lines: &[&'a str], mut i: usize) -> Result<(Vec<Hunk<'a>>, usize), String> {
    let mut hunks = Vec::new();
    while i < raw_lines.len() {
        let line = raw_lines[i];
        if line.starts_with("--- ") || line.starts_with("diff --git ") {
            break;
        }
        if !line.starts_with("@@ ") {
            i += 1;
            continue;
        }
        let (old_start, old_count, new_count) = parse_hunk_header(line, i + 1)?;
        i += 1;
        let (hunk, next_i) = parse_hunk_body(raw_lines, i, old_start, old_count, new_count)?;
        hunks.push(hunk);
        i = next_i;
    }
    Ok((hunks, i))
}

fn parse_hunk_line(line: &str, line_no: usize) -> Result<(u8, &str), String> {
    match line.as_bytes().first().copied() {
        Some(tag @ (b' ' | b'-' | b'+')) => Ok((tag, &line[1..])),
        None => Ok((b' ', "")),
        Some(other) => Err(format!(
            "line {line_no}: unexpected hunk prefix '{}'",
            other as char
        )),
    }
}

fn consume_no_newline(raw_lines: &[&str], i: &mut usize, last_tag: u8, flag: &mut bool) -> bool {
    if raw_lines.get(*i).is_some_and(|l| l.starts_with("\\ ")) {
        *flag |= last_tag != b'-';
        *i += 1;
        return true;
    }
    false
}

fn parse_hunk_body<'a>(
    raw_lines: &[&'a str],
    mut i: usize,
    old_start: usize,
    old_count: usize,
    new_count: usize,
) -> Result<(Hunk<'a>, usize), String> {
    let mut lines = Vec::with_capacity(old_count.max(new_count));
    let (mut seen_old, mut seen_new) = (0, 0);
    let mut last_tag = b' ';
    let mut new_no_newline = false;

    while i < raw_lines.len() && (seen_old < old_count || seen_new < new_count) {
        if consume_no_newline(raw_lines, &mut i, last_tag, &mut new_no_newline) {
            continue;
        }
        let (tag, body) = parse_hunk_line(raw_lines[i], i + 1)?;
        seen_old += usize::from(tag != b'+');
        seen_new += usize::from(tag != b'-');
        last_tag = tag;
        lines.push((tag, body));
        i += 1;
    }
    consume_no_newline(raw_lines, &mut i, last_tag, &mut new_no_newline);
    if seen_old != old_count || seen_new != new_count {
        return Err(format!(
            "line {i}: truncated hunk (expected -{old_count}/+{new_count}, got -{seen_old}/+{seen_new})"
        ));
    }
    Ok((
        Hunk {
            old_start,
            old_count,
            lines,
            new_no_newline,
        },
        i,
    ))
}

fn parse_hunk_header(line: &str, line_no: usize) -> Result<(usize, usize, usize), String> {
    let inner = line
        .strip_prefix("@@ ")
        .and_then(|s| s.split(" @@").next())
        .ok_or_else(|| format!("line {line_no}: malformed hunk header '{line}'"))?;
    let mut parts = inner.split_whitespace();
    let old_part = parts
        .next()
        .and_then(|s| s.strip_prefix('-'))
        .ok_or_else(|| format!("line {line_no}: missing old range"))?;
    let new_part = parts
        .next()
        .and_then(|s| s.strip_prefix('+'))
        .ok_or_else(|| format!("line {line_no}: missing new range"))?;
    let (old_start, old_count) = parse_range(old_part, line_no)?;
    let (_, new_count) = parse_range(new_part, line_no)?;
    Ok((old_start, old_count, new_count))
}

fn parse_range(part: &str, line_no: usize) -> Result<(usize, usize), String> {
    match part.split_once(',') {
        Some((start, count)) => Ok((
            start
                .parse()
                .map_err(|_| format!("line {line_no}: invalid range start '{start}'"))?,
            count
                .parse()
                .map_err(|_| format!("line {line_no}: invalid range count '{count}'"))?,
        )),
        None => Ok((
            part.parse()
                .map_err(|_| format!("line {line_no}: invalid range '{part}'"))?,
            1,
        )),
    }
}

fn strip_path(raw: &str, strip: usize) -> Option<PathBuf> {
    let path = raw.split('\t').next()?.trim();
    if path == "/dev/null" {
        return None;
    }
    let mut parts = path.split('/');
    for _ in 0..strip {
        parts.next()?;
    }
    let rest: PathBuf = parts.collect();
    if rest.as_os_str().is_empty() {
        None
    } else {
        Some(rest)
    }
}
