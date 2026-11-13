//! Opt-in contract checks that use no plan quota: the `claude` flags the app
//! passes (taken from `compile`, the source of truth) are still in `claude --help`,
//! and `claude auth status --json` still has the fields the app reads.
//!
//! ```bash
//! ORCH_LIVE_CLAUDE=1 cargo test -p agent-runtime --test contract_cli -- --ignored --nocapture
//! ```
//!
//! `ORCH_LIVE_CLAUDE_BIN=<path>` tests another `claude`. See docs/app/CONTRACT-TESTS.md
//! (or run `make contract`, which runs this and the live checks).

use std::path::{Path, PathBuf};
use std::process::Command;

use agent_runtime::compile::{CompileCtx, HelperPaths, SessionMode, compile};
use agent_runtime::env::EnvInputs;
use agent_runtime::role::{PermissionMode, Role};

fn claude() -> Option<PathBuf> {
    if std::env::var("ORCH_LIVE_CLAUDE").as_deref() != Ok("1") {
        eprintln!("skipped: set ORCH_LIVE_CLAUDE=1");
        return None;
    }
    Some(std::env::var_os("ORCH_LIVE_CLAUDE_BIN").map(PathBuf::from).unwrap_or_else(|| "claude".into()))
}

/// `(stdout, stderr)` of `claude <args>`.
fn run_split(claude: &Path, args: &[&str]) -> (String, String) {
    let o = Command::new(claude)
        .args(args)
        .env("DISABLE_AUTOUPDATER", "1")
        .output()
        .unwrap_or_else(|e| panic!("cannot run {} {}: {e}", claude.display(), args.join(" ")));
    (String::from_utf8_lossy(&o.stdout).into_owned(), String::from_utf8_lossy(&o.stderr).into_owned())
}

/// stdout then stderr (the help may go to either).
fn run(claude: &Path, args: &[&str]) -> String {
    let (out, err) = run_split(claude, args);
    out + &err
}

/// A role that makes `compile` pass every optional flag, for each session mode.
fn app_args(mode: PermissionMode, session: SessionMode) -> Vec<String> {
    let mut role = Role::solo();
    role.model = Some("haiku".into());
    role.effort = Some("low".into());
    role.strict_mcp = true;
    role.setting_sources = Some(vec!["project".into(), "local".into()]);
    role.permission_mode = mode;
    role.prompt = Some("p".into());
    let mut env = EnvInputs::from_process("/usr/bin:/bin".into());
    env.home = "/Users/u".into();
    let ctx = CompileCtx {
        agent_id: "a1".into(),
        name: "n".into(),
        agent_dir: "/app/agents/a1".into(),
        app_root: "/app".into(),
        worktree: "/p/w".into(),
        main_checkout: "/p".into(),
        main_read_denies: vec![],
        git_common_dir: "/p/.git".into(),
        other_worktrees: vec![],
        helpers: HelperPaths::in_dir(Path::new("/app/bin")),
        claude: "claude".into(),
        session,
        env,
        git_identity: ("N".into(), "e@x".into()),
        project_files: vec![],
        overwatch: None,
    };
    compile(&role, &ctx).args
}

#[test]
#[ignore = "needs a real claude: ORCH_LIVE_CLAUDE=1 … -- --ignored (no quota)"]
fn every_flag_the_app_passes_is_in_claude_help() {
    let Some(claude) = claude() else { return };
    let help = run(&claude, &["--help"]);
    // Only the option column of each line (`  -r, --resume [value]   description`): a flag
    // merely mentioned in another option's description must not count as listed.
    let words: Vec<&str> = help
        .lines()
        .map(str::trim)
        .filter(|l| l.starts_with('-'))
        .flat_map(|l| l.split("  ").next().unwrap_or(l).split(|c: char| c.is_whitespace() || c == ',' || c == '<'))
        .collect();
    let mut flags: Vec<String> =
        [SessionMode::New("s".into()), SessionMode::Resume("s".into()), SessionMode::Fork("s".into())]
            .into_iter()
            .flat_map(|m| app_args(PermissionMode::Auto, m))
            .filter(|a| a.starts_with('-'))
            .collect();
    flags.sort();
    flags.dedup();
    assert!(flags.len() >= 10, "expected the app's flags from compile.rs, got {flags:?}");
    let missing: Vec<&String> = flags.iter().filter(|f| !words.contains(&f.as_str())).collect();
    assert!(
        missing.is_empty(),
        "claude --help no longer lists {missing:?}: the app passes these in compile.rs (§6.3); see docs/app/CONTRACT-TESTS.md"
    );
}

#[test]
#[ignore = "needs a real claude: ORCH_LIVE_CLAUDE=1 … -- --ignored (no quota)"]
fn every_permission_mode_the_app_uses_is_listed() {
    let Some(claude) = claude() else { return };
    // The help wraps its list of choices over lines: compare on single spaces.
    let help = run(&claude, &["--help"]).split_whitespace().collect::<Vec<_>>().join(" ");
    let at = help.find("--permission-mode").expect("claude --help no longer has --permission-mode");
    // Up to the closing parenthesis of `(choices: …)`.
    let section = help[at..].split(')').next().unwrap_or_default().to_string();
    for mode in [
        PermissionMode::Manual,
        PermissionMode::AcceptEdits,
        PermissionMode::Auto,
        PermissionMode::DontAsk,
        PermissionMode::Plan,
    ] {
        let flag = mode.as_flag();
        let args = app_args(mode, SessionMode::New("s".into()));
        let i = args.iter().position(|a| a == "--permission-mode").expect("compile always passes --permission-mode");
        assert_eq!(args[i + 1], flag);
        assert!(
            section.contains(&format!("\"{flag}\"")),
            "claude --permission-mode no longer accepts {flag:?} (the app passes it for the {mode:?} mode). Help says: {section}"
        );
    }
}

#[test]
#[ignore = "needs a real claude: ORCH_LIVE_CLAUDE=1 … -- --ignored (no quota)"]
fn auth_status_json_has_the_fields_the_app_reads() {
    let Some(claude) = claude() else { return };
    // stdout only: a warning on stderr must not break the JSON.
    let (out, err) = run_split(&claude, &["auth", "status", "--json"]);
    let v: serde_json::Value = serde_json::from_str(out.trim())
        .unwrap_or_else(|e| panic!("claude auth status --json no longer prints JSON ({e}): {out}{err}"));
    for field in ["loggedIn", "authMethod"] {
        assert!(v.get(field).is_some(), "claude auth status --json lost the `{field}` field: {v}");
    }
    // `subscriptionType` is only there when logged in.
    assert!(v["loggedIn"] == true, "claude is not logged in: log in with Pro/Max, then run the contract tests again");
    assert!(v.get("subscriptionType").is_some(), "claude auth status --json lost the `subscriptionType` field: {v}");
}
