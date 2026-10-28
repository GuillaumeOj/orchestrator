//! FR-816: the power assertion held while off-hours work is queued or
//! running, on AC only. The real one is a `caffeinate -s -w <core pid>`
//! child (`-s` only holds on AC power; `-w` ends it with the core), the
//! same subprocess style as `pmset` for the power source. Tests use [`FakePower`].

use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use anyhow::{Context, Result};

pub trait PowerAssertion: Send + Sync + std::fmt::Debug {
    /// Prevents idle system sleep (no-op when already held).
    fn acquire(&self) -> Result<()>;
    fn release(&self);
    fn held(&self) -> bool;
}

/// `caffeinate -s -w <pid>`.
#[derive(Debug, Default)]
pub struct Caffeinate {
    child: Mutex<Option<Child>>,
}

impl PowerAssertion for Caffeinate {
    fn acquire(&self) -> Result<()> {
        let mut c = self.child.lock().unwrap_or_else(|e| e.into_inner());
        if c.as_mut().is_some_and(|ch| matches!(ch.try_wait(), Ok(None))) {
            return Ok(());
        }
        let child = Command::new("/usr/bin/caffeinate")
            .args(["-s", "-w", &std::process::id().to_string()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .context("caffeinate")?;
        *c = Some(child);
        Ok(())
    }

    fn release(&self) {
        if let Some(mut ch) = self.child.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = ch.kill();
            let _ = ch.wait();
        }
    }

    fn held(&self) -> bool {
        self.child
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_mut()
            .is_some_and(|ch| matches!(ch.try_wait(), Ok(None)))
    }
}

impl Drop for Caffeinate {
    fn drop(&mut self) {
        self.release();
    }
}

/// For tests: counts acquisitions and releases.
#[derive(Debug, Default)]
pub struct FakePower {
    held: AtomicBool,
    pub acquired: AtomicU32,
    pub released: AtomicU32,
}

impl PowerAssertion for FakePower {
    fn acquire(&self) -> Result<()> {
        if !self.held.swap(true, Ordering::SeqCst) {
            self.acquired.fetch_add(1, Ordering::SeqCst);
        }
        Ok(())
    }

    fn release(&self) {
        if self.held.swap(false, Ordering::SeqCst) {
            self.released.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn held(&self) -> bool {
        self.held.load(Ordering::SeqCst)
    }
}
