//! Numeric environment variables, read the same way everywhere (as
//! [`crate::watchdog::WatchdogConfig::from_env`] does): a missing, empty,
//! zero or unparsable value means "use the default".

/// `raw` as a positive integer, or `None`.
pub(crate) fn positive(raw: Option<&str>) -> Option<u64> {
    raw.and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|&n| n > 0)
}

/// [`positive`] of the process variable `name`.
pub(crate) fn var(name: &str) -> Option<u64> {
    positive(std::env::var(name).ok().as_deref())
}

#[cfg(test)]
mod tests {
    use super::positive;

    #[test]
    fn zero_empty_and_junk_mean_default() {
        assert_eq!(positive(None), None);
        assert_eq!(positive(Some("")), None);
        assert_eq!(positive(Some("0")), None);
        assert_eq!(positive(Some("-3")), None);
        assert_eq!(positive(Some("1.5")), None);
        assert_eq!(positive(Some("ten")), None);
        assert_eq!(positive(Some(" 42 ")), Some(42));
    }
}
