//! The Linear task backend's transport (FR-501): GraphQL at
//! api.linear.app from the core only, with the Keychain's API key (account
//! `linear`) in a header on curl's stdin and the body in a 0600 file
//! (`net.rs`). Agents never call Linear (ADR-009).

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Result, anyhow};
use orch_secrets::Secrets;
use orch_tasks::linear::{self, GqlError, Graphql};
use serde_json::{Value, json};

use crate::net::{HttpRequest, Net};
use crate::runtime::Runtime;

/// Tests replace the transport.
#[derive(Clone)]
pub struct LinearFactory(pub Arc<dyn Fn() -> Box<dyn Graphql> + Send + Sync>);

impl std::fmt::Debug for LinearFactory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("LinearFactory")
    }
}

/// A GraphQL request to api.linear.app with the API key (the sync's and the
/// integration test's).
pub fn request(token: &str, query: &str, variables: &Value, timeout: Duration, max_body: usize) -> HttpRequest {
    HttpRequest {
        method: "POST",
        url: linear::URL.into(),
        headers: vec![("Authorization".into(), token.into()), ("Content-Type".into(), "application/json".into())],
        body: Some(json!({"query": query, "variables": variables}).to_string()),
        timeout,
        max_body,
    }
}

/// The real transport. The key is read from the Keychain once and kept
/// (in `token`, shared with the runtime) until the Linear integration's
/// secret is set or removed.
pub struct LinearHttp {
    pub net: Arc<dyn Net>,
    pub secrets: Arc<dyn Secrets>,
    pub token: TokenCache,
}

/// The Linear API key, read once, dropped when the user changes it.
pub type TokenCache = Arc<std::sync::Mutex<Option<orch_secrets::Secret>>>;

impl LinearHttp {
    fn token(&self) -> Result<orch_secrets::Secret, GqlError> {
        let mut t = self.token.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(s) = t.as_ref() {
            return Ok(s.clone());
        }
        let s = self
            .secrets
            .get("linear")
            .map_err(|e| GqlError::Failed(e.to_string()))?
            .ok_or_else(|| GqlError::Unauthorized("no Linear API key: add it in Settings › Integrations".into()))?;
        *t = Some(s.clone());
        Ok(s)
    }
}

impl Graphql for LinearHttp {
    fn call(&self, query: &str, variables: &Value) -> Result<Value, GqlError> {
        let token = self.token()?;
        let req = request(token.expose(), query, variables, Duration::from_secs(30), crate::net::API_MAX_BODY);
        let r = self.net.http(&req).map_err(|e| GqlError::Failed(format!("{e:#}")))?;
        linear::parse_answer(r.status, r.header("retry-after"), &r.body)
    }
}

impl Runtime {
    pub(crate) fn linear_transport(&self) -> Box<dyn Graphql> {
        match &self.cfg.linear {
            Some(f) => (f.0)(),
            None => Box::new(LinearHttp {
                net: self.cfg.net.clone(),
                secrets: self.cfg.secrets.clone(),
                token: self.linear_token.clone(),
            }),
        }
    }

    /// `linear_teams`: `{ teams: [{id, key, name}] }`.
    pub fn linear_teams(&self) -> Result<Value> {
        Ok(json!({"teams": linear::teams(self.linear_transport().as_ref())?}))
    }

    /// `linear_states`: a team's states with their default column, and its projects.
    pub fn linear_states(&self, team: &str) -> Result<Value> {
        let (states, projects) =
            linear::team_states(self.linear_transport().as_ref(), team).map_err(|e| anyhow!("{e:#}"))?;
        Ok(json!({"states": states, "projects": projects}))
    }
}
