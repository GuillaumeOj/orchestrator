//! Running git (and gh) with no prompts, optional deadlines, and path helpers.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};

/// Runs git with no prompts and returns stdout.
pub fn git(dir: &Path, args: &[&str]) -> Result<String> {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .output()
        .with_context(|| format!("git {args:?}"))?;
    check(dir, args, out)
}

fn check(dir: &Path, args: &[&str], out: Output) -> Result<String> {
    if !out.status.success() {
        bail!("git {} in {}: {}", args.join(" "), dir.display(), String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Like [`git`], true/false on the exit status (for `--verify`, `merge-base --is-ancestor`…).
pub fn git_ok(dir: &Path, args: &[&str]) -> bool {
    Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Runs a command and kills it at the deadline (network commands: fetch, gh).
pub fn run_with_deadline(mut cmd: Command, deadline: Duration) -> Result<Output> {
    let mut child = cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;
    // Drain the pipes on threads so a chatty child never blocks on a full pipe.
    let mut so = child.stdout.take();
    let mut se = child.stderr.take();
    let t_out = std::thread::spawn(move || {
        let mut b = vec![];
        if let Some(s) = so.as_mut() {
            let _ = std::io::Read::read_to_end(s, &mut b);
        }
        b
    });
    let t_err = std::thread::spawn(move || {
        let mut b = vec![];
        if let Some(s) = se.as_mut() {
            let _ = std::io::Read::read_to_end(s, &mut b);
        }
        b
    });
    let end = Instant::now() + deadline;
    let status = loop {
        if let Some(st) = child.try_wait()? {
            break st;
        }
        if Instant::now() >= end {
            let _ = child.kill();
            let _ = child.wait();
            let _ = (t_out.join(), t_err.join());
            bail!("timed out after {} s", deadline.as_secs());
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    Ok(Output { status, stdout: t_out.join().unwrap_or_default(), stderr: t_err.join().unwrap_or_default() })
}

/// git with a deadline (fetch, clone, push).
pub fn git_deadline(dir: &Path, args: &[&str], deadline: Duration) -> Result<String> {
    let mut cmd = Command::new("git");
    cmd.arg("-C").arg(dir).args(args).env("GIT_TERMINAL_PROMPT", "0");
    let out = run_with_deadline(cmd, deadline).with_context(|| format!("git {}", args.join(" ")))?;
    check(dir, args, out)
}

/// `canonicalize` when the path exists, otherwise the path as given.
pub fn real(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

/// Agent ids and slugs end up in paths, branch names and env values.
pub fn valid_id(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 64
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        && !s.starts_with('-')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids() {
        assert!(valid_id("solo-fix_1"));
        for bad in ["", "-x", "a/b", "a b", "../x", &"x".repeat(65)] {
            assert!(!valid_id(bad), "{bad}");
        }
    }

    #[test]
    fn deadline_kills() {
        let mut c = Command::new("sleep");
        c.arg("5");
        let t = Instant::now();
        assert!(run_with_deadline(c, Duration::from_millis(200)).is_err());
        assert!(t.elapsed() < Duration::from_secs(2));
        let mut c = Command::new("echo");
        c.arg("hi");
        assert_eq!(run_with_deadline(c, Duration::from_secs(5)).unwrap().stdout, b"hi\n");
    }
}
