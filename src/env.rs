//! Parsing for positive runtime knobs that operators may tune.
//! Zero is treated as absent so a typo cannot disable a timeout or limit.
//! Callers that intentionally accept zero keep their own local parsers.

fn parse<T: std::str::FromStr + PartialOrd + From<u8>>(raw: Option<String>, default: T) -> T {
    raw.and_then(|value| value.parse().ok())
        .filter(|value| *value > T::from(0))
        .unwrap_or(default)
}

pub fn u32(key: &str, default: u32) -> u32 {
    parse(std::env::var(key).ok(), default)
}

pub fn u64(key: &str, default: u64) -> u64 {
    parse(std::env::var(key).ok(), default)
}

pub fn usize(key: &str, default: usize) -> usize {
    parse(std::env::var(key).ok(), default)
}

#[cfg(test)]
mod tests {
    use super::parse;

    #[test]
    fn parse_falls_back_on_missing_or_invalid_values() {
        assert_eq!(parse::<u32>(None, 42), 42);
        assert_eq!(parse::<u64>(Some("not-a-number".into()), 42), 42);
        assert_eq!(parse::<usize>(Some("".into()), 42), 42);
    }

    #[test]
    fn parse_uses_positive_values() {
        assert_eq!(parse::<u32>(Some("7".into()), 42), 7);
        assert_eq!(parse::<u64>(Some("7".into()), 42), 7);
        assert_eq!(parse::<usize>(Some("7".into()), 42), 7);
    }

    #[test]
    fn zero_is_not_a_valid_timeout_or_limit() {
        assert_eq!(parse::<u32>(Some("0".into()), 42), 42);
        assert_eq!(parse::<u64>(Some("0".into()), 42), 42);
        assert_eq!(parse::<usize>(Some("0".into()), 42), 42);
    }
}
