//! Pure Rust port of `Duration` from `@deepseek-ai/dsh-util/time`.
//!
//! Source: `packages/util/time/src/duration.ts` in
//! deepseek-ai/deepseek-harness (MIT), read as untrusted reference data.
//! A signed millisecond span with explicit decomposition semantics.

use crate::Error;

/// A signed millisecond duration (i64 range).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Duration(i64);

impl Duration {
    /// Build a duration, clamping to i64 range (never fails).
    pub fn from_millis(millis: i64) -> Self {
        Self(millis)
    }

    /// Build with an explicit range check. `MAX_SAFE_MILLIS` mirrors upstream's
    /// safe-integer ceiling; anything outside is rejected.
    pub const MAX_SAFE_MILLIS: i64 = 9_007_199_254_740_991;

    pub fn try_from_millis(millis: i64) -> std::result::Result<Self, Error> {
        if !(-Self::MAX_SAFE_MILLIS..=Self::MAX_SAFE_MILLIS).contains(&millis) {
            return Err(Error::Other(format!("duration {millis}ms out of range")));
        }
        Ok(Self(millis))
    }

    pub fn as_millis(self) -> i64 {
        self.0
    }

    /// Decompose into calendar-style units. Negative spans can be built but
    /// cannot be decomposed (sign policy is the caller's responsibility).
    pub fn decompose(self) -> std::result::Result<Decomposed, Error> {
        if self.0 < 0 {
            return Err(Error::Other(
                "cannot decompose a negative duration".to_string(),
            ));
        }
        let seconds_total = self.0 / 1000;
        let milliseconds = (self.0 % 1000) as u32;
        let minutes_total = seconds_total / 60;
        let seconds = (seconds_total % 60) as u32;
        let hours_total = minutes_total / 60;
        let minutes = (minutes_total % 60) as u32;
        let days = hours_total / 24;
        let hours = (hours_total % 24) as u32;
        Ok(Decomposed {
            days: days as u64,
            hours,
            minutes,
            seconds,
            milliseconds,
        })
    }

    /// Render a compact human-readable span (e.g. `1d 2h 3m 4s 5ms`),
    /// omitting zero units.
    pub fn render(self) -> std::result::Result<String, Error> {
        let parts = self.decompose()?;
        let mut out = Vec::new();
        if parts.days > 0 {
            out.push(format!("{}d", parts.days));
        }
        if parts.hours > 0 {
            out.push(format!("{}h", parts.hours));
        }
        if parts.minutes > 0 {
            out.push(format!("{}m", parts.minutes));
        }
        if parts.seconds > 0 {
            out.push(format!("{}s", parts.seconds));
        }
        if parts.milliseconds > 0 || out.is_empty() {
            out.push(format!("{}ms", parts.milliseconds));
        }
        Ok(out.join(" "))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Decomposed {
    pub days: u64,
    pub hours: u32,
    pub minutes: u32,
    pub seconds: u32,
    pub milliseconds: u32,
}
