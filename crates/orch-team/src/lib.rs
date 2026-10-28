#![doc = include_str!("../README.md")]

pub mod artifact;
pub mod compile;
pub mod export;
pub mod mcp;
pub mod model;
pub mod presets;
pub mod rules;
pub mod run;
pub mod validate;

pub use compile::{FeatureCtx, NodeConfig, compile_node, tools_for};
pub use export::{ImportError, TeamFile};
pub use model::{
    AgentNode, Edge, EdgeKind, PermissionProfile, ReviewMode, RoleDef, RoleOverrides, Scope, StarterOrigin, Team,
    Tools, TurnCeiling, valid_slug,
};
pub use run::{RunState, TaskStatus, blocked_by};
pub use validate::{Issue, validate};
