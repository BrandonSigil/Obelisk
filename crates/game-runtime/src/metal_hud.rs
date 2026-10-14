//! The frames a game presented, as Metal's Performance HUD reports them. Every
//! game starts with the HUD on but invisible; once per second Metal then writes
//! a `metal-HUD:` line to the presenting process's standard error, which the
//! game's log keeps. Read once the game has exited, those lines give its frame
//! rate whatever runs it: Rosetta's Wine, the Linux engine's wsd or a native
//! build.
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs::File,
    io::{self, BufRead, BufReader},
    path::Path,
};

/// What turns the HUD on for a process without showing it or its menu.
pub(crate) const ENVIRONMENT: [(&str, &str); 4] = [
    ("MTL_HUD_ENABLED", "1"),
    ("MTL_HUD_LOG_ENABLED", "1"),
    ("MTL_HUD_OPACITY", "0"),
    ("MTL_HUD_DISABLE_MENU_BAR", "1"),
];

const MARKER: &[u8] = b"metal-HUD: ";
/// A frame shown this long after the previous one is a hitch the player sees.
const HITCH_MS: f32 = 100.0;
/// Rates above this, such as an uncapped menu's, are reported as this.
const MAX_FPS: f64 = 1000.0;

/// The frames of the process that presented the most of them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Frames {
    pub count: u64,
    /// The median present interval, in microseconds.
    pub median_us: Option<u32>,
    /// The 99th percentile present interval, in microseconds: the slowest
    /// 1 % of frames took at least this long.
    pub slowest_percent_us: Option<u32>,
    /// Frames shown more than 100 ms after the previous one.
    pub hitches: u64,
}
impl Frames {
    /// Frames per second at the median interval, to a tenth.
    pub fn median_fps(&self) -> Option<f64> {
        self.median_us.map(rate)
    }
    /// Frames per second the slowest 1 % of frames stay under, to a tenth.
    pub fn low_1pct_fps(&self) -> Option<f64> {
        self.slowest_percent_us.map(rate)
    }
}

/// The frames `log` reports. A log that cannot be read reports none.
pub fn read(log: &Path) -> Frames {
    File::open(log)
        .and_then(|file| parse(BufReader::new(file)))
        .unwrap_or_default()
}

fn parse(log: impl BufRead) -> io::Result<Frames> {
    // Present intervals in ms, per process: a game's launcher may present too.
    let mut intervals: HashMap<u32, Vec<f32>> = HashMap::new();
    for line in log.split(b'\n') {
        let line = line?;
        let Some(at) = line.windows(MARKER.len()).position(|w| w == MARKER) else {
            continue;
        };
        let values = String::from_utf8_lossy(&line[at + MARKER.len()..]);
        // <first frame>,<GPU MB>,<process MB>, then <interval>,<GPU time> per frame.
        let process = intervals.entry(pid(&line[..at])).or_default();
        process.extend(
            values
                .trim_end()
                .split(',')
                .skip(3)
                .step_by(2)
                .filter_map(|value| value.parse::<f32>().ok())
                .filter(|interval| interval.is_finite() && *interval > 0.0),
        );
    }
    let mut intervals = intervals
        .into_values()
        .max_by_key(Vec::len)
        .unwrap_or_default();
    Ok(frames(&mut intervals))
}

/// The process ID in the `name[pid:thread]` that precedes the marker; 0 when
/// the line has none.
fn pid(prefix: &[u8]) -> u32 {
    let prefix = String::from_utf8_lossy(prefix);
    prefix
        .rfind('[')
        .and_then(|open| prefix[open + 1..].split(':').next()?.parse().ok())
        .unwrap_or(0)
}

fn frames(intervals: &mut [f32]) -> Frames {
    if intervals.is_empty() {
        return Frames::default();
    }
    intervals.sort_by(f32::total_cmp);
    let count = intervals.len();
    let middle = count / 2;
    let median = if count.is_multiple_of(2) {
        (intervals[middle - 1] + intervals[middle]) / 2.0
    } else {
        intervals[middle]
    };
    // Nearest rank: the smallest interval at least 99 % of frames don't exceed.
    let p99 = intervals[(count * 99).div_ceil(100) - 1];
    Frames {
        count: count as u64,
        median_us: Some(microseconds(median)),
        slowest_percent_us: Some(microseconds(p99)),
        hitches: intervals.iter().filter(|&&i| i > HITCH_MS).count() as u64,
    }
}

fn microseconds(interval_ms: f32) -> u32 {
    (f64::from(interval_ms) * 1000.0).round() as u32
}

/// Frames per second for a present interval, to a tenth.
fn rate(interval_us: u32) -> f64 {
    ((1_000_000.0 / f64::from(interval_us.max(1))).min(MAX_FPS) * 10.0).round() / 10.0
}

#[cfg(test)]
mod tests;
