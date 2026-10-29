//! Running external programs (spec §4), with errors that name what's
//! missing and what failed.

use std::io::Write;
use std::process::{Command, Output, Stdio};

/// Runs a program and returns its output, or an error naming the program,
/// what it was for, and the end of its stderr.
pub fn run(cmd: &mut Command, purpose: &str) -> Result<Output, String> {
    run_with_input(cmd, purpose, None)
}

pub fn run_with_input(
    cmd: &mut Command,
    purpose: &str,
    input: Option<&[u8]>,
) -> Result<Output, String> {
    let program = cmd.get_program().to_string_lossy().into_owned();
    cmd.stdin(if input.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    })
    .stdout(Stdio::piped())
    .stderr(Stdio::piped());
    let mut child = cmd.spawn().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            format!("{purpose} needs {program}, which isn't on PATH")
        } else {
            format!("{purpose}: can't run {program}: {e}")
        }
    })?;
    if let Some(bytes) = input {
        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(bytes)
                .map_err(|e| format!("{purpose}: can't write to {program}: {e}"))?;
        }
    }
    let out = child
        .wait_with_output()
        .map_err(|e| format!("{purpose}: {program} failed: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let tail: Vec<&str> = err.lines().filter(|l| !l.trim().is_empty()).collect();
        let tail = tail[tail.len().saturating_sub(6)..].join("\n  ");
        let stdout = String::from_utf8_lossy(&out.stdout);
        let extra = if tail.is_empty() {
            stdout.trim().to_string()
        } else {
            tail
        };
        return Err(format!(
            "{purpose}: {program} exited with {}{}",
            out.status,
            if extra.is_empty() {
                String::new()
            } else {
                format!(":\n  {extra}")
            }
        ));
    }
    Ok(out)
}

/// Checks that a program can be started (`<program> -version`-style probes
/// differ, so this only tries to spawn it).
pub fn available(program: &str) -> bool {
    Command::new(program)
        .arg("--help")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok()
}
