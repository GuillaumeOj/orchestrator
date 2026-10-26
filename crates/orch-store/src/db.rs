//! The database handle: WAL, one writer thread, one reader connection.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use anyhow::{Context, Result, anyhow};
use rusqlite::{Connection, OpenFlags, Transaction};

use crate::migrations;

/// A write transaction handed to [`Store::write`] closures.
pub type Tx<'a> = Transaction<'a>;

type Job = Box<dyn FnOnce(&mut Connection) + Send>;

/// The core's SQLite database (ADR-005, NFR-8). Cheap to clone; every clone
/// talks to the same writer thread.
///
/// - **Writes** run on a single writer thread, one transaction per call:
///   [`Store::write`] waits for the commit, [`Store::write_detached`] does not
///   (events, samples).
/// - **Reads** use a separate connection (WAL: readers never block the writer).
/// - `synchronous=NORMAL`: a crash of the process never loses a committed
///   transaction (a power loss can lose the last ones).
#[derive(Clone)]
pub struct Store {
    inner: Arc<Inner>,
}

struct Inner {
    path: PathBuf,
    tx: Mutex<Option<SyncSender<Job>>>,
    reader: Mutex<Connection>,
    writer: Mutex<Option<JoinHandle<()>>>,
}

impl Drop for Inner {
    fn drop(&mut self) {
        // Closing the channel ends the writer thread once queued jobs are done.
        self.tx.lock().unwrap_or_else(|e| e.into_inner()).take();
        if let Some(h) = self.writer.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = h.join();
        }
    }
}

fn configure(c: &Connection) -> rusqlite::Result<()> {
    c.busy_timeout(std::time::Duration::from_secs(5))?;
    c.pragma_update(None, "foreign_keys", "ON")?;
    Ok(())
}

impl Store {
    /// Opens (or creates) the database and runs pending migrations.
    pub fn open(path: &Path) -> Result<Store> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        let mut w = Connection::open(path).with_context(|| format!("opening {}", path.display()))?;
        let mode: String = w.query_row("PRAGMA journal_mode=WAL", [], |r| r.get(0))?;
        if !mode.eq_ignore_ascii_case("wal") {
            return Err(anyhow!("{}: journal_mode is {mode}, not WAL", path.display()));
        }
        w.pragma_update(None, "synchronous", "NORMAL")?;
        configure(&w)?;
        migrations::migrate(&mut w)?;
        let reader = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX | OpenFlags::SQLITE_OPEN_URI,
        )?;
        configure(&reader)?;
        let (tx, rx): (SyncSender<Job>, Receiver<Job>) = sync_channel(1024);
        let writer = std::thread::Builder::new().name("orch-store-writer".into()).spawn(move || {
            let mut conn = w;
            while let Ok(job) = rx.recv() {
                job(&mut conn);
            }
            let _ = conn.execute_batch("PRAGMA optimize");
        })?;
        Ok(Store {
            inner: Arc::new(Inner {
                path: path.to_path_buf(),
                tx: Mutex::new(Some(tx)),
                reader: Mutex::new(reader),
                writer: Mutex::new(Some(writer)),
            }),
        })
    }

    pub fn path(&self) -> &Path {
        &self.inner.path
    }

    /// Folder for payloads too large for a row (NFR-8: large outputs outside the rows).
    pub fn blob_dir(&self) -> PathBuf {
        self.inner.path.with_extension("blobs")
    }

    fn send(&self, job: Job) -> Result<()> {
        let g = self.inner.tx.lock().unwrap_or_else(|e| e.into_inner());
        g.as_ref().ok_or_else(|| anyhow!("store closed"))?.send(job).map_err(|_| anyhow!("store writer stopped"))
    }

    /// Runs `f` in one write transaction on the writer thread and waits for the
    /// commit. An error rolls the transaction back.
    pub fn write<R, F>(&self, f: F) -> Result<R>
    where
        R: Send + 'static,
        F: FnOnce(&Tx) -> Result<R> + Send + 'static,
    {
        let (rtx, rrx) = sync_channel::<Result<R>>(1);
        self.send(Box::new(move |conn: &mut Connection| {
            let r = (|| {
                let tx = conn.transaction()?;
                let r = f(&tx)?;
                tx.commit()?;
                Ok(r)
            })();
            let _ = rtx.send(r);
        }))?;
        rrx.recv().map_err(|_| anyhow!("store writer stopped"))?
    }

    /// Like [`Store::write`] without waiting: for append-only data (events,
    /// samples) where the caller must never block on disk. Errors are dropped.
    pub fn write_detached<F>(&self, f: F)
    where
        F: FnOnce(&Tx) -> Result<()> + Send + 'static,
    {
        let _ = self.send(Box::new(move |conn: &mut Connection| {
            if let Ok(tx) = conn.transaction() {
                if f(&tx).is_ok() {
                    let _ = tx.commit();
                }
            }
        }));
    }

    /// Waits until every write queued before this call is committed.
    pub fn flush(&self) -> Result<()> {
        self.write(|_| Ok(()))
    }

    /// Runs `f` on the reader connection (sees every committed write).
    pub fn read<R>(&self, f: impl FnOnce(&Connection) -> Result<R>) -> Result<R> {
        let c = self.inner.reader.lock().unwrap_or_else(|e| e.into_inner());
        f(&c)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wal_writer_and_reader() {
        let d = tempfile::tempdir().unwrap();
        let s = Store::open(&d.path().join("db/orch.db")).unwrap();
        let mode: String = s.read(|c| Ok(c.query_row("PRAGMA journal_mode", [], |r| r.get(0))?)).unwrap();
        assert_eq!(mode, "wal");
        s.write(|tx| Ok(tx.execute("INSERT INTO kv(key, value) VALUES ('a', '1')", [])?)).unwrap();
        let v: String = s.read(|c| Ok(c.query_row("SELECT value FROM kv WHERE key='a'", [], |r| r.get(0))?)).unwrap();
        assert_eq!(v, "1");
        // A failed closure rolls back.
        let e = s.write(|tx| {
            tx.execute("INSERT INTO kv(key, value) VALUES ('b', '2')", [])?;
            Err::<(), _>(anyhow!("boom"))
        });
        assert!(e.is_err());
        let n: i64 = s.read(|c| Ok(c.query_row("SELECT count(*) FROM kv WHERE key='b'", [], |r| r.get(0))?)).unwrap();
        assert_eq!(n, 0);
        // Detached writes are ordered before a later flush.
        for i in 0..50 {
            s.write_detached(move |tx| {
                tx.execute("INSERT INTO kv(key, value) VALUES (?1, 'x')", [format!("k{i}")])?;
                Ok(())
            });
        }
        s.flush().unwrap();
        let n: i64 =
            s.read(|c| Ok(c.query_row("SELECT count(*) FROM kv WHERE key LIKE 'k%'", [], |r| r.get(0))?)).unwrap();
        assert_eq!(n, 50);
    }

    #[test]
    fn concurrent_writers_are_serialized() {
        let d = tempfile::tempdir().unwrap();
        let s = Store::open(&d.path().join("orch.db")).unwrap();
        let hs: Vec<_> = (0..8)
            .map(|t| {
                let s = s.clone();
                std::thread::spawn(move || {
                    for i in 0..25 {
                        s.write(move |tx| {
                            tx.execute("INSERT INTO kv(key, value) VALUES (?1, 'v')", [format!("{t}-{i}")])?;
                            Ok(())
                        })
                        .unwrap();
                    }
                })
            })
            .collect();
        for h in hs {
            h.join().unwrap();
        }
        let n: i64 = s.read(|c| Ok(c.query_row("SELECT count(*) FROM kv", [], |r| r.get(0))?)).unwrap();
        assert_eq!(n, 200);
    }
}
