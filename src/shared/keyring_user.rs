//! Keyring username constant.
//!
//! The Notion modules (note/todo/bookmark) store their tokens under the same
//! keyring username. They used to each declare their own
//! `const KEYRING_USER: &str = "token"`, scattered across files — changing
//! any one would make existing accounts unreadable.
//!
//! Centralized here as `shared::keyring_user::KEYRING_USER`, single source of truth.
//! See [F002](../../docs/adr/F002-multi-account-keyring.md).

/// Shared keyring username for Notion modules (service = `everyday/<module>/<account>`).
pub const KEYRING_USER: &str = "token";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyring_user_is_literal_token() {
        // Lock the literal: once a user logs in, this constant is baked into
        // their keyring entry; changing it would make every existing token unreadable.
        assert_eq!(KEYRING_USER, "token");
    }
}
