//! The database snapshot taken before a restart into a PR build (spec §4.3, T-3).

use anyhow::Result;

use crate::BuildInfo;
use crate::core::Core;

/// Runs before `update_restart` swaps a PR build in, with the app still up. T-3 writes the
/// snapshot here when `current` is a `main` or `local` build (a PR→PR switch keeps the
/// existing one). An `Err` refuses the restart: the app stays as it was. Nothing yet.
pub fn before_pr_restart(_core: &Core, _current: &BuildInfo) -> Result<()> {
    Ok(())
}
