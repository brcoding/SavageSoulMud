//! Input-guard mechanics for the telnet read task: the trip reason,
//! line framing, the rate window, and bounded remainder discard.
//!
//! Pure functions and task-local state live here so the framing boundary
//! is unit testable without a socket; `bridge` owns the tasks that call
//! them. Split out of `bridge.rs` under the module line cap.

use std::collections::VecDeque;
use std::time::Duration;

use tokio::time::Instant;

/// Why an input guard dropped a connection. Logged with the peer address on
/// the Bevy side so guard trips are distinguishable from clean disconnects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum GuardTrip {
    /// One line ran past the buffer cap without producing a newline.
    BufferExceeded { bytes: usize },
    /// More than the allowed lines arrived inside the rate window.
    RateExceeded { lines: usize, window_secs: u64 },
}

impl std::fmt::Display for GuardTrip {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::BufferExceeded { bytes } => {
                write!(f, "input line exceeded {bytes} bytes without a newline")
            }
            Self::RateExceeded { lines, window_secs } => {
                write!(f, "more than {lines} lines in {window_secs}s")
            }
        }
    }
}

/// Frame already Telnet-decoded bytes into session text: lossily decode
/// UTF-8 (a multibyte split by truncation degrades to �, never panics), and
/// trim the line ending.
pub(crate) fn frame_input(buf: &[u8]) -> String {
    String::from_utf8_lossy(buf)
        .trim_end_matches(['\r', '\n'])
        .to_string()
}

/// Record one delivered line and report whether the rate guard trips:
/// more than `max` lines still inside `window`. Task-local sliding window.
pub(crate) fn rate_tripped(
    line_times: &mut VecDeque<Instant>,
    window: Duration,
    max: usize,
) -> bool {
    let now = Instant::now();
    while line_times
        .front()
        .is_some_and(|t| now.duration_since(*t) > window)
    {
        line_times.pop_front();
    }
    line_times.push_back(now);
    line_times.len() > max
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frame_input_trims_crlf() {
        assert_eq!(frame_input(b"hello\r\n"), "hello");
        assert_eq!(frame_input(b"hello\n"), "hello");
    }

    #[test]
    fn frame_input_truncated_multibyte_never_panics() {
        // "aé" cut to 2 bytes: 'a' plus the lead byte of é. Lossy
        // decode degrades the split char instead of panicking.
        let cut = &"aé".as_bytes()[..2];
        assert_eq!(frame_input(cut), "a�");
    }

    #[test]
    fn rate_trips_past_max() {
        let mut times = VecDeque::new();
        let window = Duration::from_secs(60);
        assert!(!rate_tripped(&mut times, window, 2));
        assert!(!rate_tripped(&mut times, window, 2));
        assert!(rate_tripped(&mut times, window, 2));
    }

    #[test]
    fn rate_window_prunes_stale_entries() {
        let mut times = VecDeque::new();
        times.push_back(
            Instant::now()
                .checked_sub(Duration::from_secs(120))
                .unwrap(),
        );
        let window = Duration::from_secs(60);
        assert!(!rate_tripped(&mut times, window, 5));
        assert_eq!(times.len(), 1);
    }

    #[test]
    fn guard_trip_displays_with_limits() {
        assert_eq!(
            GuardTrip::BufferExceeded { bytes: 64 }.to_string(),
            "input line exceeded 64 bytes without a newline"
        );
        assert_eq!(
            GuardTrip::RateExceeded {
                lines: 5,
                window_secs: 60
            }
            .to_string(),
            "more than 5 lines in 60s"
        );
    }
}
