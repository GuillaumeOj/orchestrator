#![doc = include_str!("../README.md")]

pub mod cleanup;
pub mod commit;
pub mod diff;
pub mod fetch;
pub mod gh;
pub mod git;
pub mod project;
pub mod repair;
pub mod replay;
pub mod status;
pub mod worktree;

#[cfg(any(test, feature = "fixtures"))]
pub mod fixtures;

pub use git::{git, is_app_branch, real, valid_branch, valid_id};
