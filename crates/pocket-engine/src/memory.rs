use std::collections::HashMap;
use std::sync::Mutex;

/// Per A§4.2 memory rules. Tracks loaded handles and their estimated resident
/// bytes against a budget (bytes, not a `fit()` call — CORE-08's catalog/fit
/// formulas are a separate concern; this guard only needs byte accounting).
pub struct MemoryGuard {
    budget_bytes: u64,
    handles: Mutex<HashMap<String, HandleState>>,
}

#[derive(Clone)]
struct HandleState {
    bytes: u64,
    busy: bool,
    last_used_ms: i64,
}

#[derive(Debug, PartialEq, Eq, Clone)]
pub enum Admission {
    Ok,
    EvictThenOk(Vec<String>),
    Refuse(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThermalState {
    Nominal,
    Fair,
    Serious,
    Critical,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvictReason {
    MemoryPressure,
    IdleEviction,
}

impl MemoryGuard {
    pub fn new(budget_bytes: u64) -> Self {
        Self {
            budget_bytes,
            handles: Mutex::new(HashMap::new()),
        }
    }

    fn resident_total(handles: &HashMap<String, HandleState>) -> u64 {
        handles.values().map(|h| h.bytes).sum()
    }

    /// `admit` decides whether loading `estimate` bytes fits: it fits outright
    /// (`Ok`), fits after evicting LRU idle handles (`EvictThenOk`), or
    /// doesn't fit even after evicting everything idle (`Refuse`). Never
    /// evicts a busy (mid-generation) handle.
    pub fn admit(&self, estimate: u64, now_ms: i64) -> Admission {
        let mut handles = self.handles.lock().unwrap();
        let current = Self::resident_total(&handles);
        if current + estimate <= self.budget_bytes {
            return Admission::Ok;
        }

        let mut idle: Vec<(String, HandleState)> = handles
            .iter()
            .filter(|(_, s)| !s.busy)
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        idle.sort_by_key(|(_, s)| s.last_used_ms);

        let mut freed = 0u64;
        let mut to_evict = Vec::new();
        for (handle, state) in &idle {
            if current - freed + estimate <= self.budget_bytes {
                break;
            }
            to_evict.push(handle.clone());
            freed += state.bytes;
        }

        if current - freed + estimate <= self.budget_bytes {
            for handle in &to_evict {
                handles.remove(handle);
            }
            let _ = now_ms;
            Admission::EvictThenOk(to_evict)
        } else {
            Admission::Refuse(format!(
                "needs {estimate} bytes, only {} available even after evicting all idle models",
                self.budget_bytes.saturating_sub(current - freed)
            ))
        }
    }

    pub fn mark_loaded(&self, handle: &str, bytes: u64, now_ms: i64) {
        self.handles.lock().unwrap().insert(
            handle.to_string(),
            HandleState {
                bytes,
                busy: false,
                last_used_ms: now_ms,
            },
        );
    }

    pub fn mark_busy(&self, handle: &str, busy: bool) {
        if let Some(state) = self.handles.lock().unwrap().get_mut(handle) {
            state.busy = busy;
        }
    }

    pub fn touch(&self, handle: &str, now_ms: i64) {
        if let Some(state) = self.handles.lock().unwrap().get_mut(handle) {
            state.last_used_ms = now_ms;
        }
    }

    pub fn is_loaded(&self, handle: &str) -> bool {
        self.handles.lock().unwrap().contains_key(handle)
    }

    /// Unloads every idle handle, returning their ids so the caller can emit
    /// `model.evicted` events and tell the engine to actually unload them.
    pub fn on_memory_warning(&self) -> Vec<String> {
        let mut handles = self.handles.lock().unwrap();
        let idle: Vec<String> = handles
            .iter()
            .filter(|(_, s)| !s.busy)
            .map(|(k, _)| k.clone())
            .collect();
        for handle in &idle {
            handles.remove(handle);
        }
        idle
    }

    /// Serious/Critical thermal states cap generation to one concurrent
    /// stream; the caller enforces the cap, this just reports it.
    pub fn concurrency_cap(&self, thermal: ThermalState) -> usize {
        match thermal {
            ThermalState::Serious | ThermalState::Critical => 1,
            ThermalState::Nominal | ThermalState::Fair => usize::MAX,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admits_when_under_budget() {
        let guard = MemoryGuard::new(1_000);
        assert_eq!(guard.admit(500, 0), Admission::Ok);
    }

    #[test]
    fn refuses_when_nothing_to_evict_and_still_over_budget() {
        let guard = MemoryGuard::new(1_000);
        assert!(matches!(guard.admit(2_000, 0), Admission::Refuse(_)));
    }

    #[test]
    fn evicts_lru_idle_handle_then_admits() {
        let guard = MemoryGuard::new(1_000);
        guard.mark_loaded("old", 600, 0);
        guard.mark_loaded("new", 300, 10);
        // 600 + 300 = 900 resident; admitting 200 more needs evicting "old" (LRU).
        let admission = guard.admit(200, 20);
        assert_eq!(admission, Admission::EvictThenOk(vec!["old".to_string()]));
        assert!(!guard.is_loaded("old"));
        assert!(guard.is_loaded("new"));
    }

    #[test]
    fn never_evicts_a_busy_handle() {
        let guard = MemoryGuard::new(1_000);
        guard.mark_loaded("busy", 900, 0);
        guard.mark_busy("busy", true);
        // Nothing idle to evict even though "busy" alone would free enough room.
        assert!(matches!(guard.admit(500, 10), Admission::Refuse(_)));
    }

    #[test]
    fn on_memory_warning_unloads_all_idle_but_not_busy() {
        let guard = MemoryGuard::new(1_000);
        guard.mark_loaded("idle1", 100, 0);
        guard.mark_loaded("idle2", 100, 0);
        guard.mark_loaded("busy", 100, 0);
        guard.mark_busy("busy", true);

        let mut evicted = guard.on_memory_warning();
        evicted.sort();
        assert_eq!(evicted, vec!["idle1".to_string(), "idle2".to_string()]);
        assert!(guard.is_loaded("busy"));
        assert!(!guard.is_loaded("idle1"));
    }

    #[test]
    fn thermal_caps_concurrency() {
        let guard = MemoryGuard::new(1_000);
        assert_eq!(guard.concurrency_cap(ThermalState::Nominal), usize::MAX);
        assert_eq!(guard.concurrency_cap(ThermalState::Serious), 1);
        assert_eq!(guard.concurrency_cap(ThermalState::Critical), 1);
    }
}
