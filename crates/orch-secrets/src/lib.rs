#![doc = include_str!("../README.md")]

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Mutex;

use anyhow::{Context, Result, bail};

/// The Keychain service every secret of the app is stored under.
pub const SERVICE: &str = "Orchestrator";

/// The prefix of an MCP `env` value that names a Keychain account
/// (`keychain:<account>`), resolved at compile time only after the entry's opt-in.
pub const KEYCHAIN_PREFIX: &str = "keychain:";

/// `keychain:<account>` → `<account>`; `None` for a plain value.
pub fn keychain_ref(value: &str) -> Option<&str> {
    value.strip_prefix(KEYCHAIN_PREFIX).map(str::trim).filter(|a| !a.is_empty())
}

/// A secret value. Its `Debug` and `Display` never show it, so it can't end
/// up in a log line or an error by accident; [`Secret::expose`] is the one way in.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret(String);

impl Secret {
    pub fn new(s: impl Into<String>) -> Secret {
        Secret(s.into())
    }
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(<redacted>)")
    }
}

impl fmt::Display for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<redacted>")
    }
}

/// Where secrets live: the Keychain in the app, memory in tests.
pub trait Secrets: Send + Sync + fmt::Debug {
    fn get(&self, account: &str) -> Result<Option<Secret>>;
    fn set(&self, account: &str, secret: &Secret) -> Result<()>;
    /// Removing a missing secret is not an error.
    fn remove(&self, account: &str) -> Result<()>;
    fn has(&self, account: &str) -> bool {
        self.get(account).ok().flatten().is_some()
    }
}

fn check_account(account: &str) -> Result<()> {
    if account.is_empty() || account.len() > 64 || !account.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
    {
        bail!("invalid secret account {account:?} (a-z, 0-9, - _ ., up to 64)");
    }
    Ok(())
}

/// The login Keychain through the Security framework (generic passwords,
/// service [`SERVICE`]). Never goes through the `security` tool: its `-w`
/// would put the secret on a command line.
#[derive(Clone, Copy, Debug, Default)]
pub struct Keychain;

/// `errSecItemNotFound`.
const NOT_FOUND: i32 = -25300;

impl Secrets for Keychain {
    fn get(&self, account: &str) -> Result<Option<Secret>> {
        check_account(account)?;
        match security_framework::passwords::get_generic_password(SERVICE, account) {
            Ok(bytes) => Ok(Some(Secret(String::from_utf8(bytes).context("the Keychain item is not UTF-8")?))),
            Err(e) if e.code() == NOT_FOUND => Ok(None),
            Err(e) => bail!("Keychain read of {account}: {e}"),
        }
    }

    fn set(&self, account: &str, secret: &Secret) -> Result<()> {
        check_account(account)?;
        security_framework::passwords::set_generic_password(SERVICE, account, secret.expose().as_bytes())
            .with_context(|| format!("Keychain write of {account}"))
    }

    fn remove(&self, account: &str) -> Result<()> {
        check_account(account)?;
        match security_framework::passwords::delete_generic_password(SERVICE, account) {
            Ok(()) => Ok(()),
            Err(e) if e.code() == NOT_FOUND => Ok(()),
            Err(e) => bail!("Keychain delete of {account}: {e}"),
        }
    }
}

/// In-memory secrets for tests.
#[derive(Debug, Default)]
pub struct MemorySecrets(Mutex<BTreeMap<String, Secret>>);

impl MemorySecrets {
    pub fn with(pairs: &[(&str, &str)]) -> MemorySecrets {
        MemorySecrets(Mutex::new(pairs.iter().map(|(a, s)| (a.to_string(), Secret::new(*s))).collect()))
    }
}

impl Secrets for MemorySecrets {
    fn get(&self, account: &str) -> Result<Option<Secret>> {
        check_account(account)?;
        Ok(self.0.lock().unwrap_or_else(|e| e.into_inner()).get(account).cloned())
    }
    fn set(&self, account: &str, secret: &Secret) -> Result<()> {
        check_account(account)?;
        self.0.lock().unwrap_or_else(|e| e.into_inner()).insert(account.into(), secret.clone());
        Ok(())
    }
    fn remove(&self, account: &str) -> Result<()> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).remove(account);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_never_print() {
        let s = Secret::new("lin_api_123");
        assert_eq!(format!("{s:?} {s}"), "Secret(<redacted>) <redacted>");
        assert_eq!(s.expose(), "lin_api_123");
    }

    #[test]
    fn keychain_refs() {
        assert_eq!(keychain_ref("keychain:linear"), Some("linear"));
        assert_eq!(keychain_ref("keychain:"), None);
        assert_eq!(keychain_ref("plain"), None);
    }

    #[test]
    fn memory_store_and_accounts() {
        let m = MemorySecrets::default();
        assert!(!m.has("linear"));
        m.set("linear", &Secret::new("k")).unwrap();
        assert_eq!(m.get("linear").unwrap().unwrap().expose(), "k");
        m.remove("linear").unwrap();
        m.remove("linear").unwrap();
        assert!(m.get("linear").unwrap().is_none());
        assert!(m.set("../x", &Secret::new("k")).is_err());
        assert!(Keychain.get("bad account").is_err(), "checked before the Keychain is touched");
    }

    /// Touches the real login Keychain (may show a prompt): `cargo test -p orch-secrets -- --ignored`.
    #[test]
    #[ignore]
    fn keychain_round_trip() {
        let acct = "orch-secrets-test";
        Keychain.set(acct, &Secret::new("v1")).unwrap();
        Keychain.set(acct, &Secret::new("v2")).unwrap();
        assert_eq!(Keychain.get(acct).unwrap().unwrap().expose(), "v2");
        Keychain.remove(acct).unwrap();
        assert!(Keychain.get(acct).unwrap().is_none());
    }
}
