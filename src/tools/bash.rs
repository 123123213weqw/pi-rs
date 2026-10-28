//! bash tool — port of `core/tools/bash.ts` (local shell backend).
//! Tail truncation (last 2000 lines / 50KB), full output saved to a temp file,
//! non-zero exit → tool error with output prepended, messages match pi verbatim.

use super::truncate::{self, DEFAULT_MAX_BYTES, DEFAULT_MAX_LINES};
use serde_json::Value;
use std::io::Read;
use std::process::Stdio;
use std::time::Duration;

pub const DESCRIPTION: &str = "Execute a bash command in the current working directory. Returns stdout and stderr. Output is truncated to last 2000 lines or 50KB (whichever is hit first). If truncated, full output is saved to a temp file. Optionally provide a timeout in seconds.";

pub static PARAMETERS: std::sync::LazyLock<Value> = std::sync::LazyLock::new(|| serde_json::json!({
    "type": "object",
    "required": ["command"],
        "properties": {
        "command": { "type": "string", "description": "Bash command to execute" },
        "timeout": { "type": "number", "description": "Timeout in seconds (optional, no default timeout)" }
    }
}));

pub struct BashOutput {
    pub text: String,
    pub exit_code: i32,
}

unsafe extern "C" {
    fn setsid() -> i32;
    fn kill(pid: i32, sig: i32) -> i32;
}

fn drain<R: Read>(mut r: R) -> Vec<u8> {
    let mut buf = Vec::new();
    let mut tmp = [0u8; 65536];
    loop {
        match r.read(&mut tmp) {
            Ok(0) | Err(_) => break,
            Ok(n) => {
                // Cap buffered output at 8MB; pi streams to a temp file instead,
                // we only need the tail beyond that.
                if buf.len() < 8 * 1024 * 1024 {
                    buf.extend_from_slice(&tmp[..n]);
                }
            }
        }
    }
    buf
}

/// Execute `command` in `cwd`. On timeout the process group is killed and the
/// error message matches pi's `Command timed out after N seconds` (output prepended).
pub fn run(command: &str, cwd: &std::path::Path, timeout_secs: Option<f64>) -> Result<BashOutput, String> {
    if !cwd.exists() {
        return Err(format!(
            "Working directory does not exist: {}\nCannot execute bash commands.",
            cwd.display()
        ));
    }

    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".into());
    let mut cmd = std::process::Command::new(&shell);
    cmd.arg("-c").arg(command).current_dir(cwd).stdin(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        unsafe {
            // New session/process group so the whole tree can be killed (pi: detached).
            cmd.pre_exec(|| {
                unsafe {
                    let _ = setsid();
                }
                Ok(())
            });
        }
    }
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| format!("Failed to spawn shell: {e}"))?;
    let pid = child.id() as i32;

    let stdout = child.stdout.take().expect("stdout");
    let stderr = child.stderr.take().expect("stderr");
    let out_t = std::thread::spawn(move || drain(stdout));
    let err_t = std::thread::spawn(move || drain(stderr));

    let deadline = timeout_secs.map(|s| std::time::Instant::now() + Duration::from_secs_f64(s));
    let mut timed_out = false;
    let status = loop {
        match child.try_wait() {
            Ok(Some(st)) => break Some(st),
            Ok(None) => {
                if let Some(d) = deadline {
                    if std::time::Instant::now() >= d {
                        timed_out = true;
                        unsafe {
                            kill(-pid, 15);
                        }
                        std::thread::sleep(Duration::from_millis(300));
                        unsafe {
                            kill(-pid, 9);
                        }
                        let _ = child.wait();
                        break None;
                    }
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(e) => return Err(format!("Failed to wait for shell: {e}")),
        }
    };

    let out_bytes = out_t.join().unwrap_or_default();
    let err_bytes = err_t.join().unwrap_or_default();
    let mut combined = out_bytes;
    combined.extend_from_slice(&err_bytes);
    let raw = String::from_utf8_lossy(&combined).into_owned();

    let exit_code = status.map(|s| s.code().unwrap_or(-1)).unwrap_or(-1);

    let t = truncate::truncate_tail(&raw, DEFAULT_MAX_LINES, DEFAULT_MAX_BYTES);
    let mut text = t.content.clone();
    if text.is_empty() {
        text = "(no output)".into();
    }
    if t.truncated {
        let full_path = std::env::temp_dir().join(format!("pi-bash-{}.out", crate::util::short_id()));
        let _ = std::fs::write(&full_path, &raw);
        let full = full_path.display().to_string();
        let total = t.total_lines;
        let start = total - t.output_lines + 1;
        let end = total;
        if t.last_line_partial {
            let _ = full;
            text.push_str(&format!(
                "\n\n[Showing last {} of line {end} (line is {}). Full output: {full}]",
                crate::util::format_size(t.output_bytes),
                crate::util::format_size(raw.len()),
            ));
        } else if t.truncated_by == Some("lines") {
            text.push_str(&format!("\n\n[Showing lines {start}-{end} of {total}. Full output: {full}]"));
        } else {
            text.push_str(&format!(
                "\n\n[Showing lines {start}-{end} of {total} ({} limit). Full output: {full}]",
                crate::util::format_size(DEFAULT_MAX_BYTES)
            ));
        }
    }

    if timed_out {
        let secs = timeout_secs.unwrap_or(0.0);
        let secs_fmt = if secs.fract() == 0.0 {
            format!("{}", secs as i64)
        } else {
            format!("{secs}")
        };
        return Err(format!("{text}\n\nCommand timed out after {secs_fmt} seconds"));
    }

    if exit_code != 0 {
        return Err(format!("{text}\n\nCommand exited with code {exit_code}"));
    }

    Ok(BashOutput { text, exit_code })
}
