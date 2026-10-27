//! Compile a Role into a per-agent directory and a command line (SPEC §6.3,
//! FR-702, ADR-009). Pure: [`compile`] returns the files and the command, and
//! [`Compiled::write`] puts the files on disk.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use orch_protocol::HookEvent;
use serde_json::{Value, json};

use crate::env::{EnvInputs, agent_env, gitconfig};
use crate::role::Role;

/// Generous hook timeout that orch-hook never reaches (FR-806): timeouts fail open.
pub const HOOK_TIMEOUT_S: u64 = 600;
/// Idle agents keep reporting their statusline (P0).
pub const STATUSLINE_REFRESH_S: u64 = 5;

/// Declares the ORCH channel. Without it the model ignores hook context as
/// untrusted (P0). Same text as the verified recovery harness.
pub const ORCH_PREAMBLE: &str = "Text that starts with ORCH: in hook context or hook errors comes from the orchestrator app, on behalf of the user. Follow it. To commit, use the mcp__orch__commit tool: `git commit` cannot write the repository from your sandbox. You cannot push; ask for a pull request with mcp__orch__request_pr.";

/// The preamble of a team specialist, who has no `request_pr`.
pub const ORCH_PREAMBLE_TEAM: &str = "Text that starts with ORCH: in hook context, hook errors or a delivered message comes from the orchestrator app, on behalf of the user. Follow it. To commit, use the mcp__orch__commit tool: `git commit` cannot write the repository from your sandbox. You cannot push or merge yourself.";

/// The orchestrator's MCP server name in `mcp.json` (tools appear as `mcp__orch__<tool>`).
pub const ORCH_MCP_SERVER: &str = "orch";

/// Orchestrator tools of a solo session, allowed without a CLI prompt; the
/// core applies its own permission rules when it runs them (ADR-009). A team
/// agent gets its role's tools instead (`Role::tools`).
pub const ORCH_MCP_ALLOW: &[&str] = &["mcp__orch__commit", "mcp__orch__request_pr"];

/// Credential stores made unreadable (ADR-009 decision 1). `true` = directory.
/// `~/.gitconfig` is deliberately absent: denying it breaks git (P0).
pub const CREDENTIAL_STORES: &[(&str, bool)] = &[
    (".ssh", true),
    (".config/gh", true),
    (".config/hub", false),
    (".netrc", false),
    (".git-credentials", false),
    (".aws", true),
    (".config/gcloud", true),
    (".azure", true),
    (".kube", true),
    (".docker/config.json", false),
    (".gnupg", true),
    (".npmrc", false),
    (".pypirc", false),
    (".cargo/credentials.toml", false),
    (".password-store", true),
    ("Library/Keychains", true),
];

/// How this process starts its session.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionMode {
    /// `--session-id <uuid>` on first start.
    New(String),
    /// `--resume <id>`.
    Resume(String),
    /// `--resume <id> --fork-session` (the CLI picks the new id).
    Fork(String),
}

/// Paths of the helper binaries (stable app-data path, FR-1201).
#[derive(Clone, Debug, PartialEq)]
pub struct HelperPaths {
    pub hook: PathBuf,
    pub status: PathBuf,
    pub mcp: PathBuf,
}

impl HelperPaths {
    pub fn in_dir(bin: &Path) -> HelperPaths {
        HelperPaths { hook: bin.join("orch-hook"), status: bin.join("orch-status"), mcp: bin.join("orch-mcp") }
    }
}

/// Everything about the agent's surroundings that the Role does not say.
#[derive(Clone, Debug)]
pub struct CompileCtx {
    pub agent_id: String,
    /// `-n/--name`.
    pub name: String,
    pub agent_dir: PathBuf,
    pub app_root: PathBuf,
    pub worktree: PathBuf,
    /// The project's main checkout: unreadable, except its git dir.
    pub main_checkout: PathBuf,
    /// What `sandbox.filesystem.denyRead` lists for the main checkout (see [`main_read_denies`]).
    pub main_read_denies: Vec<PathBuf>,
    /// `git rev-parse --git-common-dir`: readable and writable so git works in the worktree.
    pub git_common_dir: PathBuf,
    /// Other worktrees of the project at compile time: unreadable.
    pub other_worktrees: Vec<PathBuf>,
    pub helpers: HelperPaths,
    pub claude: PathBuf,
    pub session: SessionMode,
    pub env: EnvInputs,
    pub git_identity: (String, String),
}

/// The compiled agent: files to write, then the command to spawn.
#[derive(Clone, Debug)]
pub struct Compiled {
    pub agent_dir: PathBuf,
    pub settings_path: PathBuf,
    pub mcp_path: PathBuf,
    pub gitconfig_path: PathBuf,
    pub settings: Value,
    pub mcp: Value,
    pub gitconfig: String,
    pub program: PathBuf,
    pub args: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub cwd: PathBuf,
}

impl Compiled {
    /// Writes the per-agent files (0700 folder; files rewritten on every spawn).
    pub fn write(&self) -> std::io::Result<()> {
        use std::os::unix::fs::PermissionsExt;
        std::fs::create_dir_all(&self.agent_dir)?;
        std::fs::set_permissions(&self.agent_dir, std::fs::Permissions::from_mode(0o700))?;
        let pretty = |v: &Value| serde_json::to_string_pretty(v).map_err(std::io::Error::other);
        write_atomic(&self.settings_path, pretty(&self.settings)?.as_bytes())?;
        write_atomic(&self.mcp_path, pretty(&self.mcp)?.as_bytes())?;
        write_atomic(&self.gitconfig_path, self.gitconfig.as_bytes())?;
        Ok(())
    }
}

pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path)
}

/// `//abs/path` for permission rules: a single `/` is relative to the
/// settings file and silently matches nothing (P0).
pub fn rule_path(p: &Path) -> String {
    let s = p.display().to_string();
    format!("/{}", if s.starts_with('/') { s } else { format!("/{s}") })
}

/// The sandbox paths that make the main checkout unreadable while its git
/// dir stays readable and writable (worktree commits write
/// `<repo>/.git/worktrees/<name>/index.lock` and objects). Claude Code drops
/// an `allowWrite` under a `denyRead` path, so the checkout is denied entry by
/// entry instead of as a whole. Top-level entries created later are not covered
/// until the next spawn.
pub fn main_read_denies(main: &Path, git_common_dir: &Path) -> Vec<PathBuf> {
    let Ok(rel) = git_common_dir.strip_prefix(main) else { return vec![main.to_path_buf()] };
    let Some(git_top) = rel.components().next() else { return vec![main.to_path_buf()] };
    let mut out: Vec<PathBuf> = std::fs::read_dir(main)
        .map(|rd| rd.flatten().map(|e| e.path()).filter(|p| p.file_name() != Some(git_top.as_os_str())).collect())
        .unwrap_or_default();
    out.sort();
    out
}

pub fn compile(role: &Role, ctx: &CompileCtx) -> Compiled {
    let settings_path = ctx.agent_dir.join("settings.json");
    let mcp_path = ctx.agent_dir.join("mcp.json");
    let gitconfig_path = ctx.agent_dir.join("gitconfig");
    let home = &ctx.env.home;

    // ---- hooks: every event → orch-hook, 600 s
    let hook = json!([{ "matcher": "*", "hooks": [{
        "type": "command",
        "command": ctx.helpers.hook.display().to_string(),
        "timeout": HOOK_TIMEOUT_S,
    }]}]);
    let hooks: serde_json::Map<String, Value> =
        HookEvent::REGISTERED.iter().map(|e| (e.as_str().to_string(), hook.clone())).collect();

    // ---- sandbox (Bash only) and permission rules (Read/Edit/Write)
    let mut deny_read: Vec<String> = CREDENTIAL_STORES.iter().map(|(p, _)| format!("~/{p}")).collect();
    deny_read.extend(ctx.main_read_denies.iter().map(|p| p.display().to_string()));
    deny_read.extend(ctx.other_worktrees.iter().map(|p| p.display().to_string()));
    deny_read.push(ctx.app_root.join("state").display().to_string());
    // The CLI denies reads of `<repo>/.git/worktrees` unless re-allowed (P1 live test).
    let allow_read = vec![ctx.git_common_dir.display().to_string()];
    // Meant to let git commit in the worktree. Claude Code 2.1.302 still refuses
    // these writes from sandboxed Bash (README, "Known gaps").
    let mut allow_write = vec![ctx.git_common_dir.display().to_string()];
    allow_write.extend(role.extra_write.iter().map(|p| p.display().to_string()));

    let mut deny = vec!["Bash(git push:*)".to_string()];
    for (p, is_dir) in CREDENTIAL_STORES {
        let abs = rule_path(&home.join(p));
        deny.push(if *is_dir { format!("Read({abs}/**)") } else { format!("Read({abs})") });
    }
    for p in std::iter::once(&ctx.main_checkout).chain(&ctx.other_worktrees) {
        let abs = rule_path(p);
        deny.push(format!("Read({abs}/**)"));
        deny.push(format!("Edit({abs}/**)"));
    }
    deny.push(format!("Read({}/**)", rule_path(&ctx.app_root.join("state"))));
    deny.push(format!("Edit({}/**)", rule_path(&ctx.app_root)));
    deny.extend(role.deny_rules.iter().cloned());
    let mut allow = role.allow_rules.clone();
    let tools = role.tools();
    allow.extend(tools.iter().map(|t| format!("mcp__{ORCH_MCP_SERVER}__{t}")));

    let settings = json!({
        "hooks": hooks,
        "statusLine": {
            "type": "command",
            "command": ctx.helpers.status.display().to_string(),
            "refreshInterval": STATUSLINE_REFRESH_S,
        },
        "sandbox": {
            "enabled": true,
            "failIfUnavailable": true,
            "autoAllowBashIfSandboxed": true,
            "allowUnsandboxedCommands": false,
            "network": { "allowedDomains": role.network_allow },
            "filesystem": {
                "denyRead": deny_read,
                "allowRead": allow_read,
                "allowWrite": allow_write,
            },
        },
        "permissions": {
            "allow": allow,
            "deny": deny,
        },
    });
    // The orchestrator's own server always comes with the agent (it wins over
    // a role entry of the same name). It inherits the agent's environment;
    // the identity is repeated explicitly.
    let mut servers = role.mcp_servers.clone();
    servers.insert(
        ORCH_MCP_SERVER.into(),
        json!({
            "type": "stdio",
            "command": ctx.helpers.mcp.display().to_string(),
            "args": [],
            "env": {
                orch_protocol::env::AGENT_ID: ctx.agent_id,
                orch_protocol::env::SOCKET: ctx.env.socket.display().to_string(),
                orch_protocol::env::TOOLS: tools.join(","),
            },
        }),
    );
    let mcp = json!({ "mcpServers": servers });

    // ---- command line (flags verified in P0, 2.1.302)
    let mut args: Vec<String> = vec![
        "--settings".into(),
        settings_path.display().to_string(),
        "--mcp-config".into(),
        mcp_path.display().to_string(),
    ];
    if role.strict_mcp {
        args.push("--strict-mcp-config".into());
    }
    if let Some(src) = &role.setting_sources {
        args.push("--setting-sources".into());
        args.push(src.join(","));
    }
    if let Some(m) = &role.model {
        args.push("--model".into());
        args.push(m.clone());
    }
    if let Some(e) = &role.effort {
        args.push("--effort".into());
        args.push(e.clone());
    }
    args.push("--permission-mode".into());
    args.push(role.permission_mode.as_flag().into());
    let preamble = if tools.iter().any(|t| t == "request_pr") { ORCH_PREAMBLE } else { ORCH_PREAMBLE_TEAM };
    let prompt = match role.prompt.as_deref().map(str::trim).filter(|p| !p.is_empty()) {
        Some(p) => format!("{preamble}\n\n{p}"),
        None => preamble.to_string(),
    };
    args.push("--append-system-prompt".into());
    args.push(prompt);
    args.push("-n".into());
    args.push(ctx.name.clone());
    match &ctx.session {
        SessionMode::New(id) => args.extend(["--session-id".into(), id.clone()]),
        SessionMode::Resume(id) => args.extend(["--resume".into(), id.clone()]),
        SessionMode::Fork(id) => args.extend(["--resume".into(), id.clone(), "--fork-session".into()]),
    }
    debug_assert!(!args.iter().any(|a| a == "--bare" || a == "--safe-mode" || a.contains("bypassPermissions")));

    let mut env_in = ctx.env.clone();
    env_in.agent_id = ctx.agent_id.clone();
    env_in.agent_dir = ctx.agent_dir.clone();
    env_in.gitconfig = gitconfig_path.clone();
    Compiled {
        agent_dir: ctx.agent_dir.clone(),
        settings_path,
        mcp_path,
        gitconfig_path,
        settings,
        mcp,
        gitconfig: gitconfig(&ctx.git_identity.0, &ctx.git_identity.1),
        program: ctx.claude.clone(),
        args,
        env: {
            let mut env = agent_env(&env_in);
            env.insert(orch_protocol::env::TOOLS.into(), tools.join(","));
            env
        },
        cwd: ctx.worktree.clone(),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::role::PermissionMode;

    pub fn ctx(session: SessionMode) -> CompileCtx {
        let mut env = EnvInputs::from_process("/usr/bin:/bin".into());
        env.home = "/Users/u".into();
        env.socket = "/app/core.sock".into();
        env.user_statusline = None;
        CompileCtx {
            agent_id: "a1".into(),
            name: "solo-fix".into(),
            agent_dir: "/app/agents/a1".into(),
            app_root: "/app".into(),
            worktree: "/p/demo.worktrees/solo/fix".into(),
            main_checkout: "/p/demo".into(),
            main_read_denies: vec!["/p/demo/README.md".into(), "/p/demo/src".into()],
            git_common_dir: "/p/demo/.git".into(),
            other_worktrees: vec!["/p/demo.worktrees/solo/other".into()],
            helpers: HelperPaths::in_dir(Path::new("/app/bin")),
            claude: "/Users/u/.local/bin/claude".into(),
            session,
            env,
            git_identity: ("N".into(), "e@x".into()),
        }
    }

    fn strs(v: &Value) -> Vec<String> {
        v.as_array().unwrap().iter().map(|s| s.as_str().unwrap().to_string()).collect()
    }

    #[test]
    fn settings_hooks_statusline_sandbox_permissions() {
        let c = compile(&Role::solo(), &ctx(SessionMode::New("u-1".into())));
        let s = &c.settings;
        for e in HookEvent::REGISTERED {
            let h = &s["hooks"][e.as_str()][0]["hooks"][0];
            assert_eq!(h["command"], "/app/bin/orch-hook", "{e}");
            assert_eq!(h["timeout"], 600, "{e}");
        }
        assert_eq!(s["statusLine"]["command"], "/app/bin/orch-status");
        assert_eq!(s["statusLine"]["refreshInterval"], 5);
        let sb = &s["sandbox"];
        assert_eq!(sb["enabled"], true);
        assert_eq!(sb["failIfUnavailable"], true);
        assert_eq!(sb["allowUnsandboxedCommands"], false);
        let deny_read = strs(&sb["filesystem"]["denyRead"]);
        for must in [
            "~/.ssh",
            "~/.config/gh",
            "~/.netrc",
            "~/.git-credentials",
            "~/.aws",
            "/p/demo/src",
            "/p/demo.worktrees/solo/other",
        ] {
            assert!(deny_read.contains(&must.to_string()), "{must}");
        }
        assert!(!deny_read.iter().any(|p| p.ends_with(".gitconfig")), "denying ~/.gitconfig breaks git");
        assert!(!deny_read.contains(&"/p/demo.worktrees/solo/fix".to_string()), "own worktree stays readable");
        // allowWrite under a denyRead path is dropped by the CLI: the git dir must not be under one.
        assert_eq!(strs(&sb["filesystem"]["allowRead"]), ["/p/demo/.git"]);
        let allow_write = strs(&sb["filesystem"]["allowWrite"]);
        assert!(allow_write.contains(&"/p/demo/.git".to_string()));
        for w in &allow_write {
            assert!(
                !deny_read.iter().any(|d| Path::new(w).starts_with(d.trim_start_matches('~'))),
                "{w} under a denied read path"
            );
        }
        let deny = strs(&s["permissions"]["deny"]);
        assert!(deny.contains(&"Bash(git push:*)".to_string()));
        assert!(deny.contains(&"Read(//Users/u/.ssh/**)".to_string()));
        assert!(deny.contains(&"Read(//Users/u/.netrc)".to_string()));
        assert!(deny.contains(&"Read(//p/demo/**)".to_string()));
        assert!(deny.contains(&"Edit(//p/demo.worktrees/solo/other/**)".to_string()));
        assert!(deny.contains(&"Edit(//app/**)".to_string()));
        // Every absolute path in a rule uses the `//` form.
        for r in &deny {
            if let Some(inner) = r.strip_prefix("Read(").or_else(|| r.strip_prefix("Edit(")) {
                assert!(inner.starts_with("//"), "{r}");
            }
        }
    }

    #[test]
    fn command_line_flags() {
        let mut role = Role::solo();
        role.model = Some("haiku".into());
        role.effort = Some("low".into());
        role.strict_mcp = true;
        role.permission_mode = PermissionMode::AcceptEdits;
        role.prompt = Some("You are the backend role.".into());
        role.setting_sources = Some(vec!["project".into(), "local".into()]);
        let c = compile(&role, &ctx(SessionMode::New("u-1".into())));
        let a = &c.args;
        let after = |flag: &str| a[a.iter().position(|x| x == flag).unwrap_or_else(|| panic!("{flag}")) + 1].clone();
        assert_eq!(after("--settings"), "/app/agents/a1/settings.json");
        assert_eq!(after("--mcp-config"), "/app/agents/a1/mcp.json");
        assert!(a.contains(&"--strict-mcp-config".to_string()));
        assert_eq!(after("--setting-sources"), "project,local");
        assert_eq!(after("--model"), "haiku");
        assert_eq!(after("--effort"), "low");
        assert_eq!(after("--permission-mode"), "acceptEdits");
        assert!(after("--append-system-prompt").starts_with(ORCH_PREAMBLE));
        assert!(after("--append-system-prompt").ends_with("You are the backend role."));
        assert_eq!(after("-n"), "solo-fix");
        assert_eq!(after("--session-id"), "u-1");
        assert!(!a.iter().any(|x| x == "--bare" || x == "--safe-mode" || x == "--resume"));
        assert_eq!(c.cwd, PathBuf::from("/p/demo.worktrees/solo/fix"));
        assert_eq!(c.env["GIT_CONFIG_GLOBAL"], "/app/agents/a1/gitconfig");
        assert_eq!(c.env["ORCH_AGENT_DIR"], "/app/agents/a1");
        assert!(!c.env.contains_key("ANTHROPIC_API_KEY"));
    }

    #[test]
    fn permission_mode_always_explicit_and_resume_fork() {
        let c = compile(&Role::solo(), &ctx(SessionMode::Resume("s-9".into())));
        let i = c.args.iter().position(|x| x == "--permission-mode").unwrap();
        assert_eq!(c.args[i + 1], "manual");
        let i = c.args.iter().position(|x| x == "--resume").unwrap();
        assert_eq!(c.args[i + 1], "s-9");
        assert!(!c.args.contains(&"--session-id".to_string()));
        let f = compile(&Role::solo(), &ctx(SessionMode::Fork("s-9".into())));
        assert!(f.args.ends_with(&["--resume".into(), "s-9".into(), "--fork-session".into()]));
        assert!(!c.args.contains(&"--strict-mcp-config".to_string()));
    }

    #[test]
    fn writes_files() {
        let d = tempfile::tempdir().unwrap();
        let mut cx = ctx(SessionMode::New("u".into()));
        cx.agent_dir = d.path().join("agents/a1");
        let c = compile(&Role::solo(), &cx);
        c.write().unwrap();
        let s: Value = serde_json::from_str(&std::fs::read_to_string(&c.settings_path).unwrap()).unwrap();
        assert_eq!(s, c.settings);
        assert!(std::fs::read_to_string(&c.gitconfig_path).unwrap().contains("helper ="));
        let servers =
            serde_json::from_str::<Value>(&std::fs::read_to_string(&c.mcp_path).unwrap()).unwrap()["mcpServers"]
                .clone();
        assert_eq!(
            servers,
            json!({"orch": {"type": "stdio", "command": "/app/bin/orch-mcp", "args": [],
                            "env": {"ORCH_AGENT_ID": "a1", "ORCH_SOCK": "/app/core.sock", "ORCH_TOOLS": "commit,request_pr"}}})
        );
        // The orchestrator tools are allowed (the core applies its own rules).
        let allow = strs(&c.settings["permissions"]["allow"]);
        assert!(
            allow.contains(&"mcp__orch__commit".to_string()) && allow.contains(&"mcp__orch__request_pr".to_string())
        );
    }

    #[test]
    fn a_team_specialist_gets_its_tools_and_denies() {
        let (roles, team) = orch_team::presets::full_stack();
        let f = orch_team::FeatureCtx { slug: "f".into(), title: "F".into(), base: "main".into() };
        let node = orch_team::compile_node(&team, "back", &roles, &f).unwrap();
        let role = Role::from_node(&node).unwrap();
        let c = compile(&role, &ctx(SessionMode::New("u".into())));
        let allow = c.settings["permissions"]["allow"].as_array().unwrap();
        assert!(allow.contains(&json!("mcp__orch__report_done")));
        assert!(!allow.contains(&json!("mcp__orch__request_pr")), "specialists don't open PRs");
        assert!(c.settings["permissions"]["deny"].as_array().unwrap().contains(&json!("Agent")));
        let tools = c.mcp["mcpServers"]["orch"]["env"]["ORCH_TOOLS"].as_str().unwrap();
        assert!(tools.contains("sync_integration") && !tools.contains("merge"));
        let prompt = &c.args[c.args.iter().position(|a| a == "--append-system-prompt").unwrap() + 1];
        assert!(prompt.starts_with(ORCH_PREAMBLE_TEAM));
        assert!(prompt.contains("You are node `back`"));
        assert!(c.args.windows(2).any(|w| w[0] == "--permission-mode" && w[1] == "acceptEdits"));
    }

    #[test]
    fn main_checkout_is_denied_entry_by_entry_except_git() {
        let d = tempfile::tempdir().unwrap();
        let main = d.path().join("demo");
        std::fs::create_dir_all(main.join(".git/worktrees")).unwrap();
        std::fs::create_dir_all(main.join("src")).unwrap();
        std::fs::write(main.join("README.md"), "x").unwrap();
        std::fs::write(main.join(".env"), "SECRET=1").unwrap();
        let denies = main_read_denies(&main, &main.join(".git"));
        assert_eq!(denies, [main.join(".env"), main.join("README.md"), main.join("src")]);
        // A git dir outside the checkout: deny the checkout as a whole.
        assert_eq!(main_read_denies(&main, Path::new("/elsewhere/.git")), vec![main]);
    }

    #[test]
    fn rule_paths() {
        assert_eq!(rule_path(Path::new("/a/b")), "//a/b");
    }
}
