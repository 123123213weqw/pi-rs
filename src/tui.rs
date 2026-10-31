//! Interactive TUI — pi-style chat transcript + bottom editor, built on
//! ratatui/crossterm. Behavior parity targets pi's interactive mode basics:
//! prompt at bottom, streaming assistant text, tool lines inline, Esc/Ctrl+C
//! aborts the in-flight request (pi: Esc aborts; Ctrl+C-Ctrl+C exits).
//!
//! Also aligned with pi's interactive keys: Ctrl+P / Ctrl+Shift+P cycle the
//! model (scoped to `--models`, else the deepseek catalog; writes a
//! `model_change` session entry, effective next turn), Ctrl+O toggles tool
//! output, mouse wheel and PageUp/PageDown scroll the transcript, and Tab
//! completes `/` commands. A footer shows cwd, model and token usage
//! (pi: pwd + token stats), with a keybinding hint line above it.

use crate::agent::{self, AgentConfig, UiEvent};
use crate::messages::{AssistantMessage, Message};
use crate::provider::{self, Model};
use crate::session::Session;
use ratatui::crossterm::execute;
use ratatui::{
    crossterm::event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers,
        MouseEventKind,
    },
    layout::{Constraint, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Paragraph},
    Frame,
};
use std::collections::VecDeque;
use std::io::{Stdout, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Slash commands Tab completes (pi has a full command registry; this is the
/// subset meaningful without extensions/skills).
const COMMANDS: [&str; 6] = ["/clear", "/exit", "/help", "/model", "/models", "/quit"];

/// One rendered transcript block.
enum ChatBlock {
    User(String),
    /// Reasoning (CoT), live while streaming.
    Thinking { text: String, live: bool },
    /// Assistant text, still streaming when `live` is true.
    Assistant { text: String, live: bool },
    ToolCall(String),
    ToolResult { name: String, ok: bool, text: String, expanded: bool },
    System(String),
    Error(String),
}

struct App {
    blocks: Vec<ChatBlock>,
    editor: String,
    cursor: usize,
    history: VecDeque<String>,
    hist_idx: Option<usize>,
    /// Saved editor line while navigating history.
    hist_draft: String,
    /// Lines scrolled up from the bottom; 0 = pinned to the live tail.
    scroll_from_bottom: usize,
    generating: bool,
    spinner: usize,
    /// Footer: working-directory name and current model label.
    cwd_name: String,
    model_label: String,
    /// Running token totals (pi's footer token stats).
    usage_in: u64,
    usage_out: u64,
    usage_cached: u64,
    /// Ctrl+P cycle list and current position; empty = cycling disabled.
    cycle_list: Vec<Model>,
    cycle_idx: usize,
    /// Tab-completion candidates for the current `/` prefix.
    ac: Option<Vec<String>>,
}

impl App {
    fn new(model_label: String, cwd_name: String, cycle_list: Vec<Model>) -> App {
        App {
            blocks: Vec::new(),
            editor: String::new(),
            cursor: 0,
            history: VecDeque::new(),
            hist_idx: None,
            hist_draft: String::new(),
            scroll_from_bottom: 0,
            generating: false,
            spinner: 0,
            cwd_name,
            model_label,
            usage_in: 0,
            usage_out: 0,
            usage_cached: 0,
            cycle_list,
            cycle_idx: 0,
            ac: None,
        }
    }

    fn scroll_bottom(&mut self) {
        self.scroll_from_bottom = 0; // pinned to bottom on next render
    }
}

/// Longest common prefix of the candidates Tab is choosing between.
fn longest_common_prefix(cands: &[&str]) -> String {
    let mut prefix = String::new();
    let Some(first) = cands.first() else { return prefix };
    'outer: for (i, ch) in first.chars().enumerate() {
        for c in cands {
            if c.chars().nth(i) != Some(ch) {
                break 'outer;
            }
        }
        prefix.push(ch);
    }
    prefix
}

const SPINNER: [char; 4] = ['⠋', '⠙', '⠹', '⠸'];
const TOOL_PREVIEW_LINES: usize = 3;

fn render_block(b: &ChatBlock, area_width: u16) -> Vec<Line<'static>> {
    let wrap = |prefix: &str, text: &str, style: Style| -> Vec<Line<'static>> {
        let usable = (area_width as usize).saturating_sub(prefix.chars().count() + 1).max(8);
        // word-ish wrap: hard wrap by chars keeps it simple and correct for CJK
        let mut lines = Vec::new();
        let mut first = true;
        for raw in text.split('\n') {
            let mut chunk = String::new();
            for ch in raw.chars() {
                if chunk.chars().count() >= usable {
                    if first {
                        lines.push(Line::from(vec![Span::styled(prefix.to_string(), Style::default().fg(Color::Green), ), Span::styled(chunk.clone(), style)]));
                        first = false;
                    } else {
                        lines.push(Line::from(Span::styled(chunk.clone(), style)));
                    }
                    chunk.clear();
                }
                chunk.push(ch);
            }
            if first {
                lines.push(Line::from(vec![
                    Span::styled(prefix.to_string(), Style::default().fg(Color::Green)),
                    Span::styled(chunk.clone(), style),
                ]));
                first = false;
            } else {
                lines.push(Line::from(Span::styled(chunk.clone(), style)));
            }
        }
        lines
    };

    match b {
        ChatBlock::User(t) => wrap("❯ ", t, Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
        ChatBlock::Thinking { text, live } => {
            let dim = Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC);
            let mut lines = wrap("◇ ", text, dim);
            if *live {
                lines.push(Line::from(Span::styled(
                    "  · thinking…",
                    Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC),
                )));
            }
            lines
        }
        ChatBlock::Assistant { text, live } => {
            let mut lines = wrap("◆ ", text, Style::default().fg(Color::Gray));
            if *live {
                lines.push(Line::from(Span::styled(
                    "  …generating (Esc to abort)",
                    Style::default().fg(Color::DarkGray),
                )));
            }
            lines
        }
        ChatBlock::ToolCall(t) => wrap("$ ", t, Style::default().fg(Color::Yellow)),
        ChatBlock::ToolResult { name, ok, text, expanded } => {
            let style = Style::default().fg(if *ok { Color::DarkGray } else { Color::Red });
            if *expanded {
                wrap("⏷ ", &format!("{name}: {text}"), style)
            } else {
                let total: usize = text.split('\n').count();
                let preview: Vec<&str> = text.split('\n').take(TOOL_PREVIEW_LINES).collect();
                let mut joined = preview.join("\n");
                if total > TOOL_PREVIEW_LINES {
                    joined.push_str(&format!("\n… ({total} lines, ctrl+o expands latest)"));
                }
                wrap("⏷ ", &format!("{name}: {joined}"), style)
            }
        }
        ChatBlock::System(t) => wrap("· ", t, Style::default().fg(Color::DarkGray)),
        ChatBlock::Error(t) => wrap("✗ ", t, Style::default().fg(Color::Red)),
    }
}

fn draw(f: &mut Frame, app: &App) {
    let [transcript, hint, footer, editor] = Layout::vertical([
        Constraint::Min(3),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(3),
    ])
    .areas(f.size());
    let width = transcript.width;

    let mut lines: Vec<Line> = Vec::new();
    for b in &app.blocks {
        lines.extend(render_block(b, width));
        lines.push(Line::from(""));
    }
    if app.generating {
        lines.push(Line::from(Span::styled(
            format!("{} working…", SPINNER[app.spinner % SPINNER.len()]),
            Style::default().fg(Color::Cyan),
        )));
    }

    let visible = transcript.height as usize;
    // 0 = pinned to the live tail; otherwise that many lines above the bottom.
    let max_scroll = lines.len().saturating_sub(visible);
    let view_start = max_scroll.saturating_sub(app.scroll_from_bottom.min(max_scroll));
    let shown: Vec<Line> = lines.into_iter().skip(view_start).collect();

    let para = Paragraph::new(shown);
    f.render_widget(para, transcript);

    // hint line: Tab-completion candidates, or the standing keybinding hints
    let hint_line = match &app.ac {
        Some(cands) => Line::from(Span::styled(
            format!("  {}", cands.join("   ")),
            Style::default().fg(Color::Cyan),
        )),
        None => Line::from(Span::styled(
            " ctrl+p / ctrl+shift+p model · pgup/pgdn scroll · tab completes /cmds",
            Style::default().fg(Color::DarkGray),
        )),
    };
    f.render_widget(Paragraph::new(hint_line), hint);

    // footer: pwd + model (left), token stats (right) — pi shows pwd, token and context usage
    let footer_line = Line::from(vec![
        Span::styled(
            format!(" {} · {}", app.cwd_name, app.model_label),
            Style::default().fg(Color::Cyan),
        ),
        Span::styled(
            format!(
                "   in {} · out {} · cached {}",
                app.usage_in, app.usage_out, app.usage_cached
            ),
            Style::default().fg(Color::DarkGray),
        ),
    ]);
    f.render_widget(Paragraph::new(footer_line), footer);

    let cursor_style = Style::default().bg(Color::Green).fg(Color::Black);
    let (before, at, after) = {
        let chars: Vec<char> = app.editor.chars().collect();
        let c = app.cursor.min(chars.len());
        let after = if c < chars.len() {
            chars[c + 1..].iter().collect::<String>()
        } else {
            String::new()
        };
        (
            chars[..c].iter().collect::<String>(),
            chars.get(c).map(|x| x.to_string()).unwrap_or_else(|| " ".to_string()),
            after,
        )
    };
    let editor_line = Line::from(vec![
        Span::styled("❯ ", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
        Span::raw(before),
        Span::styled(at, cursor_style),
        Span::raw(after),
    ]);
    let editor_widget = Paragraph::new(vec![editor_line]).block(
        ratatui::widgets::Block::default()
            .borders(Borders::TOP)
            .border_style(Style::default().fg(Color::DarkGray)),
    );
    f.render_widget(editor_widget, editor);
}

pub struct TuiHandle {
    pub session: Arc<Mutex<Session>>,
    pub cfg: AgentConfig,
    pub initial_message: Option<String>,
    /// `--models` scoped list for Ctrl+P cycling (pi's scoped model cycling).
    pub cycle_models: Vec<String>,
}

fn terminal_init(out: &mut Stdout) -> std::io::Result<()> {
    use ratatui::crossterm::terminal::*;
    enable_raw_mode()?;
    execute!(out, EnterAlternateScreen, EnableMouseCapture)?;
    Ok(())
}

fn terminal_restore(out: &mut Stdout) {
    use ratatui::crossterm::terminal::*;
    let _ = disable_raw_mode();
    let _ = execute!(out, DisableMouseCapture, LeaveAlternateScreen);
    let _ = out.flush();
}

/// Run the interactive session. Blocks until the user quits.
pub fn run(handle: TuiHandle) -> Result<(), String> {
    let TuiHandle { session, cfg, initial_message, cycle_models } = handle;
    let mut stdout = std::io::stdout();
    terminal_init(&mut stdout).map_err(|e| format!("terminal init: {e}"))?;
    let backend = ratatui::backend::CrosstermBackend::new(stdout);
    let mut terminal = ratatui::Terminal::new(backend).map_err(|e| format!("terminal: {e}"))?;

    // Ctrl+P cycle list: scoped `--models` patterns, else the deepseek catalog.
    // A single-entry list is not worth cycling, so it is dropped.
    let mut cycle_list: Vec<Model> = Vec::new();
    for pat in &cycle_models {
        if let Ok(m) = provider::resolve_model(None, Some(pat)) {
            if !cycle_list.iter().any(|x| x.provider == m.provider && x.id == m.id) {
                cycle_list.push(m);
            }
        }
    }
    if cycle_list.is_empty() && cfg.model.provider == "deepseek" {
        cycle_list = provider::list_models();
    }
    if !cycle_list.iter().any(|m| m.provider == cfg.model.provider && m.id == cfg.model.id) {
        cycle_list.insert(0, cfg.model.clone());
    }
    if cycle_list.len() < 2 {
        cycle_list.clear();
    }
    let mut cycle_idx = 0;
    let mut cur_model = cfg.model.clone();

    let cwd_name = cfg
        .cwd
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| cfg.cwd.display().to_string());
    let mut app = App::new(format!("{}/{}", cur_model.provider, cur_model.id), cwd_name, cycle_list);
    app.blocks.push(ChatBlock::System(format!(
        "pi-rs · model {} @ {} · /exit or ctrl+d quits · esc aborts",
        cfg.model.id, cfg.model.provider
    )));
    if !cfg.tools.is_empty() {
        app.blocks.push(ChatBlock::System(format!(
            "tools: {}",
            cfg.tools.iter().map(|t| t.name).collect::<Vec<_>>().join(", ")
        )));
    }

    let mut history_msgs: Vec<Message> = Vec::new();
    let abort = Arc::new(AtomicBool::new(false));
    let (tx, rx) = std::sync::mpsc::channel::<UiEvent>();
    let (hist_tx, hist_rx) = std::sync::mpsc::channel::<Vec<Message>>();
    let mut worker: Option<std::thread::JoinHandle<()>> = None;

    // pi sends an initial message (from argv) before the interactive loop
    let mut queued_initial = initial_message;

    loop {
        // drain UI events from any running worker
        while let Ok(ev) = rx.try_recv() {
            match ev {
                UiEvent::ThinkingDelta(d) => match app.blocks.last_mut() {
                    Some(ChatBlock::Thinking { text, .. }) => text.push_str(&d),
                    _ => {
                        if let Some(ChatBlock::Assistant { live, .. }) = app.blocks.last_mut() {
                            *live = false;
                        }
                        app.blocks.push(ChatBlock::Thinking { text: d, live: true });
                    }
                },
                UiEvent::AssistantDelta(d) => {
                    if let Some(ChatBlock::Thinking { live, .. }) = app.blocks.last_mut() {
                        *live = false;
                    }
                    match app.blocks.last_mut() {
                        Some(ChatBlock::Assistant { text, .. }) => text.push_str(&d),
                        _ => app.blocks.push(ChatBlock::Assistant { text: d, live: true }),
                    }
                }
                UiEvent::ToolCall(desc) => {
                    if let Some(ChatBlock::Thinking { live, .. }) = app.blocks.last_mut() {
                        *live = false;
                    }
                    if let Some(ChatBlock::Assistant { live, .. }) = app.blocks.last_mut() {
                        *live = false;
                    }
                    app.blocks.push(ChatBlock::ToolCall(desc));
                }
                UiEvent::ToolResult { name, ok, text } => {
                    app.blocks.push(ChatBlock::ToolResult { name, ok, text, expanded: false });
                }
                UiEvent::Done(_) | UiEvent::Failed(_) => {
                    if let UiEvent::Done(msg) = &ev {
                        app.usage_in += msg.usage.input;
                        app.usage_out += msg.usage.output;
                        app.usage_cached += msg.usage.cache_read;
                    }
                    if let Some(ChatBlock::Thinking { live, .. }) = app.blocks.last_mut() {
                        *live = false;
                    }
                    if let Some(ChatBlock::Assistant { live, .. }) = app.blocks.last_mut() {
                        *live = false;
                    }
                    app.generating = false;
                    if let Some(h) = worker.take() {
                        let _ = h.join();
                        // worker shipped the full updated conversation before exiting
                        if let Ok(updated) = hist_rx.try_recv() {
                            history_msgs = updated;
                        }
                    }
                    if let UiEvent::Failed(e) = ev {
                        app.blocks.push(ChatBlock::Error(e));
                    }
                    app.scroll_bottom();
                }
            }
        }

        // poll terminal input (non-blocking tick)
        if event::poll(Duration::from_millis(50)).unwrap_or(false) {
            match event::read() {
                Ok(Event::Key(key)) if key.kind == KeyEventKind::Press => {
                    app.ac = None; // any key refreshes completion; Tab repopulates
                    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
                    if app.generating && key.code == KeyCode::Esc {
                        abort.store(true, Ordering::Relaxed);
                        app.blocks.push(ChatBlock::System("aborting…".into()));
                    } else if ctrl && key.code == KeyCode::Char('c') {
                        if app.generating {
                            abort.store(true, Ordering::Relaxed);
                        } else {
                            break;
                        }
                    } else if ctrl && key.code == KeyCode::Char('o') {
                        for b in app.blocks.iter_mut() {
                            if let ChatBlock::ToolResult { expanded, .. } = b {
                                *expanded = !*expanded;
                            }
                        }
                    } else if ctrl && key.code == KeyCode::Char('d') && !app.generating {
                        break;
                    } else if ctrl && (key.code == KeyCode::Char('p') || key.code == KeyCode::Char('P')) {
                        // pi: ctrl+p / ctrl+shift+p cycle models; applies to the next request
                        if app.cycle_list.len() > 1 {
                            let len = app.cycle_list.len();
                            cycle_idx = if key.code == KeyCode::Char('P') {
                                (cycle_idx + len - 1) % len
                            } else {
                                (cycle_idx + 1) % len
                            };
                            cur_model = app.cycle_list[cycle_idx].clone();
                            app.model_label = format!("{}/{}", cur_model.provider, cur_model.id);
                            session
                                .lock()
                                .unwrap()
                                .append_model_change(&cur_model.provider, &cur_model.id);
                            app.blocks
                                .push(ChatBlock::System(format!("model → {} (next request)", app.model_label)));
                            app.scroll_bottom();
                        }
                    } else if key.code == KeyCode::PageUp {
                        app.scroll_from_bottom = app.scroll_from_bottom.saturating_add(10);
                    } else if key.code == KeyCode::PageDown {
                        app.scroll_from_bottom = app.scroll_from_bottom.saturating_sub(10);
                    } else if !app.generating {
                        match key.code {
                            KeyCode::Enter => {
                                let text = app.editor.trim().to_string();
                                app.editor.clear();
                                app.cursor = 0;
                                if text.is_empty() {
                                    continue;
                                }
                                if text.starts_with('/') {
                                    match text.as_str() {
                                        "/exit" | "/quit" => break,
                                        "/clear" => app.blocks.clear(),
                                        "/model" | "/models" => {
                                            let list = app
                                                .cycle_list
                                                .iter()
                                                .map(|m| format!("{}/{}", m.provider, m.id))
                                                .collect::<Vec<_>>()
                                                .join(", ");
                                            let msg = if list.is_empty() {
                                                format!("model {} · no cycle list (pass --models a,b,c)", app.model_label)
                                            } else {
                                                format!("model {} · ctrl+p cycles: {}", app.model_label, list)
                                            };
                                            app.blocks.push(ChatBlock::System(msg));
                                        }
                                        "/help" => app.blocks.push(ChatBlock::System(
                                            "enter send · up/down history · tab completes /clear /exit /help /model /quit · \
                                             ctrl+p / ctrl+shift+p cycle model · ctrl+o expand tool output · \
                                             pgup/pgdn + wheel scroll · esc abort · ctrl+c ctrl+c / ctrl+d / /exit quit"
                                                .into(),
                                        )),
                                        other => app.blocks.push(ChatBlock::Error(format!(
                                            "unknown command {other} (tab completes: {})",
                                            COMMANDS.join(" ")
                                        ))),
                                    }
                                    app.scroll_bottom();
                                    continue;
                                }
                                app.history.push_back(text.clone());
                                app.hist_idx = None;
                                app.blocks.push(ChatBlock::User(text.clone()));
                                app.scroll_bottom();
                                app.generating = true;
                                abort.store(false, Ordering::Relaxed);
                                let cfg2 = AgentConfig {
                                    model: cur_model.clone(),
                                    api_key: cfg.api_key.clone(),
                                    system_prompt: cfg.system_prompt.clone(),
                                    tools: cfg.tools.clone(),
                                    cwd: cfg.cwd.clone(),
                                    verbose: cfg.verbose,
                                };
                                let hist = history_msgs.clone();
                                let sess = session.clone();
                                let abort2 = abort.clone();
                                let tx2 = tx.clone();
                                let htx = hist_tx.clone();
                                let mut user_text = text.clone();
                                if let Some(initial) = queued_initial.take() {
                                    user_text = format!("{initial}\n\n{text}");
                                }
                                worker = Some(std::thread::spawn(move || {
                                    let mut hist = hist;
                                    let mut sess = sess.lock().unwrap();
                                    // forward every event in emission order (mpsc preserves it)
                                    let _ = agent::run_prompt_cb(
                                        &cfg2, &mut hist, &mut sess, &user_text, abort2, &mut |ev| {
                                            let _ = tx2.send(ev);
                                        },
                                    );
                                    let _ = htx.send(hist);
                                }));
                                history_msgs.clear(); // rebuilt from hist_rx after join
                            }
                            KeyCode::Up => {
                                if app.history.is_empty() { continue; }
                                let idx = match app.hist_idx {
                                    None => {
                                        app.hist_draft = app.editor.clone();
                                        app.history.len() - 1
                                    }
                                    Some(i) => i.saturating_sub(1),
                                };
                                app.hist_idx = Some(idx);
                                app.editor = app.history[idx].clone();
                                app.cursor = app.editor.chars().count();
                            }
                            KeyCode::Down => {
                                if let Some(i) = app.hist_idx {
                                    if i + 1 >= app.history.len() {
                                        app.hist_idx = None;
                                        app.editor = app.hist_draft.clone();
                                    } else {
                                        app.editor = app.history[i + 1].clone();
                                        app.hist_idx = Some(i + 1);
                                    }
                                    app.cursor = app.editor.chars().count();
                                }
                            }
                            KeyCode::Left => { app.cursor = app.cursor.saturating_sub(1); }
                            KeyCode::Right => {
                                let len = app.editor.chars().count();
                                if app.cursor < len { app.cursor += 1; }
                            }
                            KeyCode::Backspace => {
                                if app.cursor > 0 {
                                    let chars: Vec<char> = app.editor.chars().collect();
                                    let c = (app.cursor - 1).min(chars.len());
                                    app.editor = chars[..c].iter().collect::<String>() + &chars[c + 1..].iter().collect::<String>();
                                    app.cursor = c;
                                }
                            }
                            KeyCode::Delete => {
                                let chars: Vec<char> = app.editor.chars().collect();
                                let c = app.cursor.min(chars.len());
                                if c < chars.len() {
                                    app.editor = chars[..c].iter().collect::<String>() + &chars[c + 1..].iter().collect::<String>();
                                }
                            }
                            KeyCode::Home => app.cursor = 0,
                            KeyCode::End => app.cursor = app.editor.chars().count(),
                            KeyCode::Tab => {
                                if app.editor.starts_with('/') && !app.editor.contains(' ') {
                                    let prefix = app.editor.clone();
                                    let cands: Vec<&str> = COMMANDS
                                        .iter()
                                        .copied()
                                        .filter(|c| c.starts_with(&prefix))
                                        .collect();
                                    if cands.len() == 1 {
                                        app.editor = cands[0].to_string();
                                        app.cursor = app.editor.chars().count();
                                    } else if cands.len() > 1 {
                                        app.editor = longest_common_prefix(&cands);
                                        app.cursor = app.editor.chars().count();
                                        app.ac = Some(cands.iter().map(|s| s.to_string()).collect());
                                    }
                                }
                            }
                            KeyCode::Char(ch)
                                if !key.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
                            {
                                let chars: Vec<char> = app.editor.chars().collect();
                                let c = app.cursor.min(chars.len());
                                app.editor = chars[..c].iter().collect::<String>() + &ch.to_string() + &chars[c..].iter().collect::<String>();
                                app.cursor += 1;
                            }
                            _ => {}
                        }
                    }
                }
                Ok(Event::Mouse(me)) => match me.kind {
                    // pi's wheel scroll: a notch is a small step; 3 lines keeps it visible
                    MouseEventKind::ScrollUp => {
                        app.scroll_from_bottom = app.scroll_from_bottom.saturating_add(3);
                    }
                    MouseEventKind::ScrollDown => {
                        app.scroll_from_bottom = app.scroll_from_bottom.saturating_sub(3);
                    }
                    _ => {}
                },
                Ok(Event::Resize(_, _)) => {
                    // redraw on next tick
                }
                _ => {}
            }
        }

        app.spinner += 1;
        terminal
            .draw(|f| draw(f, &app))
            .map_err(|e| format!("draw: {e}"))?;
    }

    // wait for in-flight worker to finish writing the session file
    if let Some(h) = worker.take() {
        abort.store(true, Ordering::Relaxed);
        let _ = h.join();
    }
    terminal_restore(&mut std::io::stdout());
    println!("session saved.");
    Ok(())
}
