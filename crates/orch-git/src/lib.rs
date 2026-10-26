#![doc = include_str!("../README.md")]

pub mod cleanup;
pub mod commit;
pub mod diff;
pub mod fetch;
pub mod gh;
pub mod git;
pub mod project;
pub mod repair;
pub mod status;
pub mod worktree;

#[cfg(any(test, feature = "fixtures"))]
pub mod fixtures;

pub use git::{git, real, valid_id};
