use esp_hal::gpio::Level;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct Button {
    level: Level,
    active_level: Level,
    pending_since_ms: Option<u64>,
    debounce_ms: u64,
}

impl Button {
    pub const fn new(level: Level, active_level: Level, debounce_ms: u64) -> Self {
        Self {
            level,
            active_level,
            pending_since_ms: None,
            debounce_ms,
        }
    }

    pub fn update(self, measured_level: Level, now_ms: u64) -> (Self, bool) {
        if measured_level == self.level {
            return (
                Self {
                    pending_since_ms: None,
                    ..self
                },
                false,
            );
        }

        let pending_since_ms = self.pending_since_ms.unwrap_or(now_ms);

        if now_ms.saturating_sub(pending_since_ms) >= self.debounce_ms {
            return (
                Self {
                    level: measured_level,
                    pending_since_ms: None,
                    ..self
                },
                true,
            );
        }

        (
            Self {
                pending_since_ms: Some(pending_since_ms),
                ..self
            },
            false,
        )
    }

    pub const fn level(&self) -> Level {
        self.level
    }

    pub const fn active_level(&self) -> Level {
        self.active_level
    }

    pub const fn debounce_ms(&self) -> u64 {
        self.debounce_ms
    }

    pub fn is_pressed(&self) -> bool {
        self.level == self.active_level
    }
}
