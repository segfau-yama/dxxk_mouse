use esp_hal::gpio::Level;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct Button {
    level: Level,
    active_level: Level,
    pending_since_ms: Option<u64>,
    debounce_ms: u64,
    changed: bool,
}

impl Button {
    pub const fn new(level: Level, active_level: Level, debounce_ms: u64) -> Self {
        Self {
            level,
            active_level,
            pending_since_ms: None,
            debounce_ms,
            changed: false,
        }
    }

    pub fn update(self, measured_level: Level, now_ms: u64) -> Self {
        if measured_level == self.level {
            return Self {
                pending_since_ms: None,
                changed: false,
                ..self
            };
        }

        let pending_since_ms = self.pending_since_ms.unwrap_or(now_ms);

        if now_ms.saturating_sub(pending_since_ms) >= self.debounce_ms {
            return Self {
                level: measured_level,
                pending_since_ms: None,
                changed: true,
                ..self
            };
        }

        Self {
            pending_since_ms: Some(pending_since_ms),
            changed: false,
            ..self
        }
    }

    pub const fn level(&self) -> Level {
        self.level
    }

    pub fn is_pressed(&self) -> bool {
        self.level == self.active_level
    }

    pub const fn changed(&self) -> bool {
        self.changed
    }
}
