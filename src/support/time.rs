//! Длительности в ответах: единственный перевод в миллисекунды.

use std::time::{Duration, Instant};

/// Длительность в целых миллисекундах; то, что не помещается, — `u64::MAX`.
pub fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

/// Сколько миллисекунд прошло с `started`.
pub fn elapsed_ms(started: Instant) -> u64 {
    millis(started.elapsed())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_duration_too_long_for_u64_millis_is_capped() {
        assert_eq!(millis(Duration::from_millis(1500)), 1500);
        assert_eq!(millis(Duration::MAX), u64::MAX);
    }
}
