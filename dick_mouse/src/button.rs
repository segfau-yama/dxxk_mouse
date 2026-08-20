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
    pub const fn new(level: Level, active_level: Level, now_ms: u64, debounce_ms: u64) -> Self {
        Self {
            level,
            candidate_level: level,
            active_level,
            candidate_since_ms: now_ms,
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

    pub fn is_pressed(&self) -> bool {
        self.level == self.active_level
    }

    pub fn is_released(&self) -> bool {
        !self.is_pressed()
    }
}

#[cfg(test)]
mod tests {
    #![allow(non_snake_case)]

    use super::Button;
    use esp_hal::gpio::Level;

    #[test]
    fn T01_active_lowの安定値がLowなら押下になる() {
        let button = Button::new(Level::Low, Level::Low, 0, 5);

        assert!(button.is_pressed());
        assert!(!button.is_released());
    }

    #[test]
    fn T02_チャタリング時間未満では押下状態に変わらない() {
        let button = Button::new(Level::High, Level::Low, 100, 5);

        let candidate = button.update(Level::Low, 101);
        let still_released = candidate.update(Level::Low, 105);

        assert!(still_released.is_released());
        assert_eq!(still_released.level(), Level::High);
    }

    #[test]
    fn T03_チャタリング時間経過後に押下状態へ変わる() {
        let button = Button::new(Level::High, Level::Low, 100, 5);

        let candidate = button.update(Level::Low, 101);
        let pressed = candidate.update(Level::Low, 106);

        assert!(pressed.is_pressed());
        assert_eq!(pressed.level(), Level::Low);
    }

    #[test]
    fn T04_updateは元のButtonを変更せず新しいButtonを返す() {
        let button = Button::new(Level::High, Level::Low, 100, 5);

        let next_button = button.update(Level::Low, 101);
        let pressed = next_button.update(Level::Low, 106);

        assert_eq!(button.level(), Level::High);
        assert_eq!(next_button.level(), Level::High);
        assert_eq!(pressed.level(), Level::Low);
    }

    #[test]
    fn T05_debounceが0なら実測値をすぐに反映する() {
        let button = Button::new(Level::High, Level::Low, 100, 0);

        let pressed = button.update(Level::Low, 101);

        assert!(pressed.is_pressed());
        assert_eq!(pressed.level(), Level::Low);
    }
}
