//! Scripting hooks per spec/behavior.md §8.

use crate::types::Hook;
use std::io::Write;
use std::process::{Command, Stdio};
use std::time::Duration;

/// Parse one UI line `event=command` (split at the FIRST `=`, both sides trimmed).
pub fn parse_hook_line(line: &str) -> Option<Hook> {
    let eq = line.find('=')?;
    if eq == 0 {
        return None;
    }
    let event = line[..eq].trim();
    let command = line[eq + 1..].trim();
    if (event == "copy" || event == "paste") && !command.is_empty() {
        Some(Hook { event: event.to_string(), command: command.to_string() })
    } else {
        None
    }
}

/// Run all hooks for `event` concurrently, feeding `content` to each stdin.
/// Failures are silent; each command is killed after 5 s.
pub fn run_hooks(event: &str, content: &str, hooks: &[Hook]) {
    let mut children = Vec::new();
    for hook in hooks.iter().filter(|h| h.event == event && !h.command.trim().is_empty()) {
        if let Ok(mut child) = Command::new("/bin/sh")
            .arg("-c")
            .arg(&hook.command)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            if let Some(stdin) = child.stdin.as_mut() {
                let _ = stdin.write_all(content.as_bytes());
                let _ = stdin.flush();
            }
            child.stdin.take();
            children.push(child);
        }
    }
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    for mut child in children {
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) => {
                    if std::time::Instant::now() >= deadline {
                        let _ = child.kill();
                        let _ = child.wait();
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(_) => break,
            }
        }
    }
}
