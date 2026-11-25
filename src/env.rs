//! Shared environment parsing for runtime knobs that operators may tune.

fn parse<T: std::str::FromStr + PartialOrd + From<u8>>(key: &str, default: T) -> T {
    std::env::var(key)
        .ok()
        .and_then(|value| value.parse().ok())
        .filter(|value| *value > T::from(0))
        .unwrap_or(default)
}

pub fn u32(key: &str, default: u32) -> u32 {
    parse(key, default)
}

pub fn u64(key: &str, default: u64) -> u64 {
    parse(key, default)
}

pub fn usize(key: &str, default: usize) -> usize {
    parse(key, default)
}

#[cfg(test)]
mod tests {
    use super::{u32, u64, usize as usize_value};

    #[test]
    fn env_helpers_fall_back_on_missing_or_invalid_values() {
        let key = "CODASAURUS_TEST_ENV_HELPER_INVALID";
        std::env::set_var(key, "not-a-number");
        assert_eq!(u32(key, 42), 42);
        assert_eq!(u64(key, 42), 42);
        assert_eq!(usize_value(key, 42), 42);
        std::env::remove_var(key);

        assert_eq!(u32(key, 42), 42);
    }

    #[test]
    fn env_helpers_use_positive_values() {
        let key = "CODASAURUS_TEST_ENV_HELPER_POSITIVE";
        std::env::set_var(key, "7");
        assert_eq!(u32(key, 42), 7);
        assert_eq!(u64(key, 42), 7);
        assert_eq!(usize_value(key, 42), 7);
        std::env::remove_var(key);
    }

    #[test]
    fn zero_is_not_a_valid_timeout_or_limit() {
        let key = "CODASAURUS_TEST_ENV_HELPER_ZERO";
        std::env::set_var(key, "0");
        assert_eq!(u32(key, 42), 42);
        assert_eq!(u64(key, 42), 42);
        assert_eq!(usize_value(key, 42), 42);
        std::env::remove_var(key);
    }
}
