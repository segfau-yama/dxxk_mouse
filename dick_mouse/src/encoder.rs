#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct RotaryEncoder {
    stable_count: i32,
    candidate_count: i32,
    candidate_since_ms: u64,
    debounce_ms: u64,
}

impl RotaryEncoder {
    pub const fn new(count: i32, now_ms: u64, debounce_ms: u64) -> Self {
        Self {
            stable_count: count,
            candidate_count: count,
            candidate_since_ms: now_ms,
            debounce_ms,
        }
    }

    pub fn update(self, measured_count: i32, now_ms: u64) -> Self {
        if measured_count != self.candidate_count {
            return Self {
                candidate_count: measured_count,
                candidate_since_ms: now_ms,
                ..self
            };
        }

        if measured_count != self.stable_count
            && now_ms.saturating_sub(self.candidate_since_ms) >= self.debounce_ms
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

    pub const fn candidate_count(&self) -> i32 {
        self.candidate_count
    }

    pub const fn is_chattering(&self) -> bool {
        self.stable_count != self.candidate_count
    }
}

#[cfg(test)]
mod tests {
    #![allow(non_snake_case)]

    use super::RotaryEncoder;

    #[test]
    fn T01_初期化時は安定カウントと候補カウントが一致する() {
        let encoder = RotaryEncoder::new(12, 100, 2);

        assert_eq!(encoder.stable_count(), 12);
        assert_eq!(encoder.candidate_count(), 12);
        assert!(!encoder.is_chattering());
    }

    #[test]
    fn T02_チャタリング時間未満では安定カウントが変わらない() {
        let encoder = RotaryEncoder::new(0, 100, 2);

        let candidate = encoder.update(3, 101);
        let still_stable = candidate.update(3, 102);

        assert_eq!(still_stable.stable_count(), 0);
        assert_eq!(still_stable.candidate_count(), 3);
        assert!(still_stable.is_chattering());
    }

    #[test]
    fn T03_チャタリング時間経過後に安定カウントが変わる() {
        let encoder = RotaryEncoder::new(0, 100, 2);

        let candidate = encoder.update(3, 101);
        let stable = candidate.update(3, 103);

        assert_eq!(stable.stable_count(), 3);
        assert_eq!(stable.candidate_count(), 3);
        assert!(!stable.is_chattering());
    }

    #[test]
    fn T04_updateは元のRotaryEncoderを変更せず新しいインスタンスを返す() {
        let encoder = RotaryEncoder::new(0, 100, 2);

        let next_encoder = encoder.update(3, 101);

        assert_eq!(encoder.stable_count(), 0);
        assert_eq!(encoder.candidate_count(), 0);
        assert_eq!(next_encoder.stable_count(), 0);
        assert_eq!(next_encoder.candidate_count(), 3);
    }
}
