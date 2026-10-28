//! Port of `core/tools/edit-diff.ts` — exact + fuzzy (NFKC) matching, reverse
//! application, overlap detection, unified diff string with line numbers.

use unicode_normalization::UnicodeNormalization;

pub struct Edit {
    pub old_text: String,
    pub new_text: String,
}

pub fn normalize_to_lf(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

fn detect_line_ending(content: &str) -> &'static str {
    let crlf = content.find("\r\n");
    let lf = content.find('\n');
    match (crlf, lf) {
        (_, None) => "\n",
        (None, _) => "\n",
        (Some(a), Some(b)) => {
            if a < b {
                "\r\n"
            } else {
                "\n"
            }
        }
    }
}

fn restore_line_endings(text: &str, ending: &str) -> String {
    if ending == "\r\n" {
        text.replace('\n', "\r\n")
    } else {
        text.to_string()
    }
}

/// NFKC + trailing-whitespace strip + smart quote/dash/space normalization,
/// matching `normalizeForFuzzyMatch` exactly.
pub fn normalize_for_fuzzy_match(text: &str) -> String {
    let nfkc: String = text.nfkc().collect();
    let lines: Vec<String> = nfkc.split('\n').map(|l| l.trim_end().to_string()).collect();
    let joined = lines.join("\n");
    let mut out = String::with_capacity(joined.len());
    let mut chars = joined.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{201B}' => out.push('\''),
            '\u{201C}' | '\u{201D}' | '\u{201E}' | '\u{201F}' => out.push('"'),
            '\u{2010}' | '\u{2011}' | '\u{2012}' | '\u{2013}' | '\u{2014}' | '\u{2015}'
            | '\u{2212}' => out.push('-'),
            '\u{00A0}' | '\u{202F}' | '\u{205F}' | '\u{3000}' => out.push(' '),
            '\u{2002}'..='\u{200A}' => out.push(' '),
            _ => out.push(c),
        }
    }
    let _ = chars.peek();
    out
}

struct FuzzyMatch {
    found: bool,
    index: usize,
    match_length: usize,
    used_fuzzy: bool,
    content_for_replacement: String,
}

fn fuzzy_find_text(content: &str, old_text: &str) -> FuzzyMatch {
    if let Some(idx) = content.find(old_text) {
        return FuzzyMatch {
            found: true,
            index: idx,
            match_length: old_text.len(),
            used_fuzzy: false,
            content_for_replacement: content.to_string(),
        };
    }
    let fuzzy_content = normalize_for_fuzzy_match(content);
    let fuzzy_old = normalize_for_fuzzy_match(old_text);
    if let Some(idx) = fuzzy_content.find(&fuzzy_old) {
        return FuzzyMatch {
            found: true,
            index: idx,
            match_length: fuzzy_old.len(),
            used_fuzzy: true,
            content_for_replacement: fuzzy_content,
        };
    }
    FuzzyMatch {
        found: false,
        index: 0,
        match_length: 0,
        used_fuzzy: false,
        content_for_replacement: content.to_string(),
    }
}

fn count_occurrences(content: &str, old_text: &str) -> usize {
    let fuzzy_content = normalize_for_fuzzy_match(content);
    let fuzzy_old = normalize_for_fuzzy_match(old_text);
    if fuzzy_old.is_empty() {
        return 0;
    }
    fuzzy_content.matches(&fuzzy_old).count()
}

fn not_found_error(path: &str, edit_index: usize, total: usize) -> String {
    if total == 1 {
        return format!(
            "Could not find the exact text in {path}. The old text must match exactly including all whitespace and newlines."
        );
    }
    format!(
        "Could not find edits[{edit_index}] in {path}. The oldText must match exactly including all whitespace and newlines."
    )
}

fn duplicate_error(path: &str, edit_index: usize, total: usize, occurrences: usize) -> String {
    if total == 1 {
        return format!(
            "Found {occurrences} occurrences of the text in {path}. The text must be unique. Please provide more context to make it unique."
        );
    }
    format!(
        "Found {occurrences} occurrences of edits[{edit_index}] in {path}. Each oldText must be unique. Please provide more context to make it unique."
    )
}

fn empty_old_text_error(path: &str, edit_index: usize, total: usize) -> String {
    if total == 1 {
        return format!("oldText must not be empty in {path}.");
    }
    format!("edits[{edit_index}].oldText must not be empty in {path}.")
}

fn no_change_error(path: &str, total: usize) -> String {
    if total == 1 {
        return format!(
            "No changes made to {path}. The replacement produced identical content. This might indicate an issue with special characters or the text not existing as expected."
        );
    }
    format!("No changes made to {path}. The replacements produced identical content.")
}

pub struct AppliedEdits {
    pub base_content: String,
    pub new_content: String,
}

/// Port of `applyEditsToNormalizedContent`: all edits match the same original
/// content; applied in reverse offset order; fuzzy match switches the whole
/// operation into normalized space.
pub fn apply_edits_to_normalized_content(
    normalized_content: &str,
    edits: &[Edit],
    path: &str,
) -> Result<AppliedEdits, String> {
    let normalized_edits: Vec<Edit> = edits
        .iter()
        .map(|e| Edit {
            old_text: normalize_to_lf(&e.old_text),
            new_text: normalize_to_lf(&e.new_text),
        })
        .collect();

    for (i, e) in normalized_edits.iter().enumerate() {
        if e.old_text.is_empty() {
            return Err(empty_old_text_error(path, i, normalized_edits.len()));
        }
    }

    let any_fuzzy = normalized_edits
        .iter()
        .any(|e| fuzzy_find_text(normalized_content, &e.old_text).used_fuzzy);
    let base_content = if any_fuzzy {
        normalize_for_fuzzy_match(normalized_content)
    } else {
        normalized_content.to_string()
    };

    struct Matched {
        edit_index: usize,
        match_index: usize,
        match_length: usize,
        new_text: String,
    }
    let mut matched: Vec<Matched> = Vec::new();

    for (i, edit) in normalized_edits.iter().enumerate() {
        let m = fuzzy_find_text(&base_content, &edit.old_text);
        if !m.found {
            return Err(not_found_error(path, i, normalized_edits.len()));
        }
        let occurrences = count_occurrences(&base_content, &edit.old_text);
        if occurrences > 1 {
            return Err(duplicate_error(path, i, normalized_edits.len(), occurrences));
        }
        matched.push(Matched {
            edit_index: i,
            match_index: m.index,
            match_length: m.match_length,
            new_text: edit.new_text.clone(),
        });
    }

    matched.sort_by_key(|m| m.match_index);
    for i in 1..matched.len() {
        let prev = &matched[i - 1];
        let cur = &matched[i];
        if prev.match_index + prev.match_length > cur.match_index {
            return Err(format!(
                "edits[{}] and edits[{}] overlap in {path}. Merge them into one edit or target disjoint regions.",
                prev.edit_index, cur.edit_index
            ));
        }
    }

    let mut new_content = base_content.clone();
    for m in matched.iter().rev() {
        let start = m.match_index;
        let end = m.match_index + m.match_length;
        // Byte offsets are valid since indices came from searching the same string.
        new_content = format!(
            "{}{}{}",
            &new_content[..start],
            m.new_text,
            &new_content[end.min(new_content.len())..]
        );
    }

    if base_content == new_content {
        return Err(no_change_error(path, normalized_edits.len()));
    }

    Ok(AppliedEdits { base_content, new_content })
}

/// Port of edit.ts execute: BOM strip, LF normalization, apply, restore endings.
pub fn apply_edits_to_file_content(raw: &str, edits: &[Edit], path: &str) -> Result<String, String> {
    let (bom, text) = strip_bom(raw);
    let ending = detect_line_ending(text);
    let normalized = normalize_to_lf(text);
    let applied = apply_edits_to_normalized_content(&normalized, edits, path)?;
    Ok(format!("{bom}{}", restore_line_endings(&applied.new_content, ending)))
}

fn strip_bom(content: &str) -> (&str, &str) {
    if let Some(rest) = content.strip_prefix('\u{FEFF}') {
        ("\u{FEFF}", rest)
    } else {
        ("", content)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Op {
    Same,
    Del,
    Add,
}

/// Unified diff with line numbers and 4 context lines — port of generateDiffString.
/// Line diffing via LCS on lines (equivalent granularity to the `diff` npm package).
pub fn generate_diff_string(old_content: &str, new_content: &str, context_lines: usize) -> String {
    let old_lines: Vec<&str> = old_content.split('\n').collect();
    let new_lines: Vec<&str> = new_content.split('\n').collect();
    let ops = diff_lines(&old_lines, &new_lines);

    let max_line_num = old_lines.len().max(new_lines.len());
    let width = max_line_num.to_string().len();

    // ops: (op, value) with Same carrying the line
    let mut flat: Vec<(Op, String)> = Vec::new();
    for (o, old_seg, new_seg) in ops {
        match o {
            Op::Same => {
                for l in old_seg {
                    flat.push((Op::Same, (*l).to_string()));
                }
            }
            Op::Del => {
                for l in old_seg {
                    flat.push((Op::Del, (*l).to_string()));
                }
            }
            Op::Add => {
                for l in new_seg {
                    flat.push((Op::Add, (*l).to_string()));
                }
            }
        }
    }

    let mut output: Vec<String> = Vec::new();
    let mut old_num = 1usize;
    let mut new_num = 1usize;
    let mut i = 0usize;

    while i < flat.len() {
        // measure the run
        let op = flat[i].0;
        let mut j = i;
        while j < flat.len() && flat[j].0 == op {
            j += 1;
        }
        let run_len = j - i;
        let is_change = op != Op::Same;

        if is_change {
            for k in i..j {
                match flat[k].0 {
                    Op::Add => {
                        output.push(format!("+{:>width$} {}", new_num, flat[k].1, width = width));
                        new_num += 1;
                    }
                    Op::Del => {
                        output.push(format!("-{:>width$} {}", old_num, flat[k].1, width = width));
                        old_num += 1;
                    }
                    Op::Same => unreachable!(),
                }
            }
        } else {
            let prev_change = i > 0 && flat[i - 1].0 != Op::Same;
            let next_change = j < flat.len() && flat[j].0 != Op::Same;
            let run: Vec<(Op, String)> = flat[i..j].to_vec();

            let push_ctx = |out: &mut Vec<String>, line: &str, old_num: &mut usize, new_num: &mut usize| {
                out.push(format!(" {:>width$} {}", *old_num, line, width = width));
                *old_num += 1;
                *new_num += 1;
            };

            if prev_change && next_change {
                if run_len <= context_lines * 2 {
                    for (_, l) in &run {
                        push_ctx(&mut output, l, &mut old_num, &mut new_num);
                    }
                } else {
                    for (_, l) in run.iter().take(context_lines) {
                        push_ctx(&mut output, l, &mut old_num, &mut new_num);
                    }
                    let skipped = run_len - context_lines * 2;
                    output.push(format!(" {:>width$} ...", "", width = width));
                    old_num += skipped;
                    new_num += skipped;
                    for (_, l) in run.iter().skip(run_len - context_lines) {
                        push_ctx(&mut output, l, &mut old_num, &mut new_num);
                    }
                }
            } else if prev_change {
                let shown = run_len.min(context_lines);
                for (_, l) in run.iter().take(shown) {
                    push_ctx(&mut output, l, &mut old_num, &mut new_num);
                }
                let skipped = run_len - shown;
                if skipped > 0 {
                    output.push(format!(" {:>width$} ...", "", width = width));
                    old_num += skipped;
                    new_num += skipped;
                }
            } else if next_change {
                let skipped = run_len.saturating_sub(context_lines);
                if skipped > 0 {
                    output.push(format!(" {:>width$} ...", "", width = width));
                    old_num += skipped;
                    new_num += skipped;
                }
                for (_, l) in run.iter().skip(skipped) {
                    push_ctx(&mut output, l, &mut old_num, &mut new_num);
                }
            }
            // else: skip entirely; numbers already advanced by the loop below
            if !prev_change && !next_change {
                old_num += run_len;
                new_num += run_len;
            }
        }
        i = j;
    }

    output.join("\n")
}

/// Line-based LCS diff producing ordered runs. (Equivalent to `diff.diffLines`
/// segmentation for our purposes.)
fn diff_lines<'a>(old: &[&'a str], new: &[&'a str]) -> Vec<(Op, Vec<&'a str>, Vec<&'a str>)> {
    let n = old.len();
    let m = new.len();
    let mut table = vec![0u32; (n + 1) * (m + 1)];
    let idx = |i: usize, j: usize| i * (m + 1) + j;
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            table[idx(i, j)] = if old[i] == new[j] {
                table[idx(i + 1, j + 1)] + 1
            } else {
                table[idx(i + 1, j)].max(table[idx(i, j + 1)])
            };
        }
    }
    // Standard LCS backtrace: one op per step; consecutive same-op steps merged below.
    let mut steps: Vec<(Op, &str)> = Vec::new();
    let mut i = 0usize;
    let mut j = 0usize;
    while i < n && j < m {
        if old[i] == new[j] {
            steps.push((Op::Same, old[i]));
            i += 1;
            j += 1;
        } else if table[idx(i + 1, j)] >= table[idx(i, j + 1)] {
            steps.push((Op::Del, old[i]));
            i += 1;
        } else {
            steps.push((Op::Add, new[j]));
            j += 1;
        }
    }
    while i < n {
        steps.push((Op::Del, old[i]));
        i += 1;
    }
    while j < m {
        steps.push((Op::Add, new[j]));
        j += 1;
    }

    let mut merged: Vec<(Op, Vec<&'a str>, Vec<&'a str>)> = Vec::new();
    for (op, line) in steps {
        match merged.last_mut() {
            Some(last) if last.0 == op => match op {
                Op::Add => last.2.push(line),
                Op::Del => last.1.push(line),
                Op::Same => last.1.push(line),
            },
            _ => match op {
                Op::Add => merged.push((op, vec![], vec![line])),
                Op::Del => merged.push((op, vec![line], vec![])),
                Op::Same => merged.push((op, vec![line], vec![])),
            },
        }
    }
    merged
}
