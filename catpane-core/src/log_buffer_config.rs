use std::sync::OnceLock;

pub const DEFAULT_LOG_BUFFER_CAPACITY: usize = 50_000;
pub const DEFAULT_INITIAL_LOG_BACKLOG: usize = 2_000;
const LOG_BUFFER_CAPACITY_ENV: &str = "CATPANE_LOG_BUFFER_CAPACITY";
const INITIAL_LOG_BACKLOG_ENV: &str = "CATPANE_INITIAL_LOG_BACKLOG";

pub fn log_buffer_capacity() -> usize {
    static CAPACITY: OnceLock<usize> = OnceLock::new();

    *CAPACITY.get_or_init(|| {
        parse_positive_usize_env(LOG_BUFFER_CAPACITY_ENV, DEFAULT_LOG_BUFFER_CAPACITY)
    })
}

pub fn initial_log_backlog() -> usize {
    static BACKLOG: OnceLock<usize> = OnceLock::new();

    *BACKLOG.get_or_init(|| {
        parse_positive_usize_env(INITIAL_LOG_BACKLOG_ENV, DEFAULT_INITIAL_LOG_BACKLOG)
            .min(log_buffer_capacity())
    })
}

fn parse_positive_usize_env(name: &str, default: usize) -> usize {
    match std::env::var(name) {
        Ok(raw) => match raw.trim().parse::<usize>() {
            Ok(value) if value > 0 => value,
            _ => {
                eprintln!("Invalid {name}={raw:?}; using default {default}");
                default
            }
        },
        Err(_) => default,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_capacity_is_positive() {
        assert!(DEFAULT_LOG_BUFFER_CAPACITY > 0);
        assert!(log_buffer_capacity() > 0);
    }

    #[test]
    fn default_initial_backlog_is_positive() {
        assert!(DEFAULT_INITIAL_LOG_BACKLOG > 0);
        assert!(initial_log_backlog() > 0);
    }

    #[test]
    fn default_initial_backlog_does_not_exceed_capacity() {
        assert!(DEFAULT_INITIAL_LOG_BACKLOG <= DEFAULT_LOG_BUFFER_CAPACITY);
        assert!(initial_log_backlog() <= log_buffer_capacity());
    }
}
