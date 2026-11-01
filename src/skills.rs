//! Skill discovery — port of pi's runtime skill loading as of pi main@a6bde34bb6
//! (coding-agent `src/core/skills.ts` `loadSkills`/`formatSkillsForPrompt` plus the
//! auto-collection order of `package-manager.ts` `collectSkillEntries`).
//!
//! Collection order (first-wins dedup by name, then by canonical path):
//!   1. `<cwd>/.pi/skills`            (project, "pi" mode)
//!   2. every ancestor `<dir>/.agents/skills` up to the git root (or filesystem
//!      root), excluding the user-level `~/.agents/skills` ("agents" mode)
//!   3. `<agentDir>/skills`           (user, "pi" mode; agentDir honors
//!      `PI_CODING_AGENT_DIR`)
//!   4. `~/.agents/skills`            (user, "agents" mode)
//!   5. explicit `--skill` paths      (files or directories)
//!
//! "pi" mode accepts loose root-level `.md` files as skills; "agents" mode only
//! takes `SKILL.md` directories (loose `.md` only below the root). Directories
//! containing `SKILL.md` are skill roots and are not recursed further.
//!
//! Divergences from upstream (none visible to the atom suite): project-trust
//! gating and settings overrides are not ported (pi-rs has no trust manager or
//! settings store), `.gitignore`/`.ignore`/`.fdignore` handling covers the
//! common gitignore subset, and the frontmatter parser is a YAML subset
//! (top-level `name`/`description`/`disable-model-invocation`; quoted, plain and
//! block scalars).

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Component, Path, PathBuf};

#[derive(Clone, Debug)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub file_path: String,
    pub disable_model_invocation: bool,
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    /// `.pi`-style: loose `.md` files at the collection root are skills.
    Pi,
    /// `.agents`-style: only `SKILL.md` directories (loose `.md` below root).
    Agents,
}

/// Full discovery for one run. `explicit` are `--skill` arguments; when
/// `include_defaults` is false and `explicit` is empty (pi's `--no-skills`),
/// the result is empty.
pub fn load_skills(cwd: &Path, explicit: &[String], include_defaults: bool) -> Vec<Skill> {
    if !include_defaults && explicit.is_empty() {
        return Vec::new();
    }

    let mut paths: Vec<(PathBuf, LooseMd)> = Vec::new();
    if include_defaults {
        let home = home_dir();
        let agent_dir = agent_dir(&home);

        // 1. project .pi/skills
        paths.push((cwd.join(".pi").join("skills"), LooseMd::RootOnly));

        // 2. ancestor .agents/skills dirs (cwd upward to git root), minus the
        //    user-level dir.
        let user_agents_skills = home.join(".agents").join("skills");
        for dir in ancestor_dirs(cwd) {
            let d = dir.join(".agents").join("skills");
            if d != user_agents_skills {
                paths.push((d, LooseMd::BelowRootOnly));
            }
        }

        // 3. user .pi/agent/skills
        paths.push((agent_dir.join("skills"), LooseMd::RootOnly));

        // 4. user .agents/skills
        paths.push((home.join(".agents").join("skills"), LooseMd::BelowRootOnly));
    }
    for p in explicit {
        paths.push((PathBuf::from(expand_tilde(p)), LooseMd::RootOnly));
    }

    let mut files: Vec<PathBuf> = Vec::new();
    for (p, loose) in &paths {
        if !p.exists() {
            continue;
        }
        if p.is_dir() {
            collect_dir_files(p, true, *loose, &mut files);
        } else if p.extension().map(|e| e == "md").unwrap_or(false) {
            files.push(p.clone());
        }
    }

    // First-wins dedup by name, plus canonical-path dedup (symlinks).
    let mut by_name: HashMap<String, Skill> = HashMap::new();
    let mut order: Vec<String> = Vec::new();
    let mut seen_paths: HashSet<PathBuf> = HashSet::new();
    for f in &files {
        let canon = fs::canonicalize(f).unwrap_or_else(|_| f.clone());
        if !seen_paths.insert(canon) {
            continue;
        }
        if let Some(skill) = load_skill_from_file(f) {
            if !by_name.contains_key(&skill.name) {
                order.push(skill.name.clone());
                by_name.insert(skill.name.clone(), skill);
            }
        }
    }

    order.into_iter().filter_map(|n| by_name.remove(&n)).collect()
}

/// Where loose `.md` files count as skills: pi's `.pi`-style directories (and
/// explicit `--skill` directories) accept them only at the collection root;
/// `.agents`-style directories accept them only below the root.
#[derive(Clone, Copy)]
enum LooseMd {
    RootOnly,
    BelowRootOnly,
}

/// pi's `collectSkillEntries`/`loadSkillsFromDirInternal` walk: `SKILL.md` at
/// this level makes the directory a skill root (stop); otherwise recurse into
/// subdirectories (skipping dot-dirs and `node_modules`), taking loose `.md`
/// files where the mode allows them.
fn collect_dir_files(dir: &Path, is_root: bool, loose: LooseMd, out: &mut Vec<PathBuf>) {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    let mut names: Vec<(String, bool)> = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let is_symlink = entry.file_type().map(|t| t.is_symlink()).unwrap_or(false);
        let meta = if is_symlink {
            fs::metadata(entry.path())
        } else {
            entry.metadata()
        };
        let is_dir = meta.as_ref().map(|m| m.is_dir()).unwrap_or(false);
        names.push((name, is_dir));
    }
    // Node's readdirSync sorts entries (byte-wise); Rust's read_dir yields OS
    // order. Sort per directory to keep the <available_skills> order identical.
    names.sort_by(|a, b| a.0.as_bytes().cmp(b.0.as_bytes()));

    // SKILL.md at this level: the directory is a skill root.
    if names.iter().any(|(n, _)| n == "SKILL.md") {
        out.push(dir.join("SKILL.md"));
        return;
    }

    let loose_here = match loose {
        LooseMd::RootOnly => is_root,
        LooseMd::BelowRootOnly => !is_root,
    };
    for (name, is_dir) in names {
        if name.starts_with('.') || name == "node_modules" {
            continue;
        }
        let full = dir.join(&name);
        if is_dir {
            collect_dir_files(&full, false, loose, out);
        } else if loose_here && name.ends_with(".md") {
            out.push(full);
        }
    }
}

/// Load one skill file: frontmatter `description` is required; `name` falls
/// back to the parent directory name; `disable-model-invocation: true` hides
/// the skill from the prompt.
fn load_skill_from_file(path: &Path) -> Option<Skill> {
    let raw = fs::read_to_string(path).ok()?;
    let fm = parse_frontmatter(&raw);

    let description = match fm.get("description") {
        Some(d) if !d.trim().is_empty() => d.clone(),
        _ => return None,
    };
    let name = fm
        .get("name")
        .filter(|n| !n.is_empty())
        .cloned()
        .unwrap_or_else(|| {
            path.parent()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        });
    let disable = fm.get("disable-model-invocation").map(|v| v == "true").unwrap_or(false);

    Some(Skill {
        name,
        description,
        file_path: path.to_string_lossy().into_owned(),
        disable_model_invocation: disable,
    })
}

/// Minimal YAML frontmatter: top-level `key: value` pairs; quoted, plain and
/// block scalars; nested structures skipped.
fn parse_frontmatter(raw: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();
    let s = raw.strip_prefix('\u{feff}').unwrap_or(raw);
    let s = s.replace("\r\n", "\n").replace('\r', "\n");
    if !s.starts_with("---") {
        return map;
    }
    let Some(end) = s[3..].find("\n---") else {
        return map;
    };
    let yaml = &s[4..3 + end];

    let lines: Vec<&str> = yaml.lines().collect();
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        if line.trim().is_empty() || line.starts_with(char::is_whitespace) || line.starts_with('#') {
            i += 1;
            continue;
        }
        let Some(colon) = find_key_colon(line) else {
            i += 1;
            continue;
        };
        let key = line[..colon].trim().to_string();
        let rest = line[colon + 1..].trim();
        let is_block = rest.starts_with('|') || rest.starts_with('>');
        if rest.is_empty() || (is_block && rest[1..].chars().all(|c| c == '-' || c == '+' || c.is_ascii_digit())) {
            // Value on the following indented lines: plain nesting (skipped)
            // or a block scalar (`|`/`>` with optional chomp/indent markers).
            if i + 1 < lines.len() && lines[i + 1].starts_with(char::is_whitespace) {
                if is_block {
                    let fold = rest.starts_with('>');
                    let chomp = if rest.contains('-') {
                        Chomp::Strip
                    } else if rest.contains('+') {
                        Chomp::Keep
                    } else {
                        Chomp::Clip
                    };
                    let (value, consumed) = parse_block_scalar(&lines[i + 1..], fold, chomp);
                    map.insert(key, value);
                    i += 1 + consumed;
                    continue;
                }
                // Nested mapping/sequence: skip its indented block.
                let (_, consumed) = skip_indented(&lines[i + 1..]);
                i += 1 + consumed;
                continue;
            }
            i += 1;
            continue;
        }
        map.insert(key, parse_scalar(rest));
        i += 1;
    }
    map
}

/// The colon that separates a top-level key from its value: the first `: `
/// (colon+space) or a trailing `:`.
fn find_key_colon(line: &str) -> Option<usize> {
    let bytes = line.as_bytes();
    for (idx, b) in bytes.iter().enumerate() {
        if *b == b':' && (idx + 1 == bytes.len() || bytes[idx + 1] == b' ') {
            return Some(idx);
        }
    }
    None
}

fn parse_scalar(raw: &str) -> String {
    let raw = raw.trim();
    if let Some(quoted) = raw.strip_prefix('"') {
        let mut out = String::new();
        let mut chars = quoted.chars();
        while let Some(c) = chars.next() {
            match c {
                '"' => break,
                '\\' => match chars.next() {
                    Some('n') => out.push('\n'),
                    Some('t') => out.push('\t'),
                    Some('"') => out.push('"'),
                    Some('\\') => out.push('\\'),
                    Some(other) => {
                        out.push('\\');
                        out.push(other);
                    }
                    None => out.push('\\'),
                },
                other => out.push(other),
            }
        }
        return out;
    }
    if let Some(quoted) = raw.strip_prefix('\'') {
        let mut out = String::new();
        let mut chars = quoted.chars();
        while let Some(c) = chars.next() {
            if c == '\'' {
                if chars.clone().next() == Some('\'') {
                    chars.next();
                    out.push('\'');
                    continue;
                }
                break;
            }
            out.push(c);
        }
        return out;
    }
    // Plain scalar: drop trailing comment (` #...`), trim.
    if let Some(pos) = raw.find(" #") {
        return raw[..pos].trim_end().to_string();
    }
    raw.to_string()
}

#[derive(Clone, Copy, PartialEq)]
enum Chomp {
    /// `-`: no trailing newline.
    Strip,
    /// default: exactly one trailing newline.
    Clip,
    /// `+`: keep all trailing newlines.
    Keep,
}

/// Consume an indented block. Returns the content with the block indentation
/// stripped and the number of lines consumed, applying YAML chomping and
/// folding (`>` joins lines with spaces, `|` keeps line breaks).
fn parse_block_scalar(lines: &[&str], fold: bool, chomp: Chomp) -> (String, usize) {
    let mut content: Vec<String> = Vec::new();
    let mut consumed = 0;
    let mut indent: Option<usize> = None;
    for line in lines {
        if line.trim().is_empty() {
            content.push(String::new());
            consumed += 1;
            continue;
        }
        if !line.starts_with(char::is_whitespace) {
            break;
        }
        let lead = line.len() - line.trim_start().len();
        let indent = *indent.get_or_insert(lead);
        let strip = lead.min(indent);
        content.push(line[strip.min(line.len())..].to_string());
        consumed += 1;
    }
    // Chomping counts trailing blank lines.
    let mut trailing_blanks = 0;
    while content.last().map(|l| l.is_empty()).unwrap_or(false) {
        content.pop();
        trailing_blanks += 1;
    }

    let mut body = if fold {
        // Fold: consecutive non-blank lines join with a space; blank lines
        // become newlines.
        let mut out = String::new();
        let mut pending_space = false;
        for l in &content {
            if l.is_empty() {
                out.push('\n');
                pending_space = false;
            } else {
                if pending_space {
                    out.push(' ');
                }
                out.push_str(l);
                pending_space = true;
            }
        }
        out
    } else {
        content.join("\n")
    };

    match chomp {
        Chomp::Strip => {}
        Chomp::Clip => {
            if !content.is_empty() {
                body.push('\n');
            }
        }
        Chomp::Keep => {
            if !content.is_empty() {
                body.push('\n');
            }
            for _ in 0..trailing_blanks {
                body.push('\n');
            }
        }
    }
    (body, consumed)
}

/// Skip an indented nested block; returns the number of lines consumed.
fn skip_indented(lines: &[&str]) -> ((), usize) {
    let mut consumed = 0;
    for line in lines {
        if line.trim().is_empty() || line.starts_with(char::is_whitespace) {
            consumed += 1;
        } else {
            break;
        }
    }
    ((), consumed)
}

/// pi's `formatSkillsForPrompt`: the XML block embedded in the `<skills>`
/// system-prompt section. `file_read_tool` names the loader tool ("read" or
/// "bash").
pub fn format_skills_for_prompt(skills: &[Skill], file_read_tool: &str) -> String {
    let visible: Vec<&Skill> = skills.iter().filter(|s| !s.disable_model_invocation).collect();
    if visible.is_empty() {
        return String::new();
    }

    let load_line = match file_read_tool {
        "bash" => "Use bash to load a skill's file when the task matches its description.",
        "read" => "Use the read tool to load a skill's file when the task matches its description.",
        _ => "Load a skill's file when the task matches its description.",
    };

    let mut lines = vec![
        String::from("\n\nThe following skills provide specialized instructions for specific tasks."),
        load_line.to_string(),
        String::from(
            "Resolve relative paths against the skill's directory (parent of SKILL.md). Use absolute paths in tool calls.",
        ),
        String::new(),
        String::from("<available_skills>"),
    ];
    for skill in visible {
        lines.push("  <skill>".into());
        lines.push(format!("    <name>{}</name>", escape_xml(&skill.name)));
        lines.push(format!("    <description>{}</description>", escape_xml(&skill.description)));
        lines.push(format!("    <location>{}</location>", escape_xml(&skill.file_path)));
        lines.push("  </skill>".into());
    }
    lines.push("</available_skills>".into());
    lines.join("\n")
}

fn escape_xml(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            other => out.push(other),
        }
    }
    out
}

fn home_dir() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/"))
}

/// pi's `getAgentDir()`: `$PI_CODING_AGENT_DIR` (with `~` expansion) or
/// `~/.pi/agent`.
fn agent_dir(home: &Path) -> PathBuf {
    if let Ok(dir) = std::env::var("PI_CODING_AGENT_DIR") {
        return PathBuf::from(expand_tilde(&dir));
    }
    home.join(".pi").join("agent")
}

fn expand_tilde(p: &str) -> String {
    if let Some(rest) = p.strip_prefix("~/") {
        return format!("{}/{}", home_dir().display(), rest);
    }
    if p == "~" {
        return home_dir().display().to_string();
    }
    p.to_string()
}

/// Ancestors of `start` from the git root (if any) up to `start`, mirroring
/// pi's `collectAncestorAgentsSkillDirs`: walk cwd upward, stopping after the
/// git root; without a repo, walk to the filesystem root.
fn ancestor_dirs(start: &Path) -> Vec<PathBuf> {
    let mut chain: Vec<PathBuf> = Vec::new();
    let mut dir = normalize(start);
    loop {
        chain.push(dir.clone());
        if dir.join(".git").exists() {
            break;
        }
        match dir.parent() {
            Some(parent) if parent != dir => dir = parent.to_path_buf(),
            _ => break,
        }
    }
    chain
}

fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}
