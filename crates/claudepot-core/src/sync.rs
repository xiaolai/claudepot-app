//! Thread-synchronization helpers shared across the SQLite stores
//! and any other site that holds a `std::sync::Mutex` and needs
//! poison-recovery semantics.
//!
//! ## Why this exists
//!
//! Six modules in this crate own a `Mutex<Connection>` (rusqlite) or
//! a `Mutex<Option<Arc<Notify>>>` and need to read its contents from
//! many call sites. The naive `.lock().expect("... mutex poisoned")`
//! shape cascades: once one caller panics while holding the mutex,
//! every subsequent acquire re-panics, taking the whole store offline
//! for the process lifetime. The right shape is to log a warn and
//! recover via `poisoned.into_inner()` — rusqlite's
//! `unchecked_transaction` rolls back on guard drop, so a panic
//! mid-transaction leaves the connection in a clean state when we
//! re-acquire here. For the runtime's `Option<Arc<Notify>>` slot
//! the recovered guard is also fine — losing visibility into a
//! prior task's exit can at worst produce a brief double-tick
//! window, which is much less bad than cascading a panic across
//! the whole live-runtime surface.
//!
//! Each affected module previously declared its own copy of this
//! helper — five-plus near-identical implementations drifted in
//! comment wording, log target, and call-site indentation. Hoist
//! into one place so future poisoning policy lives in one diff.

use std::sync::{Mutex, MutexGuard};

/// Acquire `m`, recovering from a poisoned mutex with a warn log and
/// `poisoned.into_inner()`. `target` names the resource being guarded
/// and lands in the warn message so operators can correlate the
/// recovery line with the right store.
///
/// The previous shape — `.lock().expect("... mutex poisoned")` —
/// cascades panics across every IPC command that touches a poisoned
/// store. This helper is the project-wide replacement.
pub fn recover_lock<'a, T: ?Sized>(m: &'a Mutex<T>, target: &str) -> MutexGuard<'a, T> {
    match m.lock() {
        Ok(g) => g,
        Err(poisoned) => {
            tracing::warn!(
                target = "claudepot_core::sync",
                resource = target,
                "mutex was poisoned by an earlier panic; recovering with under-lock data"
            );
            poisoned.into_inner()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    /// Happy path: the mutex isn't poisoned, the function returns the
    /// guard as `lock().unwrap()` would.
    #[test]
    fn recover_lock_returns_guard_on_clean_mutex() {
        let m = Mutex::new(42_i32);
        let g = recover_lock(&m, "test");
        assert_eq!(*g, 42);
    }

    /// Recovery path: poison the mutex via a panicking child thread,
    /// then confirm `recover_lock` still hands back a guard pointing
    /// at the same data the panicking holder left behind.
    #[test]
    fn recover_lock_returns_guard_on_poisoned_mutex() {
        let m = Arc::new(Mutex::new(99_i32));
        let m_for_panic = Arc::clone(&m);
        let join = std::thread::spawn(move || {
            let _g = m_for_panic.lock().unwrap();
            panic!("intentional panic to poison the mutex");
        });
        let _ = join.join();
        assert!(m.is_poisoned(), "test setup: mutex must be poisoned");

        // Before the fix, `.lock().expect("...")` would re-panic
        // here. `recover_lock` logs a warn and returns the guard so
        // callers can continue.
        let g = recover_lock(&m, "test");
        assert_eq!(*g, 99);
    }

    /// The returned guard is mutable when the mutex itself is so —
    /// confirm callers can still mutate after recovery, which is the
    /// shape the SQLite stores need (they run `db.execute(...)`
    /// through the guard).
    #[test]
    fn recover_lock_yields_writable_guard() {
        let m = Mutex::new(0_i32);
        {
            let mut g = recover_lock(&m, "test");
            *g = 7;
        }
        assert_eq!(*recover_lock(&m, "test"), 7);
    }
}
