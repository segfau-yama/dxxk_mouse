#![no_std]
#![no_main]
#![allow(non_snake_case)]
#![allow(unexpected_cfgs)]

esp_bootloader_esp_idf::esp_app_desc!();

#[cfg(test)]
#[embedded_test::tests]
mod tests {
    use core::{assert_eq, assert_ne};

    use dick_mouse::input::RotaryEncoder;

    #[test]
    fn T01_初期化時は安定カウントと実測カウントが一致する() {
        let encoder = RotaryEncoder::new(12, 100, 2);

        assert_eq!(encoder.stable_count(), 12);
        assert_eq!(encoder.measured_count(), 12);
        assert_eq!(encoder.stable_count(), encoder.measured_count());
    }

    #[test]
    fn T02_チャタリング時間未満では安定カウントが変わらない() {
        let encoder = RotaryEncoder::new(0, 100, 2);

        let encoder = encoder.update(3, 101);
        let encoder = encoder.update(3, 102);

        assert_eq!(encoder.stable_count(), 0);
        assert_eq!(encoder.measured_count(), 3);
        assert_ne!(encoder.stable_count(), encoder.measured_count());
    }

    #[test]
    fn T03_チャタリング時間経過後に安定カウントが変わる() {
        let encoder = RotaryEncoder::new(0, 100, 2);

        let encoder = encoder.update(3, 101);
        let encoder = encoder.update(3, 103);

        assert_eq!(encoder.stable_count(), 3);
        assert_eq!(encoder.measured_count(), 3);
        assert_eq!(encoder.stable_count(), encoder.measured_count());
    }

    #[test]
    fn T04_updateは実測カウントを更新する() {
        let encoder = RotaryEncoder::new(0, 100, 2);

        let encoder = encoder.update(3, 101);

        assert_eq!(encoder.stable_count(), 0);
        assert_eq!(encoder.measured_count(), 3);
    }

    #[test]
    fn T05_detents_fromは分解能でクリック数に変換する() {
        let encoder = RotaryEncoder::new(16, 100, 2);

        assert_eq!(encoder.detents_from(8, 4), 2);
    }

    #[test]
    fn T06_counts_per_detentが0ならデテント数は0になる() {
        let encoder = RotaryEncoder::new(16, 100, 2);

        assert_eq!(encoder.detents_from(8, 0), 0);
    }
}
