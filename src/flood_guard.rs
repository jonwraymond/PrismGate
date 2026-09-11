//! Rolling discovery budgets, isolated by transport session and agent context.

use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Normal,
    SoftCapped,
    HardBlocked { retry_after: Duration },
}

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

type ContextKey = (u64, Option<String>);

impl Decision {
    /// Enforce the decision before doing discovery work.
    pub fn limit(self, requested: u32) -> Result<u32, Self> {
        match self {
            Self::Normal => Ok(requested),
            Self::SoftCapped => Ok(requested.min(1)),
            Self::HardBlocked { .. } => Err(self),
        }
    }

    pub fn notice(self) -> Option<String> {
        match self {
            Self::Normal => None,
            Self::SoftCapped => Some("Discovery flood guard: soft cap for this agent context. search_tools returns at most 1 result per query; tool_info uses brief detail. Reuse discovered tools with call_tool_chain.".into()),
            Self::HardBlocked { retry_after } => Some(format!(
                "Discovery flood guard: search_tools/tool_info blocked for this agent context. Retry after {} seconds; reuse already discovered tools with call_tool_chain instead of repeating discovery.",
                retry_after.as_secs() + u64::from(retry_after.subsec_nanos() > 0)
            )),
        }
    }
}

pub struct FloodGuard {
    calls: Mutex<HashMap<ContextKey, VecDeque<Instant>>>,
    soft: usize,
    hard: usize,
    window: Duration,
}

impl Default for FloodGuard {
    fn default() -> Self {
        // First 10 calls normal; calls 11–20 capped; subsequent calls blocked.
        Self::new(10, 20, Duration::from_secs(60))
    }
}

impl FloodGuard {
    pub fn check(&self, session: u64, agent: Option<&str>) -> Decision {
        self.check_at(session, agent, Instant::now())
    }

    pub fn new(soft: usize, hard: usize, window: Duration) -> Self {
        assert!(soft < hard && !window.is_zero());
        Self {
            calls: Mutex::new(HashMap::new()),
            soft,
            hard,
            window,
        }
    }

    fn check_at(&self, session: u64, agent: Option<&str>, now: Instant) -> Decision {
        let mut calls = self.calls.lock().unwrap_or_else(|e| e.into_inner());
        // Prune idle contexts as well as expired calls. Blocked attempts do not
        // extend the window or grow a context's queue beyond the hard limit.
        calls.retain(|_, history| {
            while history
                .front()
                .is_some_and(|at| now.duration_since(*at) >= self.window)
            {
                history.pop_front();
            }
            !history.is_empty()
        });
        let history = calls
            .entry((session, agent.map(str::to_owned)))
            .or_default();
        if history.len() >= self.hard {
            return Decision::HardBlocked {
                retry_after: self.window - now.duration_since(history[0]),
            };
        }
        history.push_back(now);
        if history.len() > self.soft {
            Decision::SoftCapped
        } else {
            Decision::Normal
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hard_blocked_and_isolated() {
        let guard = FloodGuard::new(1, 2, Duration::from_secs(60));
        let now = Instant::now();
        guard.check_at(1, Some("a"), now);
        guard.check_at(1, Some("a"), now);
        assert_eq!(
            guard.check_at(1, Some("a"), now),
            Decision::HardBlocked {
                retry_after: Duration::from_secs(60)
            }
        );
        assert_eq!(guard.check_at(1, Some("b"), now), Decision::Normal);
        assert_eq!(guard.check_at(2, Some("a"), now), Decision::Normal);
        assert_eq!(guard.check_at(1, None, now), Decision::Normal);
    }

    #[test]
    fn rolling_window_reset_and_retry() {
        let guard = FloodGuard::new(1, 2, Duration::from_secs(60));
        let now = Instant::now();
        guard.check_at(1, None, now);
        guard.check_at(1, None, now + Duration::from_secs(30));
        assert_eq!(
            guard.check_at(1, None, now + Duration::from_secs(59)),
            Decision::HardBlocked {
                retry_after: Duration::from_secs(1)
            }
        );
        assert_eq!(
            guard.check_at(1, None, now + Duration::from_secs(60)),
            Decision::SoftCapped
        );
        assert_eq!(
            guard.check_at(1, None, now + Duration::from_secs(120)),
            Decision::Normal
        );
        assert_eq!(guard.calls.lock().unwrap().len(), 1);
    }

    #[test]
    fn limits_notices_and_parallel_budget() {
        assert_eq!(Decision::Normal.limit(10), Ok(10));
        assert_eq!(Decision::SoftCapped.limit(10), Ok(1));
        assert_eq!(Decision::SoftCapped.limit(0), Ok(0));
        assert!(Decision::Normal.notice().is_none());
        assert!(Decision::SoftCapped.notice().unwrap().contains("1 result"));
        let blocked = Decision::HardBlocked {
            retry_after: Duration::from_millis(1),
        };
        assert_eq!(blocked.limit(10), Err(blocked));
        assert!(blocked.notice().unwrap().contains("1 seconds"));
        let guard = std::sync::Arc::new(FloodGuard::default());
        let handles: Vec<_> = (0..40)
            .map(|_| {
                let guard = guard.clone();
                std::thread::spawn(move || guard.check(1, Some("a")))
            })
            .collect();
        let decisions: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert_eq!(
            decisions.iter().filter(|d| **d == Decision::Normal).count(),
            10
        );
        assert_eq!(
            decisions
                .iter()
                .filter(|d| **d == Decision::SoftCapped)
                .count(),
            10
        );
        assert_eq!(
            decisions
                .iter()
                .filter(|d| matches!(d, Decision::HardBlocked { .. }))
                .count(),
            20
        );
        assert_eq!(guard.check(1, Some("b")), Decision::Normal);
    }

    #[test]
    fn normal_then_soft_capped() {
        let guard = FloodGuard::new(2, 4, Duration::from_secs(60));
        let now = Instant::now();
        assert_eq!(guard.check_at(1, Some("a"), now), Decision::Normal);
        assert_eq!(guard.check_at(1, Some("a"), now), Decision::Normal);
        assert_eq!(guard.check_at(1, Some("a"), now), Decision::SoftCapped);
    }
}
