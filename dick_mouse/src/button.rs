use esp_hal::gpio::Level;

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct Button {
    level: Level,
    candidate_level: Level,
    active_level: Level,
    candidate_since_ms: u64,
    debounce_ms: u64,
}

impl Button {
    pub const fn new(
        level: Level,
        candidate_level: Level,
        active_level: Level,
        candidate_since_ms: u64,
        debounce_ms: u64,
    ) -> Self {
        Self {
            level,
            candidate_level,
            active_level,
            candidate_since_ms,
            debounce_ms,
        }
    }

    pub fn update(self, measured_level: Level, now_ms: u64) -> Self {
        if measured_level != self.candidate_level {
            return Self {
                level: if self.debounce_ms == 0 {
                    measured_level
                } else {
                    self.level
                },
                candidate_level: measured_level,
                candidate_since_ms: now_ms,
                ..self
            };
        }

        if measured_level != self.level
            && now_ms.saturating_sub(self.candidate_since_ms) >= self.debounce_ms
        {
            return Self {
                level: measured_level,
                ..self
            };
        }

        self
    }

    pub const fn level(&self) -> Level {
        self.level
    }

    pub const fn is_pressed(&self) -> bool {
        matches!(
            (self.level, self.active_level),
            (Level::High, Level::High) | (Level::Low, Level::Low)
        )
    }

    pub const fn is_released(&self) -> bool {
        !self.is_pressed()
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct Toggle {
    is_on: bool,
    was_pressed: bool,
}

impl Toggle {
    pub const fn new(is_on: bool, was_pressed: bool) -> Self {
        Self { is_on, was_pressed }
    }

    pub fn update(self, button: Button) -> Self {
        let is_pressed = button.is_pressed();
        let is_on = if is_pressed && !self.was_pressed {
            !self.is_on
        } else {
            self.is_on
        };

        Self {
            is_on,
            was_pressed: is_pressed,
        }
    }

    pub const fn is_on(&self) -> bool {
        self.is_on
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct Led {
    level: Level,
    active_level: Level,
}

impl Led {
    pub const fn new(level: Level, active_level: Level) -> Self {
        Self {
            level,
            active_level,
        }
    }

    pub fn update(self, is_on: bool) -> Self {
        Self {
            level: if is_on {
                self.active_level
            } else {
                !self.active_level
            },
            ..self
        }
    }

    pub fn update_with_button(self, button: Button) -> Self {
        self.update(button.is_pressed())
    }

    pub fn update_with_toggle(self, toggle: Toggle) -> Self {
        self.update(toggle.is_on())
    }

    pub const fn level(&self) -> Level {
        self.level
    }
}
