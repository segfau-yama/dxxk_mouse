use esp_hal::gpio::{Input, InputConfig, InputPin, Level, Pull};

#[derive(Debug)]
pub struct Button<'d> {
    input: Input<'d>,
    level: Level,
    active_level: Level,
    pending_since_ms: Option<u64>,
    debounce_ms: u64,
}

impl<'d> Button<'d> {
    pub fn new(gpio: impl InputPin + 'd, active_level: Level, debounce_ms: u64) -> Self {
        let input = Input::new(gpio, InputConfig::default().with_pull(Pull::Up));
        let level = input.level();

        Self {
            input,
            level,
            active_level,
            pending_since_ms: None,
            debounce_ms,
        }
    }

    pub fn update(self, now_ms: u64) -> (Self, bool) {
        let measured_level = self.input.level();

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

    pub fn values(&self) -> (&Input<'d>, Level, Level, Option<u64>, u64) {
        (
            &self.input,
            self.level,
            self.active_level,
            self.pending_since_ms,
            self.debounce_ms,
        )
    }

    pub fn is_pressed(&self) -> bool {
        self.level == self.active_level
    }
}
