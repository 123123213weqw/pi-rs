//! System prompt construction — port of `coding-agent/src/core/system-prompt.ts`
//! (pi 0.74.2), byte-for-byte structure with the default (non-custom) branch.

use crate::context::ContextFile;

pub struct SystemPromptOptions<'a> {
    pub custom_prompt: Option<&'a str>,
    pub append_system_prompt: Option<&'a str>,
    pub selected_tools: &'a [String],
    /// One-line snippets keyed by tool name (pi: ToolDef.promptSnippet).
    pub tool_snippets: &'a [(&'a str, &'a str)],
    pub cwd: &'a str,
    pub context_files: &'a [ContextFile],
}

pub fn build_system_prompt(o: SystemPromptOptions) -> String {
    let prompt_cwd = o.cwd.replace('\\', "/");
    let date = today();

    let append_section = o
        .append_system_prompt
        .map(|s| format!("\n\n{s}"))
        .unwrap_or_default();

    if let Some(custom) = o.custom_prompt {
        let mut prompt = custom.to_string();
        prompt.push_str(&append_section);
        append_context_files(&mut prompt, o.context_files);
        if o.selected_tools.iter().any(|t| t == "read") {
            append_skills_section(&mut prompt);
        }
        prompt.push_str(&format!("\nCurrent date: {date}"));
        prompt.push_str(&format!("\nCurrent working directory: {prompt_cwd}"));
        return prompt;
    }

    // Default prompt: tools list only shows tools that have a snippet.
    let visible: Vec<String> = o
        .selected_tools
        .iter()
        .filter_map(|t| {
            o.tool_snippets
                .iter()
                .find(|(n, _)| n == t)
                .map(|(n, s)| format!("- {n}: {s}"))
        })
        .collect();
    let tools_list = if visible.is_empty() {
        "(none)".to_string()
    } else {
        visible.join("\n")
    };

    let has_bash = o.selected_tools.iter().any(|t| t == "bash");
    let has_grep = o.selected_tools.iter().any(|t| t == "grep");
    let has_find = o.selected_tools.iter().any(|t| t == "find");
    let has_ls = o.selected_tools.iter().any(|t| t == "ls");
    let has_read = o.selected_tools.iter().any(|t| t == "read");

    let mut guidelines: Vec<String> = Vec::new();
    if has_bash && !has_grep && !has_find && !has_ls {
        guidelines.push("Use bash for file operations like ls, rg, find".into());
    } else if has_bash && (has_grep || has_find || has_ls) {
        guidelines.push(
            "Prefer grep/find/ls tools over bash for file exploration (faster, respects .gitignore)".into(),
        );
    }
    // Per-tool promptGuidelines, in selected-tools order (pi: ToolDef.promptGuidelines).
    for t in o.selected_tools {
        if let Some(gs) = crate::tools::prompt_guidelines(t) {
            for g in gs {
                if !guidelines.iter().any(|x| x == g) {
                    guidelines.push(g.to_string());
                }
            }
        }
    }
    guidelines.push("Be concise in your responses".into());
    guidelines.push("Show file paths clearly when working with files".into());
    let guidelines = guidelines.iter().map(|g| format!("- {g}")).collect::<Vec<_>>().join("\n");

    let readme_path = docs_path("README.md");
    let docs_path_p = docs_path("docs");
    let examples_path = docs_path("examples");

    let mut prompt = format!(
        "You are an expert coding assistant operating inside pi, a coding agent harness. You help users by reading files, executing commands, editing code, and writing new files.

Available tools:
{tools_list}

In addition to the tools above, you may have access to other custom tools depending on the project.

Guidelines:
{guidelines}

Pi documentation (read only when the user asks about pi itself, its SDK, extensions, themes, skills, or TUI):
- Main documentation: {readme_path}
- Additional docs: {docs_path_p}
- Examples: {examples_path} (extensions, custom tools, SDK)
- When asked about: extensions (docs/extensions.md, examples/extensions/), themes (docs/themes.md), skills (docs/skills.md), prompt templates (docs/prompt-templates.md), TUI components (docs/tui.md), keybindings (docs/keybindings.md), SDK integrations (docs/sdk.md), custom providers (docs/custom-provider.md), adding models (docs/models.md), pi packages (docs/packages.md)
- When working on pi topics, read the docs and examples, and follow .md cross-references before implementing
- Always read pi .md files completely and follow links to related docs (e.g., tui.md for TUI API details)"
    );

    prompt.push_str(&append_section);
    append_context_files(&mut prompt, o.context_files);
    if has_read {
        append_skills_section(&mut prompt);
    }
    prompt.push_str(&format!("\nCurrent date: {date}"));
    prompt.push_str(&format!("\nCurrent working directory: {prompt_cwd}"));

    prompt
}

fn append_context_files(prompt: &mut String, files: &[ContextFile]) {
    if files.is_empty() {
        return;
    }
    prompt.push_str("\n\n# Project Context\n\n");
    prompt.push_str("Project-specific instructions and guidelines:\n\n");
    for f in files {
        prompt.push_str(&format!("## {}\n\n{}\n\n", f.path.display(), f.content));
    }
}

fn append_skills_section(_prompt: &mut String) {
    // Skills discovery (`.pi/skills`, `~/.pi/agent/skills`) — no skills installed
    // renders nothing; formatSkillsForPrompt emits nothing for an empty list.
}

fn docs_path(rel: &str) -> String {
    // pi prints paths into its installed package. This port resolves against
    // PI_DOC_DIR, falling back to the install prefix.
    let base = std::env::var("PI_DOC_DIR").unwrap_or_else(|_| "/usr/local/share/pi".into());
    format!("{base}/{rel}")
}

fn today() -> String {
    let iso = crate::util::iso_now();
    iso[..10].to_string()
}
