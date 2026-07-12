//! Render-mode probing utilities.
//!
//! Thread-local state, set once at process startup by `main.rs` from the
//! clap-parsed `--json` flag. Deep helper functions can read `is_json()`
//! without threading a `RenderMode` through every call. See
//! [R001](../../docs/adr/R001-thread-local-json-mode.md).
//!
//! Replaces the old implementation's second `std::env::args()` scan, which
//! was polluted by the host process's command line and duplicated the
//! already-parsed `cli.json` probe.

use std::cell::Cell;

thread_local! {
    /// Process-wide JSON-mode flag. Defaults to false; `main.rs` sets it at
    /// startup from the clap parse result.
    static JSON_MODE: Cell<bool> = const { Cell::new(false) };
}

/// Set the current thread's JSON-mode flag. Called once by `main` at startup.
pub fn set_json_mode(json: bool) {
    JSON_MODE.with(|c| c.set(json));
}

/// Current thread's JSON-mode flag. Deep helper functions query this.
pub fn is_json() -> bool {
    JSON_MODE.with(|c| c.get())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_false() {
        // Defaults to false when the process has not set it. This test runs
        // on its own thread, so the TLS is clean.
        // Note: `cargo test` runs tests serially in one process; the TLS is
        // not reset between `#[test]`s. To avoid cross-test pollution we only
        // assert set/get consistency.
        set_json_mode(false);
        assert!(!is_json());
        set_json_mode(true);
        assert!(is_json());
        set_json_mode(false);
    }
}
