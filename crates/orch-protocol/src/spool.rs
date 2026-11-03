//! The orch-hook spool: events recorded in `<agent dir>/spool.jsonl` while the
//! core was unreachable, replayed by the reconciler (J10 step 2.1).

use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::hook::HookEvent;

pub const SPOOL_FILE: &str = "spool.jsonl";

/// One spooled hook event (one JSON line).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SpoolRecord {
    pub v: u32,
    pub ts_ms: u64,
    pub agent_id: String,
    pub event: HookEvent,
    /// Why the core could not be reached: `ENOENT`, `ECONNREFUSED`, `EOF`, `deadline`…
    pub why: String,
    pub payload: serde_json::Value,
}

pub fn spool_path(agent_dir: &Path) -> PathBuf {
    agent_dir.join(SPOOL_FILE)
}

/// Appends one record. A single `write` of one line with `O_APPEND`, so
/// concurrent hooks (parallel tool calls) do not interleave.
pub fn append(agent_dir: &Path, rec: &SpoolRecord) -> std::io::Result<()> {
    let mut line = serde_json::to_vec(rec).map_err(std::io::Error::other)?;
    line.push(b'\n');
    let mut f = OpenOptions::new().create(true).append(true).open(spool_path(agent_dir))?;
    f.write_all(&line)
}

/// Reads every record; malformed lines (a torn write) are skipped and counted.
pub fn read(path: &Path) -> std::io::Result<(Vec<SpoolRecord>, usize)> {
    let f = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok((vec![], 0)),
        Err(e) => return Err(e),
    };
    let (mut recs, mut bad) = (vec![], 0);
    for line in BufReader::new(f).lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str(&line) {
            Ok(r) => recs.push(r),
            Err(_) => bad += 1,
        }
    }
    Ok((recs, bad))
}

/// A spool taken for replay. Its files stay on disk until [`Taken::done`]: a
/// core that dies mid-replay finds them again (the next [`take`] reads them
/// first, in order), and a record already applied is dropped as stale.
pub struct Taken {
    pub records: Vec<SpoolRecord>,
    pub malformed: usize,
    files: Vec<PathBuf>,
}

impl Taken {
    /// Every record was applied: the files go.
    pub fn done(self) {
        for f in self.files {
            let _ = std::fs::remove_file(f);
        }
    }
}

const REPLAYING: &str = "spool.jsonl.replaying.";

/// A spool (or one left by a replay that did not finish) waits for replay.
pub fn pending(agent_dir: &Path) -> bool {
    spool_path(agent_dir).exists() || !leftovers(agent_dir).is_empty()
}

/// The files of earlier replays that never finished, oldest first.
fn leftovers(agent_dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(agent_dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with(REPLAYING))
        .map(|e| e.path())
        .collect();
    v.sort_by_key(|p| {
        let ts: u64 =
            p.file_name().and_then(|n| n.to_str()).and_then(|n| n.strip_prefix(REPLAYING)?.parse().ok()).unwrap_or(0);
        (ts, p.clone())
    });
    v
}

/// Takes the spool for replay: renamed first, so a hook spooling meanwhile
/// starts a new file and two replays never apply the same records. The
/// records come back in order: earlier unfinished replays first, then the
/// spool (each session's order).
pub fn take(agent_dir: &Path) -> std::io::Result<Taken> {
    let mut files = leftovers(agent_dir);
    let taken = agent_dir.join(format!("{REPLAYING}{}", crate::now_ms()));
    match std::fs::rename(spool_path(agent_dir), &taken) {
        Ok(()) => files.push(taken),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    let (mut records, mut malformed) = (vec![], 0);
    for f in &files {
        let (r, bad) = read(f)?;
        records.extend(r);
        malformed += bad;
    }
    Ok(Taken { records, malformed, files })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_and_read_back() {
        let d = tempfile::tempdir().unwrap();
        for (i, ev) in [HookEvent::Stop, HookEvent::SessionEnd].into_iter().enumerate() {
            append(
                d.path(),
                &SpoolRecord {
                    v: 1,
                    ts_ms: i as u64,
                    agent_id: "a".into(),
                    event: ev,
                    why: "ECONNREFUSED".into(),
                    payload: serde_json::json!({}),
                },
            )
            .unwrap();
        }
        std::fs::OpenOptions::new().append(true).open(spool_path(d.path())).unwrap().write_all(b"{torn\n").unwrap();
        let (recs, bad) = read(&spool_path(d.path())).unwrap();
        assert_eq!(recs.len(), 2);
        assert_eq!(bad, 1);
        assert_eq!(recs[0].event, HookEvent::Stop);
        assert!(read(&d.path().join("none")).unwrap().0.is_empty());
        let taken = take(d.path()).unwrap();
        assert_eq!(taken.records.len(), 2);
        assert!(!spool_path(d.path()).exists(), "taken once");
        // A replay that dies before `done` loses nothing: the next take has the records again.
        drop(taken);
        let again = take(d.path()).unwrap();
        assert_eq!(again.records.len(), 2);
        assert!(pending(d.path()));
        again.done();
        assert!(!pending(d.path()));
        assert!(take(d.path()).unwrap().records.is_empty());
        assert_eq!(std::fs::read_dir(d.path()).unwrap().count(), 0, "nothing left behind");
    }
}
