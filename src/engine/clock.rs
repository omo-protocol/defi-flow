/// Simulation clock that advances through sorted timestamps from data sources.
pub struct SimClock {
    timestamps: Vec<u64>,
    current_idx: usize,
}

impl SimClock {
    pub fn new(mut timestamps: Vec<u64>) -> Self {
        timestamps.sort();
        timestamps.dedup();
        Self {
            timestamps,
            current_idx: 0,
        }
    }

    /// Create a clock with evenly spaced ticks (for backtests without CSV data).
    /// `start` and `end` are unix timestamps, `step` is seconds between ticks.
    pub fn uniform(start: u64, end: u64, step: u64) -> Self {
        let timestamps: Vec<u64> = (start..=end).step_by(step as usize).collect();
        Self {
            timestamps,
            current_idx: 0,
        }
    }

    pub fn current_timestamp(&self) -> u64 {
        self.timestamps
            .get(self.current_idx)
            .copied()
            .unwrap_or(0)
    }

    /// Advance to the next tick. Returns false when exhausted.
    pub fn advance(&mut self) -> bool {
        if self.current_idx + 1 < self.timestamps.len() {
            self.current_idx += 1;
            true
        } else {
            false
        }
    }

    pub fn tick_index(&self) -> usize {
        self.current_idx
    }

    pub fn total_ticks(&self) -> usize {
        self.timestamps.len()
    }

    /// Seconds elapsed since the previous tick (0 for the first tick).
    pub fn dt_seconds(&self) -> u64 {
        if self.current_idx == 0 {
            return 0;
        }
        self.timestamps[self.current_idx] - self.timestamps[self.current_idx - 1]
    }

    /// First timestamp in the series.
    pub fn first_timestamp(&self) -> u64 {
        self.timestamps.first().copied().unwrap_or(0)
    }

    /// Last timestamp in the series.
    pub fn last_timestamp(&self) -> u64 {
        self.timestamps.last().copied().unwrap_or(0)
    }
}
