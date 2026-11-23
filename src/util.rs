//! Shared helpers used across the bot and API.

use std::fmt::Write as _;
use std::future::Future;
use std::sync::LazyLock;
use tokio::runtime::{Handle, Runtime};

static FALLBACK_RT: LazyLock<Runtime> =
    LazyLock::new(|| Runtime::new().expect("failed to create fallback tokio runtime"));

/// Run a future from sync code, reusing the current multi-thread runtime when possible.
///
/// `block_in_place` is required because callers may already hold blocking work on the
/// runtime; calling `Handle::block_on` directly would panic in that case.
pub fn block_on<F: Future>(fut: F) -> F::Output {
    match Handle::try_current() {
        Ok(handle) => tokio::task::block_in_place(|| handle.block_on(fut)),
        Err(_) => FALLBACK_RT.block_on(fut),
    }
}

/// Percent-encode everything outside RFC 3986 unreserved characters.
pub fn url_encode(s: &str) -> String {
    percent_encode(s, false)
}

/// Like [`url_encode`], but leaves `/` intact for path segments.
pub fn url_encode_path(s: &str) -> String {
    percent_encode(s, true)
}

fn percent_encode(s: &str, keep_slash: bool) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric()
            || matches!(b, b'-' | b'_' | b'.' | b'~')
            || (keep_slash && b == b'/')
        {
            out.push(b as char);
        } else {
            let _ = write!(out, "%{b:02X}");
        }
    }
    out
}

/// GitHub API related constants.
pub mod github {
    /// Number of files to fetch per API page when listing PR files.
    pub const PR_FILES_PER_PAGE: usize = 100;

    /// Maximum number of pages to fetch when listing PR files (30 * 100 = 3000 files max).
    pub const MAX_PR_FILE_PAGES: usize = 30;
}

/// Parse common truthy env / dashboard flag values (`1`, `true`, `yes`, `on`).
pub fn flag_truthy(v: &str) -> bool {
    matches!(
        v.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

/// Read an env var as a boolean flag (missing/empty → false).
pub fn env_flag(name: &str) -> bool {
    std::env::var(name).ok().is_some_and(|v| flag_truthy(&v))
}

/// True when an env var holds an actual value. Compose files commonly inject
/// `${VAR:-}` placeholders, so presence alone is not enough to treat it as set.
pub fn env_set(name: &str) -> bool {
    std::env::var(name).is_ok_and(|v| !v.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_set_ignores_empty_values() {
        std::env::set_var("CODASAURUS_TEST_ENV_SET", "");
        assert!(!env_set("CODASAURUS_TEST_ENV_SET"));
        std::env::set_var("CODASAURUS_TEST_ENV_SET", "  ");
        assert!(!env_set("CODASAURUS_TEST_ENV_SET"));
        std::env::set_var("CODASAURUS_TEST_ENV_SET", "value");
        assert!(env_set("CODASAURUS_TEST_ENV_SET"));
        std::env::remove_var("CODASAURUS_TEST_ENV_SET");
        assert!(!env_set("CODASAURUS_TEST_ENV_SET"));
    }

    #[test]
    fn flag_truthy_accepts_common_forms() {
        for v in ["1", "true", "TRUE", " yes ", "on", "On"] {
            assert!(flag_truthy(v), "{v}");
        }
        for v in ["", "0", "false", "no", "off", "maybe"] {
            assert!(!flag_truthy(v), "{v}");
        }
    }
}
