use std::sync::OnceLock;

pub const DEFAULT_LOG_BUFFER_CAPACITY: usize = 50_000;
const LOG_BUFFER_CAPACITY_ENV: &str = "CATPANE_LOG_BUFFER_CAPACITY";

pub fn log_buffer_capacity() -> usize {
    static CAPACITY: OnceLock<usize> = OnceLock::new();

    *CAPACITY.get_or_init(|| match std::env::var(LOG_BUFFER_CAPACITY_ENV) {
        Ok(raw) => match raw.trim().parse::<usize>() {
            Ok(value) if value > 0 => value,
            _ => {
                eprintln!(
                    "Invalid {LOG_BUFFER_CAPACITY_ENV}={raw:?}; using default {}",
                    DEFAULT_LOG_BUFFER_CAPACITY
                );
                DEFAULT_LOG_BUFFER_CAPACITY
            }
        },
        Err(_) => DEFAULT_LOG_BUFFER_CAPACITY,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_capacity_is_positive() {
        assert!(DEFAULT_LOG_BUFFER_CAPACITY > 0);
        assert!(log_buffer_capacity() > 0);
    }
}
