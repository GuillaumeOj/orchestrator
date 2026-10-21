//! Checks run at every spawn: the `claude` version range (NFR-7), the helper
//! binaries (FR-1201: a missing hook command fails open), git ≥ 2.48 (FR-105).

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::Command;

use anyhow::{Context, Result, bail};

use crate::compile::HelperPaths;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version(pub u32, pub u32, pub u32);

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.0, self.1, self.2)
    }
}

impl Version {
    /// The first `x.y.z` in the text (`2.1.302 (Claude Code)`, `git version 2.54.0 (Apple Git-157)`).
    pub fn find(text: &str) -> Option<Version> {
        text.split(|c: char| !(c.is_ascii_digit() || c == '.')).find_map(|tok| {
            let mut it = tok.split('.').filter(|s| !s.is_empty());
            let v = Version(it.next()?.parse().ok()?, it.next()?.parse().ok()?, it.next().unwrap_or("0").parse().ok()?);
            Some(v)
        })
    }
}

/// The supported Claude Code range. Below `min`: refuse. Above `tested_max`:
/// warn (hooks, flags and statusline formats can change; run the contract
/// tests, R2) unless `strict`.
#[derive(Clone, Copy, Debug)]
pub struct VersionRange {
    pub min: Version,
    pub tested_max: Version,
    pub strict: bool,
}

/// P0 baseline: 2.1.302.
pub const SUPPORTED_CLAUDE: VersionRange =
    VersionRange { min: Version(2, 1, 302), tested_max: Version(2, 1, 302), strict: false };

pub const MIN_GIT: Version = Version(2, 48, 0);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VersionCheck {
    Supported(Version),
    Untested(Version),
}

impl VersionRange {
    pub fn check(&self, v: Version) -> Result<VersionCheck> {
        if v < self.min {
            bail!("claude {v} is older than the supported minimum {}", self.min);
        }
        if v > self.tested_max {
            if self.strict {
                bail!("claude {v} is newer than the last tested version {}", self.tested_max);
            }
            return Ok(VersionCheck::Untested(v));
        }
        Ok(VersionCheck::Supported(v))
    }
}

pub fn claude_version(claude: &Path) -> Result<Version> {
    let out = Command::new(claude)
        .arg("--version")
        .env("DISABLE_AUTOUPDATER", "1")
        .output()
        .with_context(|| format!("running {} --version", claude.display()))?;
    let text = String::from_utf8_lossy(&out.stdout);
    Version::find(&text).with_context(|| format!("no version in {text:?}"))
}

pub fn git_version() -> Result<Version> {
    let out = Command::new("git").arg("--version").output().context("running git --version")?;
    Version::find(&String::from_utf8_lossy(&out.stdout)).context("no git version")
}

pub fn check_git() -> Result<Version> {
    let v = git_version()?;
    if v < MIN_GIT {
        bail!("git {v} is too old: relative worktree paths need git ≥ {MIN_GIT}");
    }
    Ok(v)
}

/// Each helper must exist and be executable at a stable path, not in the app bundle.
pub fn check_helpers(h: &HelperPaths) -> Result<()> {
    for p in [&h.hook, &h.status] {
        let m = std::fs::metadata(p).with_context(|| format!("helper missing: {}", p.display()))?;
        if !m.is_file() || m.permissions().mode() & 0o111 == 0 {
            bail!("helper not executable: {}", p.display());
        }
        if p.components().any(|c| c.as_os_str().to_string_lossy().ends_with(".app")) {
            bail!("helper inside an app bundle (moves on update): {}", p.display());
        }
    }
    Ok(())
}

/// Copies the helpers next to `from_exe` (the build output) to the stable bin dir.
pub fn install_helpers(from_dir: &Path, bin_dir: &Path) -> Result<HelperPaths> {
    std::fs::create_dir_all(bin_dir)?;
    for name in ["orch-hook", "orch-status"] {
        let src = from_dir.join(name);
        let dst = bin_dir.join(name);
        let tmp = bin_dir.join(format!(".{name}.tmp"));
        std::fs::copy(&src, &tmp).with_context(|| format!("copying {}", src.display()))?;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))?;
        std::fs::rename(&tmp, &dst)?;
    }
    let h = HelperPaths::in_dir(bin_dir);
    check_helpers(&h)?;
    Ok(h)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_versions() {
        assert_eq!(Version::find("2.1.302 (Claude Code)"), Some(Version(2, 1, 302)));
        assert_eq!(Version::find("git version 2.54.0 (Apple Git-157)"), Some(Version(2, 54, 0)));
        assert_eq!(Version::find("nothing"), None);
    }

    #[test]
    fn range() {
        let r = SUPPORTED_CLAUDE;
        assert_eq!(r.check(Version(2, 1, 302)).unwrap(), VersionCheck::Supported(Version(2, 1, 302)));
        assert!(r.check(Version(2, 1, 296)).is_err());
        assert_eq!(r.check(Version(2, 2, 0)).unwrap(), VersionCheck::Untested(Version(2, 2, 0)));
        assert!(VersionRange { strict: true, ..r }.check(Version(2, 2, 0)).is_err());
    }

    #[test]
    fn helpers_must_exist_and_be_executable() {
        let d = tempfile::tempdir().unwrap();
        let h = HelperPaths::in_dir(d.path());
        assert!(check_helpers(&h).is_err());
        std::fs::write(&h.hook, "#!/bin/sh\n").unwrap();
        std::fs::write(&h.status, "#!/bin/sh\n").unwrap();
        assert!(check_helpers(&h).is_err(), "not executable");
        let src = tempfile::tempdir().unwrap();
        for n in ["orch-hook", "orch-status"] {
            std::fs::write(src.path().join(n), "#!/bin/sh\n").unwrap();
        }
        let h = install_helpers(src.path(), &d.path().join("bin")).unwrap();
        check_helpers(&h).unwrap();
    }
}
