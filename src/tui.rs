//! Interactive TUI — pi-style chat transcript + bottom editor, built on
//! ratatui/crossterm. Behavior parity targets pi's interactive mode basics:
//! prompt at bottom, streaming assistant text, tool lines inline, Esc/Ctrl+C
//! aborts the in-flight request (pi: Esc aborts; Ctrl+C-Ctrl+C exits).

use crate::agent::{self, AgentConfig, UiEvent};
use crate::messages::{AssistantMessage, Message};
use crate::session::Session;
use ratatui::crossterm::execute;
use ratatui::{
    crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
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

/// One rendered transcript block.
enum ChatBlock {
    User(String),
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
    scroll: usize,
    generating: bool,
    spinner: usize,
}

impl App {
    fn new() -> App {
        App {
            blocks: Vec::new(),
            editor: String::new(),
            cursor: 0,
            history: VecDeque::new(),
            hist_idx: None,
            hist_draft: String::new(),
            scroll: 0,
            generating: false,
            spinner: 0,
        }
    }

    fn scroll_bottom(&mut self) {
        self.scroll = usize::MAX; // pinned to bottom on next render
    }
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
    let [transcript, editor] = Layout::vertical([Constraint::Min(3), Constraint::Length(3)])
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
    // auto-pin to bottom unless the user scrolled up
    let max_scroll = lines.len().saturating_sub(visible);
    let scroll = if app.scroll == usize::MAX || app.scroll > max_scroll {
        max_scroll
    } else {
        app.scroll
    };
    let view_start = scroll.min(max_scroll);
    let shown: Vec<Line> = lines.into_iter().skip(view_start).collect();

    let para = Paragraph::new(shown);
    f.render_widget(para, transcript);

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
}

fn terminal_init(out: &mut Stdout) -> std::io::Result<()> {
    use ratatui::crossterm::terminal::*;
    enable_raw_mode()?;
    execute!(out, EnterAlternateScreen)?;
    Ok(())
}

fn terminal_restore(out: &mut Stdout) {
    use ratatui::crossterm::terminal::*;
    let _ = disable_raw_mode();
    let _ = execute!(out, LeaveAlternateScreen);
    let _ = out.flush();
}

/// Run the interactive session. Blocks until the user quits.
pub fn run(handle: TuiHandle) -> Result<(), String> {
    let TuiHandle { session, cfg, initial_message } = handle;
    let mut stdout = std::io::stdout();
    terminal_init(&mut stdout).map_err(|e| format!("terminal init: {e}"))?;
    let backend = ratatui::backend::CrosstermBackend::new(stdout);
    let mut terminal = ratatui::Terminal::new(backend).map_err(|e| format!("terminal: {e}"))?;

    let mut app = App::new();
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
                UiEvent::AssistantDelta(d) => match app.blocks.last_mut() {
                    Some(ChatBlock::Assistant { text, .. }) => text.push_str(&d),
                    _ => app.blocks.push(ChatBlock::Assistant { text: d, live: true }),
                },
                UiEvent::ToolCall(desc) => {
                    if let Some(ChatBlock::Assistant { live, .. }) = app.blocks.last_mut() {
                        *live = false;
                    }
                    app.blocks.push(ChatBlock::ToolCall(desc));
                }
                UiEvent::ToolResult { name, ok, text } => {
                    app.blocks.push(ChatBlock::ToolResult { name, ok, text, expanded: false });
                }
                UiEvent::Done(_) | UiEvent::Failed(_) => {
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
            if let Ok(Event::Key(key)) = event::read() {
                if key.kind == KeyEventKind::Press {
                    if app.generating && key.code == KeyCode::Esc {
                        abort.store(true, Ordering::Relaxed);
                        app.blocks.push(ChatBlock::System("aborting…".into()));
                    } else if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                        if app.generating {
                            abort.store(true, Ordering::Relaxed);
                        } else {
                            break;
                        }
                    } else if key.code == KeyCode::Char('d') && key.modifiers.contains(KeyModifiers::CONTROL) && !app.generating {
                        break;
                    } else if !app.generating {
                        match key.code {
                            KeyCode::Enter => {
                                let text = app.editor.trim().to_string();
                                app.editor.clear();
                                app.cursor = 0;
                                if text.is_empty() {
                                    continue;
                                }
                                if text == "/exit" || text == "/quit" {
                                    break;
                                }
                                app.history.push_back(text.clone());
                                app.hist_idx = None;
                                app.blocks.push(ChatBlock::User(text.clone()));
                                app.scroll_bottom();
                                app.generating = true;
                                abort.store(false, Ordering::Relaxed);
                                app.blocks.push(ChatBlock::Assistant { text: String::new(), live: true });
                                let cfg2 = AgentConfig {
                                    model: cfg.model.clone(),
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
                                    let mut events: Vec<UiEvent> = Vec::new();
                                    let _result = agent::run_prompt_cb(
                                        &cfg2, &mut hist, &mut sess, &user_text, abort2, &mut |ev| {
                                            // forward deltas live; batch the rest for ordering
                                            match &ev {
                                                UiEvent::AssistantDelta(_) => {
                                                    let _ = tx2.send(ev);
                                                }
                                                other => events.push(other.clone()),
                                            }
                                        },
                                    );
                                    for ev in events {
                                        let _ = tx2.send(ev);
                                    }
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
                            KeyCode::Char(ch) => {
                                let chars: Vec<char> = app.editor.chars().collect();
                                let c = app.cursor.min(chars.len());
                                app.editor = chars[..c].iter().collect::<String>() + &ch.to_string() + &chars[c..].iter().collect::<String>();
                                app.cursor += 1;
                            }
                            _ => {}
                        }
                    }
                }
            } else if let Ok(Event::Resize(_, _)) = event::read() {
                // redraw on next tick
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
