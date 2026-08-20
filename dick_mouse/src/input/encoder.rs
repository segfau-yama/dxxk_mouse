use esp_hal::gpio::{Input, InputConfig, InputPin, Pull};

#[derive(Debug)]
pub struct RotaryEncoder<'d> {
    input_a: Input<'d>,
    input_b: Input<'d>,
    stable_count: i32,
    measured_count: i32,
    measured_since_ms: u64,
    debounce_ms: u64,
}

impl<'d> RotaryEncoder<'d> {
    pub fn new(
        gpio_a: impl InputPin + 'd,
        gpio_b: impl InputPin + 'd,
        stable_count: i32,
        measured_count: i32,
        measured_since_ms: u64,
        debounce_ms: u64,
    ) -> Self {
        let input_config = InputConfig::default().with_pull(Pull::Up);

        Self {
            input_a: Input::new(gpio_a, input_config),
            input_b: Input::new(gpio_b, input_config),
            stable_count,
            measured_count,
            measured_since_ms,
            debounce_ms,
        }
    }

    pub fn initial(
        gpio_a: impl InputPin + 'd,
        gpio_b: impl InputPin + 'd,
        count: i32,
        now_ms: u64,
        debounce_ms: u64,
    ) -> Self {
        Self::new(gpio_a, gpio_b, count, count, now_ms, debounce_ms)
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

    pub fn values(&self) -> (&Input<'d>, &Input<'d>, i32, i32, u64, u64) {
        (
            &self.input_a,
            &self.input_b,
            self.stable_count,
            self.measured_count,
            self.measured_since_ms,
            self.debounce_ms,
        )
    }

    pub fn detents_from(&self, previous_count: i32, counts_per_detent: i32) -> i32 {
        if counts_per_detent == 0 {
            return 0;
        }

        self.stable_count.saturating_sub(previous_count) / counts_per_detent
    }
}
