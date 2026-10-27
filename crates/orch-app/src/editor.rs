//! Open in editor (FR-1101, FR-1102): the editor command template, defaulting
//! to `nvim +{line} {file}` (or kitty.conf's `editor`), runs in an embedded
//! pane. Each project gets one nvim server socket; when it answers, the file
//! opens there (`nvim --server <sock> --remote-send`) instead of a new pane.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use agent_runtime::shell::sh_quote;
use anyhow::{Result, bail};

pub const DEFAULT_TEMPLATE: &str = "nvim +{line} {file}";

/// The template from kitty's `editor` (a bare command gets `+{line} {file}`).
pub fn template_from_kitty(editor: Option<&str>) -> String {
    match editor.map(str::trim).filter(|e| !e.is_empty() && *e != ".") {
        Some(e) if e.contains("{file}") => e.to_string(),
        Some(e) if is_nvim(e) || e.ends_with("vim") || e.ends_with("vi") => format!("{e} +{{line}} {{file}}"),
        Some(e) => format!("{e} {{file}}"),
        None => DEFAULT_TEMPLATE.to_string(),
    }
}

fn program(template: &str) -> &str {
    template.split_whitespace().next().unwrap_or("")
}

pub fn is_nvim(template: &str) -> bool {
    Path::new(program(template)).file_name().is_some_and(|n| n == "nvim")
}

/// The command line for a new editor pane. nvim also listens on the
/// project's socket so later opens reuse it.
pub fn render(template: &str, file: Option<&str>, line: Option<u32>, listen: Option<&Path>) -> String {
    let mut cmd = template.to_string();
    let line = line.unwrap_or(1).max(1);
    cmd = cmd.replace("+{line}", if file.is_some() { "+{line}" } else { "" });
    cmd = cmd.replace("{line}", &line.to_string());
    cmd = cmd.replace("{file}", &file.map(sh_quote).unwrap_or_default());
    let mut cmd = cmd.split_whitespace().collect::<Vec<_>>().join(" ");
    if let (Some(sock), true) = (listen, is_nvim(template)) {
        let prog = program(template).to_string();
        cmd = cmd.replacen(&prog, &format!("{prog} --listen {}", sh_quote(&sock.display().to_string())), 1);
    }
    cmd
}

/// `<home>/nvim/<project>.sock` (short enough for `sun_path`).
pub fn server_socket(home: &Path, project_id: &str) -> PathBuf {
    home.join("nvim").join(format!("{project_id}.sock"))
}

/// Keys that open `file` at `line` in a running nvim, from any mode.
pub fn remote_keys(file: &str, line: Option<u32>) -> String {
    let path = file.replace(' ', r"\ ");
    format!("<C-\\><C-N>:e +{} {path}<CR>", line.unwrap_or(1).max(1))
}

/// nvim's path as the login shell resolves it (bob, Homebrew…), resolved once.
fn nvim_path() -> Option<&'static str> {
    static P: OnceLock<Option<String>> = OnceLock::new();
    P.get_or_init(|| {
        let out =
            Command::new(crate::terms::login_shell()).args(["-l", "-i", "-c", "command -v nvim"]).output().ok()?;
        let s = String::from_utf8_lossy(&out.stdout);
        s.lines().rev().map(str::trim).find(|l| l.starts_with('/')).map(str::to_string)
    })
    .as_deref()
}

/// Opens the file in the project's running nvim. False when no server answers.
pub fn open_remote(sock: &Path, file: &str, line: Option<u32>) -> Result<bool> {
    if !orch_protocol::is_listening(sock) {
        return Ok(false);
    }
    let Some(nvim) = nvim_path() else { bail!("nvim not found in the login shell's PATH") };
    let st = Command::new(nvim).arg("--server").arg(sock).arg("--remote-send").arg(remote_keys(file, line)).status()?;
    Ok(st.success())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn templates() {
        assert_eq!(template_from_kitty(None), DEFAULT_TEMPLATE);
        assert_eq!(template_from_kitty(Some("nvim")), "nvim +{line} {file}");
        assert_eq!(template_from_kitty(Some("/opt/bin/nvim")), "/opt/bin/nvim +{line} {file}");
        assert_eq!(template_from_kitty(Some("code -g {file}")), "code -g {file}");
        assert_eq!(template_from_kitty(Some(".")), DEFAULT_TEMPLATE);
    }

    #[test]
    fn renders_with_quoting_and_listen() {
        let sock = Path::new("/tmp/o/nvim/shop.sock");
        assert_eq!(
            render(DEFAULT_TEMPLATE, Some("src/a b.ts"), Some(72), Some(sock)),
            "nvim --listen /tmp/o/nvim/shop.sock +72 'src/a b.ts'"
        );
        assert_eq!(render(DEFAULT_TEMPLATE, None, None, None), "nvim");
        assert_eq!(render("hx {file}", Some("x.rs"), Some(3), Some(sock)), "hx x.rs", "only nvim listens");
    }

    #[test]
    fn remote_keys_escape_spaces() {
        assert_eq!(remote_keys("/a b/c.rs", Some(9)), r"<C-\><C-N>:e +9 /a\ b/c.rs<CR>");
        assert!(!open_remote(Path::new("/nonexistent/x.sock"), "f", None).unwrap());
    }
}
