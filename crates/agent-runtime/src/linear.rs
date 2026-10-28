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

/// The real transport.
pub struct LinearHttp {
    pub net: Arc<dyn Net>,
    pub secrets: Arc<dyn Secrets>,
}

impl Graphql for LinearHttp {
    fn call(&self, query: &str, variables: &Value) -> Result<Value, GqlError> {
        let token = self
            .secrets
            .get("linear")
            .map_err(|e| GqlError::Failed(e.to_string()))?
            .ok_or_else(|| GqlError::Unauthorized("no Linear API key: add it in Settings › Integrations".into()))?;
        let req = HttpRequest {
            method: "POST",
            url: linear::URL.into(),
            headers: vec![
                ("Authorization".into(), token.expose().into()),
                ("Content-Type".into(), "application/json".into()),
            ],
            body: Some(json!({"query": query, "variables": variables}).to_string()),
            timeout: Duration::from_secs(30),
            max_body: crate::net::API_MAX_BODY,
        };
        let r = self.net.http(&req).map_err(|e| GqlError::Failed(format!("{e:#}")))?;
        linear::parse_answer(r.status, r.header("retry-after"), &r.body)
    }
}

impl Runtime {
    pub(crate) fn linear_transport(&self) -> Box<dyn Graphql> {
        match &self.cfg.linear {
            Some(f) => (f.0)(),
            None => Box::new(LinearHttp { net: self.cfg.net.clone(), secrets: self.cfg.secrets.clone() }),
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
