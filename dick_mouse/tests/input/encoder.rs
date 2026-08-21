#![no_std]
#![no_main]
#![allow(unexpected_cfgs)]

esp_bootloader_esp_idf::esp_app_desc!();

#[cfg(test)]
#[embedded_test::tests]
mod tests {
    use core::assert_eq;

    use dick_mouse::input::RotaryEncoder;

    #[test]
    fn updateはデバウンス時間未満なら安定カウントを変えない() {
        let encoder = RotaryEncoder::new(0, 100, 2);

        let encoder = encoder.update(3, 101);
        let encoder = encoder.update(3, 102);

        assert_eq!(encoder.stable_count(), 0);
        assert_eq!(encoder.measured_count(), 3);
    }

    #[test]
    fn updateはデバウンス時間後に安定カウントを変える() {
        let encoder = RotaryEncoder::new(0, 100, 2);

        let encoder = encoder.update(3, 101);
        let encoder = encoder.update(3, 103);

        assert_eq!(encoder.stable_count(), 3);
        assert_eq!(encoder.measured_count(), 3);
    }

    #[test]
    fn detents_fromは分解能でクリック数に変換する() {
        let encoder = RotaryEncoder::new(16, 100, 2);

        assert_eq!(encoder.detents_from(8, 4), 2);
        assert_eq!(encoder.detents_from(8, 0), 0);
    }
}
