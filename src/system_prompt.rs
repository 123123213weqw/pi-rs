//! System prompt construction — port of `coding-agent/src/core/system-prompt.ts`
//! as of **pi v1.4.0**: sectioned prompt (preamble + XML-tagged sections joined
//! with blank lines), rendered via `getSystemMessageText` semantics.
//!
//! Wire layout for the default run (captured from real pi 1.4.0):
//! preamble \n\n <tools>…</tools> \n\n <rules>…</rules> \n\n <docs>…</docs> \n\n <cwd>…</cwd>
//! No date line; `<addendum>`/`<project_context>`/`<skills>` sections are
//! emitted only when their inputs are non-empty.

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

/// Named prompt sections (pi v1.4.0 SystemPromptSections): preamble untagged,
/// everything else wrapped in its tag. Ordered as pi emits them. The session's
/// `role:"system"` message stores this map verbatim.
pub fn build_sections(o: SystemPromptOptions) -> serde_json::Value {
    let prompt_cwd = o.cwd.replace('\\', "/");
    let mut sections: Vec<(String, String)> = Vec::new();

    match o.custom_prompt {
        Some(custom) => sections.push(("preamble".into(), custom.to_string())),
        None => {
            sections.push((
                "preamble".into(),
                "You are an expert coding assistant operating inside pi, a coding agent harness. You help users by reading files, executing commands, editing code, and writing new files.".into(),
            ));
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
            let tail =
                "In addition to the tools above, you may have access to other custom tools depending on the project.";
            let tools_body = if visible.is_empty() {
                format!("(none)\n\n{tail}")
            } else {
                format!("{}\n\n{tail}", visible.join("\n"))
            };
            sections.push(("tools".into(), wrap_section("tools", &tools_body)));
            sections.push(("rules".into(), wrap_section("rules", &build_rules(o.selected_tools))));

            let readme_path = docs_path("README.md");
            let docs_path_p = docs_path("docs");
            let examples_path = docs_path("examples");
            let docs_body = format!(
                "Pi documentation (read only when the user asks about pi itself, its SDK, extensions, themes, skills, or TUI):\n- Main documentation: {readme_path}\n- Additional docs: {docs_path_p}\n- Examples: {examples_path} (extensions, custom tools, SDK)\n- When reading pi docs or examples, resolve docs/... under Additional docs and examples/... under Examples, not the current working directory\n- When asked about: extensions (docs/extensions.md, examples/extensions/), themes (docs/themes.md), skills (docs/skills.md), prompt templates (docs/prompt-templates.md), TUI components (docs/tui.md), keybindings (docs/keybindings.md), SDK integrations (docs/sdk.md), custom providers (docs/custom-provider.md), adding models (docs/models.md), pi packages (docs/packages.md), environment variables (docs/environment-variables.md), MCP servers (docs/mcp.md), codemode scripts and non-LLM models such as classifiers and image models (docs/codemode.md)\n- When working on pi topics, read the docs and examples, and follow .md cross-references before implementing\n- Always read pi .md files completely and follow links to related docs (e.g., tui.md for TUI API details)"
            );
            sections.push(("docs".into(), wrap_section("docs", &docs_body)));
        }
    }

    if let Some(append) = o.append_system_prompt {
        if !append.is_empty() {
            sections.push(("addendum".into(), wrap_section("addendum", append)));
        }
    }

    if !o.context_files.is_empty() {
        let files: Vec<String> = o
            .context_files
            .iter()
            .map(|f| {
                format!(
                    "<project_instructions path=\"{}\">\n{}\n</project_instructions>",
                    f.path.display(),
                    f.content
                )
            })
            .collect();
        let body = format!(
            "Project-specific instructions and guidelines:\n\n{}",
            files.join("\n\n")
        );
        sections.push(("project_context".into(), wrap_section("project_context", &body)));
    }

    sections.push(("cwd".into(), wrap_section("cwd", &prompt_cwd)));

    let mut map = serde_json::Map::new();
    for (name, content) in sections {
        map.insert(name, serde_json::Value::String(content));
    }
    serde_json::Value::Object(map)
}

pub fn build_system_prompt(o: SystemPromptOptions) -> String {
    let prompt_cwd = o.cwd.replace('\\', "/");

    let mut parts: Vec<String> = Vec::new();

    match o.custom_prompt {
        Some(custom) => parts.push(custom.to_string()),
        None => {
            parts.push(
                "You are an expert coding assistant operating inside pi, a coding agent harness. You help users by reading files, executing commands, editing code, and writing new files."
                    .to_string(),
            );

            // <tools>: snippet list, then the custom-tools sentence.
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
            let tail =
                "In addition to the tools above, you may have access to other custom tools depending on the project.";
            let tools_body = if visible.is_empty() {
                format!("(none)\n\n{tail}")
            } else {
                format!("{}\n\n{tail}", visible.join("\n"))
            };
            parts.push(wrap_section("tools", &tools_body));

            parts.push(wrap_section("rules", &build_rules(o.selected_tools)));

            let readme_path = docs_path("README.md");
            let docs_path_p = docs_path("docs");
            let examples_path = docs_path("examples");
            let docs_body = format!(
                "Pi documentation (read only when the user asks about pi itself, its SDK, extensions, themes, skills, or TUI):\n- Main documentation: {readme_path}\n- Additional docs: {docs_path_p}\n- Examples: {examples_path} (extensions, custom tools, SDK)\n- When reading pi docs or examples, resolve docs/... under Additional docs and examples/... under Examples, not the current working directory\n- When asked about: extensions (docs/extensions.md, examples/extensions/), themes (docs/themes.md), skills (docs/skills.md), prompt templates (docs/prompt-templates.md), TUI components (docs/tui.md), keybindings (docs/keybindings.md), SDK integrations (docs/sdk.md), custom providers (docs/custom-provider.md), adding models (docs/models.md), pi packages (docs/packages.md), environment variables (docs/environment-variables.md), MCP servers (docs/mcp.md), codemode scripts and non-LLM models such as classifiers and image models (docs/codemode.md)\n- When working on pi topics, read the docs and examples, and follow .md cross-references before implementing\n- Always read pi .md files completely and follow links to related docs (e.g., tui.md for TUI API details)"
            );
            parts.push(wrap_section("docs", &docs_body));
        }
    }

    if let Some(append) = o.append_system_prompt {
        if !append.is_empty() {
            parts.push(wrap_section("addendum", append));
        }
    }

    if !o.context_files.is_empty() {
        let files: Vec<String> = o
            .context_files
            .iter()
            .map(|f| {
                format!(
                    "<project_instructions path=\"{}\">\n{}\n</project_instructions>",
                    f.path.display(),
                    f.content
                )
            })
            .collect();
        let body = format!(
            "Project-specific instructions and guidelines:\n\n{}",
            files.join("\n\n")
        );
        parts.push(wrap_section("project_context", &body));
    }

    parts.push(wrap_section("cwd", &prompt_cwd));

    parts.join("\n\n")
}

fn wrap_section(name: &str, body: &str) -> String {
    format!("<{name}>\n{body}\n</{name}>")
}

fn build_rules(selected_tools: &[String]) -> String {
    let mut rules: Vec<String> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    fn add(rule: &str, rules: &mut Vec<String>, seen: &mut std::collections::HashSet<String>) {
        let normalized = rule.trim();
        if normalized.is_empty() || seen.contains(normalized) {
            return;
        }
        seen.insert(normalized.to_string());
        rules.push(normalized.to_string());
    }

    let has_bash = selected_tools.iter().any(|t| t == "bash");
    let has_grep = selected_tools.iter().any(|t| t == "grep");
    let has_find = selected_tools.iter().any(|t| t == "find");
    let has_ls = selected_tools.iter().any(|t| t == "ls");

    // 1.4.0 buildRules: the exploration rule exists only when NO read-only
    // search tool is enabled; the 0.74.2 "Prefer grep/find/ls over bash" rule
    // is gone.
    if has_bash && !has_grep && !has_find && !has_ls {
        add("Use bash for file operations like ls, rg, find", &mut rules, &mut seen);
    }

    for name in selected_tools {
        for rule in crate::tools::prompt_guidelines(name).unwrap_or(&[]) {
            add(rule, &mut rules, &mut seen);
        }
    }
    add("Be concise in your responses", &mut rules, &mut seen);
    add("Show file paths clearly when working with files", &mut rules, &mut seen);

    rules.iter().map(|r| format!("- {r}")).collect::<Vec<_>>().join("\n")
}

fn docs_path(rel: &str) -> String {
    // pi prints paths into its installed package. This port resolves against
    // PI_DOC_DIR, falling back to the install prefix.
    let base = std::env::var("PI_DOC_DIR").unwrap_or_else(|_| "/usr/local/share/pi".into());
    format!("{base}/{rel}")
}
