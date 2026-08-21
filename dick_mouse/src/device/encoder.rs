#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct RotaryEncoder {
    stable_count: i32,
    measured_count: i32,
    measured_since_ms: u64,
    debounce_ms: u64,
}

impl RotaryEncoder {
    pub const fn new(count: i32, now_ms: u64, debounce_ms: u64) -> Self {
        Self {
            stable_count: count,
            measured_count: count,
            measured_since_ms: now_ms,
            debounce_ms,
        }
    }

    pub fn update(self, measured_count: i32, now_ms: u64) -> Self {
        if measured_count != self.measured_count {
            return Self {
                measured_count,
                measured_since_ms: now_ms,
                ..self
            };
        }

        if measured_count != self.stable_count
            && now_ms.saturating_sub(self.measured_since_ms) >= self.debounce_ms
        {
            return Self {
                stable_count: measured_count,
                ..self
            };
        }

        self
    }

    pub const fn stable_count(&self) -> i32 {
        self.stable_count
    }

    pub const fn measured_count(&self) -> i32 {
        self.measured_count
    }

    pub fn detents_from(&self, previous_count: i32, counts_per_detent: i32) -> i32 {
        if counts_per_detent == 0 {
            return 0;
        }

        self.stable_count.saturating_sub(previous_count) / counts_per_detent
    }
}
