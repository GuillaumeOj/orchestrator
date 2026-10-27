//! The live-agent cap under macOS memory pressure (FR-203, NFR-1b) and the
//! idle auto-suspend rule (FR-204). Pure policy plus a probe behind a trait.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PressureLevel {
    #[default]
    Normal,
    Warn,
    Critical,
}

/// Where the memory pressure level comes from (NFR-6: behind an interface).
pub trait MemoryProbe: Send + Sync + std::fmt::Debug {
    fn level(&self) -> PressureLevel;
}

/// macOS: `kern.memorystatus_vm_pressure_level` (1 normal, 2 warn, 4 critical),
/// the level `memory_pressure` and dispatch sources report.
#[derive(Debug, Default)]
pub struct SystemMemory;

impl MemoryProbe for SystemMemory {
    fn level(&self) -> PressureLevel {
        let mut v: libc::c_int = 0;
        let mut len = std::mem::size_of::<libc::c_int>();
        let name = c"kern.memorystatus_vm_pressure_level";
        // SAFETY: sysctlbyname writes at most `len` bytes into `v`, which we own.
        let r = unsafe {
            libc::sysctlbyname(name.as_ptr(), (&mut v as *mut libc::c_int).cast(), &mut len, std::ptr::null_mut(), 0)
        };
        if r != 0 {
            return PressureLevel::Normal;
        }
        match v {
            4 => PressureLevel::Critical,
            2 => PressureLevel::Warn,
            _ => PressureLevel::Normal,
        }
    }
}

/// A fixed level (tests, or to turn the probe off).
#[derive(Debug)]
pub struct FixedMemory(pub std::sync::atomic::AtomicU8);

impl FixedMemory {
    pub fn new(level: PressureLevel) -> Self {
        let f = FixedMemory(std::sync::atomic::AtomicU8::new(0));
        f.set(level);
        f
    }

    pub fn set(&self, level: PressureLevel) {
        self.0.store(level as u8, std::sync::atomic::Ordering::SeqCst);
    }
}

impl MemoryProbe for FixedMemory {
    fn level(&self) -> PressureLevel {
        match self.0.load(std::sync::atomic::Ordering::SeqCst) {
            2 => PressureLevel::Critical,
            1 => PressureLevel::Warn,
            _ => PressureLevel::Normal,
        }
    }
}

/// FR-203: the base cap under normal pressure, one less on warn (at least 1),
/// 1 on critical. It recovers as soon as the pressure ends.
pub fn effective_cap(base: u32, level: PressureLevel) -> u32 {
    match level {
        PressureLevel::Normal => base.max(1),
        PressureLevel::Warn => base.saturating_sub(1).max(1),
        PressureLevel::Critical => 1,
    }
}

/// What the runtime knows about one live agent for the cap and idle rules.
#[derive(Clone, Debug, PartialEq)]
pub struct LiveAgent {
    pub id: String,
    pub idle: bool,
    pub background_tasks: usize,
    pub queued_turns: usize,
    pub attached: bool,
    /// Last hook event, user keystroke or delivery, ms since the epoch.
    pub last_activity_ms: u64,
    /// A team lead: its process has a slot of its own, outside the cap.
    pub reserved: bool,
    /// Its prompt cache is warm until then (ms): a reserved agent is kept
    /// that long even when idle, so the next report costs almost nothing.
    pub warm_until_ms: u64,
}

impl LiveAgent {
    /// Nothing would be lost by stopping it now (resumable by id).
    pub fn suspendable(&self) -> bool {
        self.idle && self.background_tasks == 0 && self.queued_turns == 0 && !self.attached
    }
}

/// Agents to suspend now: idle ones past `idle_after_ms`, then, while more
/// agents are live than `cap`, the least recently active idle ones. Working
/// agents are never stopped; reserved ones (team leads) don't count toward
/// the cap and are never stopped for it.
pub fn to_suspend(
    agents: &[LiveAgent],
    cap: u32,
    idle_after_ms: Option<u64>,
    now_ms: u64,
) -> Vec<(String, &'static str)> {
    let mut out: Vec<(String, &'static str)> = vec![];
    if let Some(idle) = idle_after_ms {
        let idle_for = |a: &&LiveAgent| now_ms.saturating_sub(a.last_activity_ms) >= idle;
        let cold = |a: &&LiveAgent| !a.reserved || now_ms >= a.warm_until_ms;
        for a in agents.iter().filter(|a| a.suspendable()).filter(idle_for).filter(cold) {
            out.push((a.id.clone(), "idle"));
        }
    }
    let counted = |a: &&LiveAgent| !a.reserved && !out.iter().any(|(id, _)| id == &a.id);
    let mut over = agents.iter().filter(counted).count().saturating_sub(cap as usize);
    if over > 0 {
        let mut cands: Vec<&LiveAgent> = agents.iter().filter(counted).filter(|a| a.suspendable()).collect();
        cands.sort_by_key(|a| a.last_activity_ms);
        for a in cands {
            if over == 0 {
                break;
            }
            out.push((a.id.clone(), "memory_pressure"));
            over -= 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(id: &str, idle: bool, last: u64) -> LiveAgent {
        LiveAgent {
            id: id.into(),
            idle,
            background_tasks: 0,
            queued_turns: 0,
            attached: false,
            last_activity_ms: last,
            reserved: false,
            warm_until_ms: 0,
        }
    }

    #[test]
    fn caps() {
        assert_eq!(effective_cap(3, PressureLevel::Normal), 3);
        assert_eq!(effective_cap(3, PressureLevel::Warn), 2);
        assert_eq!(effective_cap(1, PressureLevel::Warn), 1);
        assert_eq!(effective_cap(3, PressureLevel::Critical), 1);
        assert!(SystemMemory.level() <= PressureLevel::Critical);
    }

    #[test]
    fn idle_then_lru_never_working() {
        let min = 60_000;
        let now = 100 * min;
        let mut agents = vec![a("old-idle", true, now - 20 * min), a("idle", true, now - min), a("work", false, 0)];
        // Idle timer only.
        assert_eq!(to_suspend(&agents, 3, Some(10 * min), now), vec![("old-idle".to_string(), "idle")]);
        // Pressure: cap 1 → also the next idle one; the working agent stays.
        assert_eq!(
            to_suspend(&agents, 1, Some(10 * min), now),
            vec![("old-idle".to_string(), "idle"), ("idle".to_string(), "memory_pressure")]
        );
        // Attached, queued work or background tasks protect an idle agent.
        agents[1].attached = true;
        agents[0].queued_turns = 1;
        assert!(to_suspend(&agents, 1, Some(10 * min), now).is_empty());
        agents[0].queued_turns = 0;
        agents[0].background_tasks = 1;
        assert!(to_suspend(&agents, 1, None, now).is_empty());
    }

    #[test]
    fn a_reserved_lead_is_outside_the_cap() {
        let now = 1_000_000;
        let mut lead = a("lead", true, 0);
        lead.reserved = true;
        let agents = vec![lead, a("back", true, 10), a("front", false, 0)];
        // Cap 2: back and front fit, the lead doesn't count.
        assert!(to_suspend(&agents, 2, None, now).is_empty());
        // Cap 1: back goes, never the lead (older, idle).
        assert_eq!(to_suspend(&agents, 1, None, now), vec![("back".to_string(), "memory_pressure")]);
        // Idle past the timer: suspended only once its cache went cold.
        let mut warm = agents[0].clone();
        warm.warm_until_ms = now + 1;
        assert!(to_suspend(&[warm.clone()], 3, Some(10), now).is_empty());
        warm.warm_until_ms = now;
        assert_eq!(to_suspend(&[warm], 3, Some(10), now), vec![("lead".to_string(), "idle")]);
    }
}
