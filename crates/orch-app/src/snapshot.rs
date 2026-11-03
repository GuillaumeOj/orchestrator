//! Your data survives a PR build with a migration (spec §4.3, api-contract §6).
//!
//! A PR build may add a migration; once it ran, a `main` build refuses the database. So
//! restarting into a PR build from a `main` or `local` build first writes a snapshot of the
//! store (`state/backup/`), and a later start whose store is newer than the build restores it.
//!
//! ```text
//! state/orch.db, orch.blobs/             the live store
//! state/backup/orch.db, orch.blobs/      the snapshot
//! state/backup/backup.json               { schema, taken_at, from: BuildInfo, to_pr }
//! state/aside/orch-<taken_at>-schema-<v>/   a store moved aside by a restore, never deleted
//! ```

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::BuildInfo;
use crate::core::Core;

/// How long a snapshot is kept once a `main` build runs on the database normally.
const KEEP_SECS: i64 = 14 * 24 * 3600;
const DB: &str = "orch.db";
const BLOBS: &str = "orch.blobs";
const INFO: &str = "backup.json";

/// `backup.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Backup {
    schema: u32,
    taken_at: i64,
    from: serde_json::Value,
    to_pr: u64,
}

/// The `state/` dir and the paths in it, from the live database path.
struct Dirs {
    state: PathBuf,
}

impl Dirs {
    fn of(db: &Path) -> Dirs {
        Dirs { state: db.parent().map(Path::to_path_buf).unwrap_or_default() }
    }
    fn backup(&self) -> PathBuf {
        self.state.join("backup")
    }
    fn aside(&self) -> PathBuf {
        self.state.join("aside")
    }
    fn db(&self) -> PathBuf {
        self.state.join(DB)
    }
    fn blobs(&self) -> PathBuf {
        self.state.join(BLOBS)
    }
}

/// Runs before `update_restart` swaps a PR build in, with the app still up. Takes the snapshot
/// when `current` is a `main` or `local` build; a PR→PR switch keeps the existing one (it is the
/// last state a release build could read). An `Err` refuses the restart: the app stays as it
/// was, the previous snapshot untouched.
pub fn before_pr_restart(core: &Core, current: &BuildInfo, to_pr: u64) -> Result<()> {
    core.db.flush().context("flushing the store")?;
    take_unless_pr(&core.db, now_secs(), current, to_pr)
}

fn take_unless_pr(db: &orch_store::Store, taken_at: i64, current: &BuildInfo, to_pr: u64) -> Result<()> {
    if current.channel == "pr" {
        return Ok(());
    }
    take(db, taken_at, current, to_pr)
}

fn now_secs() -> i64 {
    jiff::Timestamp::now().as_second()
}

/// Writes the snapshot next to the old one and swaps it in, so a failure leaves the old one.
fn take(db: &orch_store::Store, taken_at: i64, from: &BuildInfo, to_pr: u64) -> Result<()> {
    let d = Dirs::of(db.path());
    let new = d.state.join("backup.new");
    let _ = std::fs::remove_dir_all(&new);
    let res = (|| -> Result<()> {
        std::fs::create_dir_all(&new)?;
        db.backup_to(&new.join(DB))?;
        // The copy's own version: what a restore compares with the build.
        let schema = orch_store::schema_version(&new.join(DB))?.context("the snapshot has no database")?;
        let blobs = db.blob_dir();
        if blobs.is_dir() {
            link_tree(&blobs, &new.join(BLOBS)).context("copying the blobs")?;
        }
        let info = Backup { schema, taken_at, from: serde_json::to_value(from)?, to_pr };
        std::fs::write(new.join(INFO), serde_json::to_vec_pretty(&info)?)?;
        let old = d.state.join("backup.old");
        let _ = std::fs::remove_dir_all(&old);
        if d.backup().exists() {
            std::fs::rename(d.backup(), &old)?;
        }
        if let Err(e) = std::fs::rename(&new, d.backup()) {
            let _ = std::fs::rename(&old, d.backup());
            return Err(e.into());
        }
        let _ = std::fs::remove_dir_all(&old);
        Ok(())
    })();
    if res.is_err() {
        let _ = std::fs::remove_dir_all(&new);
    }
    res.context("taking the database snapshot")
}

/// `from` copied to `to`, files hard-linked when the volume allows it, else copied.
fn link_tree(from: &Path, to: &Path) -> Result<()> {
    std::fs::create_dir_all(to)?;
    for e in std::fs::read_dir(from)? {
        let e = e?;
        let dest = to.join(e.file_name());
        if e.file_type()?.is_dir() {
            link_tree(&e.path(), &dest)?;
        } else if std::fs::hard_link(e.path(), &dest).is_err() {
            std::fs::copy(e.path(), &dest)?;
        }
    }
    Ok(())
}

fn read_backup(dir: &Path) -> Option<Backup> {
    serde_json::from_slice(&std::fs::read(dir.join(INFO)).ok()?).ok()
}

/// At start, before the store is opened: when the live store is newer than this build can read
/// and a usable snapshot exists, moves the live store aside and puts the snapshot in its place.
/// Returns the warning to show. Without a usable snapshot, or when anything fails before the
/// live store was touched, nothing changes (the caller's scratch-store behaviour applies).
pub fn restore_if_newer(db: &Path, zone: &jiff::tz::TimeZone) -> Option<String> {
    let have = orch_store::schema_version(db).ok().flatten()?;
    if have <= orch_store::SCHEMA_VERSION {
        return None;
    }
    let d = Dirs::of(db);
    let info = read_backup(&d.backup())?;
    // A snapshot newer than this build is as unreadable as the store.
    let snap_schema = orch_store::schema_version(&d.backup().join(DB)).ok().flatten()?;
    if info.schema > orch_store::SCHEMA_VERSION || snap_schema > orch_store::SCHEMA_VERSION {
        return None;
    }
    match restore(&d, &info, have) {
        Ok(aside) => Some(format!(
            "Your data was restored from before PR #{} (taken {}). What changed while testing it is kept in {}.",
            info.to_pr,
            when(info.taken_at, zone),
            aside.display()
        )),
        Err(e) => {
            eprintln!("snapshot: restore failed: {e:#}");
            None
        }
    }
}

fn restore(d: &Dirs, info: &Backup, have: u32) -> Result<PathBuf> {
    // The snapshot is staged next to the live store first: nothing is moved if that fails.
    let (db_new, blobs_new) = (d.state.join("orch.db.restore"), d.state.join("orch.blobs.restore"));
    let _ = std::fs::remove_file(&db_new);
    let _ = std::fs::remove_dir_all(&blobs_new);
    let staged = std::fs::copy(d.backup().join(DB), &db_new).map_err(anyhow::Error::from).and_then(|_| {
        if d.backup().join(BLOBS).is_dir() {
            link_tree(&d.backup().join(BLOBS), &blobs_new)?;
        }
        Ok(())
    });
    if let Err(e) = staged {
        let _ = std::fs::remove_file(&db_new);
        let _ = std::fs::remove_dir_all(&blobs_new);
        return Err(e.context("staging the snapshot"));
    }
    let aside = unused(&d.aside().join(format!("orch-{}-schema-{have}", info.taken_at)));
    std::fs::create_dir_all(&aside)?;
    // The live store (and its WAL files) moves aside, then the snapshot moves in.
    for name in [DB, "orch.db-wal", "orch.db-shm", BLOBS] {
        let from = d.state.join(name);
        if from.exists() {
            std::fs::rename(&from, aside.join(name)).with_context(|| format!("moving {name} aside"))?;
        }
    }
    std::fs::rename(&db_new, d.db()).context("putting the snapshot in")?;
    if blobs_new.is_dir() {
        std::fs::rename(&blobs_new, d.blobs()).context("putting the snapshot's blobs in")?;
    }
    Ok(aside)
}

/// `path`, or `path-2`, `-3`… so a store set aside is never overwritten.
fn unused(path: &Path) -> PathBuf {
    if !path.exists() {
        return path.to_path_buf();
    }
    (2..).map(|n| PathBuf::from(format!("{}-{n}", path.display()))).find(|p| !p.exists()).expect("an unused name")
}

/// `14:02, 3 Nov 2026` in `zone`.
fn when(secs: i64, zone: &jiff::tz::TimeZone) -> String {
    jiff::Timestamp::from_second(secs)
        .map(|t| t.to_zoned(zone.clone()).strftime("%H:%M, %-d %b %Y").to_string())
        .unwrap_or_else(|_| secs.to_string())
}

/// A `main` build on a database it opened normally drops a snapshot older than 14 days.
pub fn prune(db: &Path, running: &BuildInfo, now: i64) {
    if running.channel != "main" {
        return;
    }
    let d = Dirs::of(db);
    let Some(info) = read_backup(&d.backup()) else { return };
    if now - info.taken_at > KEEP_SECS {
        let _ = std::fs::remove_dir_all(d.backup());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use orch_store::{SCHEMA_VERSION, Store};

    fn build(channel: &str) -> BuildInfo {
        BuildInfo { version: "0.1.50".into(), sha: "abc".into(), channel: channel.into(), pr: None }
    }

    fn utc() -> jiff::tz::TimeZone {
        jiff::tz::TimeZone::UTC
    }

    /// A store under `<root>/state/` holding the value `marker`, plus a blob.
    fn store(root: &Path, marker: &str) -> Store {
        let s = Store::open(&root.join("state/orch.db")).unwrap();
        let m = marker.to_string();
        s.write(move |tx| Ok(tx.execute("INSERT OR REPLACE INTO kv(key, value) VALUES ('m', ?1)", [m])?)).unwrap();
        std::fs::create_dir_all(s.blob_dir()).unwrap();
        std::fs::write(s.blob_dir().join("b1"), marker).unwrap();
        s
    }

    fn marker(db: &Path) -> String {
        let s = Store::open(db).unwrap();
        s.read(|c| Ok(c.query_row("SELECT value FROM kv WHERE key='m'", [], |r| r.get(0))?)).unwrap()
    }

    /// What a PR build with a migration leaves behind: the store at a newer schema.
    fn migrate_ahead(s: Store) {
        s.write(|tx| Ok(tx.pragma_update(None, "user_version", SCHEMA_VERSION + 2)?)).unwrap();
    }

    fn snapshot_then_newer_store(root: &Path, channel: &str) -> PathBuf {
        let s = store(root, "before");
        take_unless_pr(&s, 1793600000, &build(channel), 12).unwrap();
        s.write(|tx| Ok(tx.execute("UPDATE kv SET value='after' WHERE key='m'", [])?)).unwrap();
        std::fs::write(s.blob_dir().join("b2"), "new").unwrap();
        migrate_ahead(s);
        root.join("state/orch.db")
    }

    #[test]
    fn a_restart_from_main_or_local_writes_the_snapshot() {
        for channel in ["main", "local"] {
            let tmp = tempfile::tempdir().unwrap();
            let s = store(tmp.path(), "before");
            take_unless_pr(&s, 1793600000, &build(channel), 12).unwrap();
            let b = tmp.path().join("state/backup");
            assert_eq!(marker(&b.join("orch.db")), "before");
            assert_eq!(std::fs::read_to_string(b.join("orch.blobs/b1")).unwrap(), "before");
            let info: Backup = serde_json::from_slice(&std::fs::read(b.join("backup.json")).unwrap()).unwrap();
            assert_eq!((info.schema, info.taken_at, info.to_pr), (SCHEMA_VERSION, 1793600000, 12));
            assert_eq!(info.from["channel"], channel);
            assert!(!tmp.path().join("state/backup.new").exists());
            // The next one replaces it.
            s.write(|tx| Ok(tx.execute("UPDATE kv SET value='later' WHERE key='m'", [])?)).unwrap();
            take_unless_pr(&s, 1793700000, &build(channel), 14).unwrap();
            assert_eq!(marker(&b.join("orch.db")), "later");
            assert_eq!(read_backup(&b).unwrap().to_pr, 14);
        }
    }

    #[test]
    fn switching_from_one_pr_build_to_another_keeps_the_snapshot() {
        let tmp = tempfile::tempdir().unwrap();
        let s = store(tmp.path(), "before");
        take_unless_pr(&s, 1793600000, &build("main"), 12).unwrap();
        s.write(|tx| Ok(tx.execute("UPDATE kv SET value='during' WHERE key='m'", [])?)).unwrap();
        take_unless_pr(&s, 1793700000, &build("pr"), 14).unwrap();
        let b = tmp.path().join("state/backup");
        assert_eq!(marker(&b.join("orch.db")), "before");
        assert_eq!(read_backup(&b).unwrap().to_pr, 12);
        // Nothing to keep and nothing taken either.
        let fresh = tempfile::tempdir().unwrap();
        let s = store(fresh.path(), "x");
        take_unless_pr(&s, 1, &build("pr"), 14).unwrap();
        assert!(!fresh.path().join("state/backup").exists());
    }

    #[test]
    fn a_newer_store_is_replaced_by_the_snapshot_and_kept_aside() {
        let tmp = tempfile::tempdir().unwrap();
        let db = snapshot_then_newer_store(tmp.path(), "main");
        let warning = restore_if_newer(&db, &utc()).unwrap();
        // 1793600000 = 2026-11-02 06:13:20 UTC.
        let aside = tmp.path().join(format!("state/aside/orch-1793600000-schema-{}", SCHEMA_VERSION + 2));
        assert_eq!(
            warning,
            format!(
                "Your data was restored from before PR #12 (taken 06:13, 2 Nov 2026). What changed while testing it is kept in {}.",
                aside.display()
            )
        );
        // The snapshot is what the store opens with, and its blobs are back.
        assert_eq!(marker(&db), "before");
        let blobs = tmp.path().join("state/orch.blobs");
        assert_eq!(std::fs::read_to_string(blobs.join("b1")).unwrap(), "before");
        assert!(!blobs.join("b2").exists());
        // The testing session's data is kept, whole and unmigrated-down.
        assert_eq!(orch_store::schema_version(&aside.join("orch.db")).unwrap(), Some(SCHEMA_VERSION + 2));
        assert_eq!(std::fs::read_to_string(aside.join("orch.blobs/b2")).unwrap(), "new");
        // The snapshot stays (pruned by age), and a store that opens needs no restore.
        assert!(tmp.path().join("state/backup/backup.json").exists());
        assert!(restore_if_newer(&db, &utc()).is_none());
    }

    #[test]
    fn a_store_set_aside_is_never_overwritten() {
        let tmp = tempfile::tempdir().unwrap();
        let db = snapshot_then_newer_store(tmp.path(), "main");
        restore_if_newer(&db, &utc()).unwrap();
        let again = Store::open(&db).unwrap();
        migrate_ahead(again);
        restore_if_newer(&db, &utc()).unwrap();
        let aside = tmp.path().join("state/aside");
        let mut names: Vec<_> =
            std::fs::read_dir(&aside).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        names.sort();
        let base = format!("orch-1793600000-schema-{}", SCHEMA_VERSION + 2);
        assert_eq!(names, vec![base.clone(), format!("{base}-2")]);
    }

    #[test]
    fn no_restore_without_a_usable_snapshot() {
        // No snapshot: today's behaviour (the caller falls back to a scratch store).
        let tmp = tempfile::tempdir().unwrap();
        let s = store(tmp.path(), "x");
        migrate_ahead(s);
        let db = tmp.path().join("state/orch.db");
        assert!(restore_if_newer(&db, &utc()).is_none());
        assert_eq!(orch_store::schema_version(&db).unwrap(), Some(SCHEMA_VERSION + 2));
        assert!(!tmp.path().join("state/aside").exists());

        // A snapshot taken at a schema newer than this build is as unreadable as the store.
        let tmp = tempfile::tempdir().unwrap();
        let s = store(tmp.path(), "x");
        s.write(|tx| Ok(tx.pragma_update(None, "user_version", SCHEMA_VERSION + 1)?)).unwrap();
        take_unless_pr(&s, 1793600000, &build("main"), 12).unwrap();
        migrate_ahead(s);
        assert!(restore_if_newer(&tmp.path().join("state/orch.db"), &utc()).is_none());
        assert!(!tmp.path().join("state/aside").exists());
        assert_eq!(orch_store::schema_version(&tmp.path().join("state/orch.db")).unwrap(), Some(SCHEMA_VERSION + 2));
    }

    #[test]
    fn a_readable_store_is_never_replaced() {
        let tmp = tempfile::tempdir().unwrap();
        let s = store(tmp.path(), "before");
        take_unless_pr(&s, 1793600000, &build("main"), 12).unwrap();
        s.write(|tx| Ok(tx.execute("UPDATE kv SET value='live' WHERE key='m'", [])?)).unwrap();
        drop(s);
        let db = tmp.path().join("state/orch.db");
        assert!(restore_if_newer(&db, &utc()).is_none());
        assert_eq!(marker(&db), "live");
        // No database at all is fine too.
        assert!(restore_if_newer(&tmp.path().join("nowhere/orch.db"), &utc()).is_none());
    }

    #[test]
    fn only_a_main_build_drops_a_snapshot_older_than_14_days() {
        let tmp = tempfile::tempdir().unwrap();
        let s = store(tmp.path(), "x");
        take_unless_pr(&s, 1000, &build("main"), 12).unwrap();
        let db = tmp.path().join("state/orch.db");
        let backup = tmp.path().join("state/backup");
        let day = 24 * 3600;
        prune(&db, &build("main"), 1000 + 14 * day);
        assert!(backup.exists());
        for channel in ["local", "pr"] {
            prune(&db, &build(channel), 1000 + 30 * day);
            assert!(backup.exists(), "{channel}");
        }
        prune(&db, &build("main"), 1000 + 14 * day + 1);
        assert!(!backup.exists());
        // Nothing to drop is fine.
        prune(&db, &build("main"), 1000 + 30 * day);
    }
}
