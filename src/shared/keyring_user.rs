//! Keyring 用户名常量。
//!
//! notion 模块（note/todo/bookmark）在 keyring 里用同一个用户名存储 token。
//! 三个模块过去各自 `const KEYRING_USER: &str = "token"`，分散维护；
//! 任何一处改都会让旧账户读不到 token。
//!
//! 统一到 `shared::keyring_user::KEYRING_USER`，单一来源。

/// Notion 模块统一 keyring username（service = `everyday/<module>/<account>`）。
pub const KEYRING_USER: &str = "token";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyring_user_is_literal_token() {
        // 锁住字面值：用户登录到 keyring 后这个常量被固化，
        // 改它意味着所有用户的现有 token 不可读。
        assert_eq!(KEYRING_USER, "token");
    }
}
