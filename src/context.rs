//! Project context loading — port of `core/resource-loader.ts` context-file logic.
//! Global `~/.pi/agent/AGENTS.md` first, then cwd → root ancestors; per directory
//! the first of AGENTS.md / AGENTS.MD / CLAUDE.md / CLAUDE.MD wins.

use std::path::{Path, PathBuf};

pub struct ContextFile {
    pub path: PathBuf,
    pub content: String,
}

const CANDIDATES: [&str; 4] = ["AGENTS.md", "AGENTS.MD", "CLAUDE.md", "CLAUDE.MD"];

fn load_from_dir(dir: &Path) -> Option<ContextFile> {
    for name in CANDIDATES {
        let p = dir.join(name);
        if p.is_file() {
            if let Ok(content) = std::fs::read_to_string(&p) {
                return Some(ContextFile { path: p, content });
            }
        }
    }
    None
}

pub fn load_project_context_files(cwd: &Path, agent_dir: &Path) -> Vec<ContextFile> {
    let mut out: Vec<ContextFile> = Vec::new();
    let mut seen: Vec<PathBuf> = Vec::new();

    if let Some(global) = load_from_dir(agent_dir) {
        seen.push(global.path.clone());
        out.push(global);
    }

    // Ancestors from cwd up to root; unshift so the deepest directory ends last
    // (order: global, root … cwd).
    let mut ancestors: Vec<ContextFile> = Vec::new();
    let mut cur = Some(cwd.to_path_buf());
    while let Some(dir) = cur {
        if let Some(f) = load_from_dir(&dir) {
            if !seen.contains(&f.path) {
                seen.push(f.path.clone());
                ancestors.insert(0, f);
            }
        }
        cur = dir.parent().map(|p| p.to_path_buf());
    }
    out.extend(ancestors);
    out
}
