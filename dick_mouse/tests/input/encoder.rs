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
    use esp_hal::gpio::AnyPin;

    fn pin(number: u8) -> AnyPin<'static> {
        unsafe { AnyPin::steal(number) }
    }

    #[test]
    fn T01_初期化時は安定カウントと実測カウントが一致する() {
        let encoder = RotaryEncoder::initial(pin(11), pin(12), 12, 100, 2);
        let (_, _, stable_count, measured_count, _, _) = encoder.values();

        assert_eq!(stable_count, 12);
        assert_eq!(measured_count, 12);
        assert_eq!(stable_count, measured_count);
    }

    #[test]
    fn T02_チャタリング時間未満では安定カウントが変わらない() {
        let mut encoder = RotaryEncoder::initial(pin(11), pin(12), 0, 100, 2);

        encoder.update(3, 101);
        encoder.update(3, 102);
        let (_, _, stable_count, measured_count, _, _) = encoder.values();

        assert_eq!(stable_count, 0);
        assert_eq!(measured_count, 3);
        assert_ne!(stable_count, measured_count);
    }

    #[test]
    fn T03_チャタリング時間経過後に安定カウントが変わる() {
        let mut encoder = RotaryEncoder::initial(pin(11), pin(12), 0, 100, 2);

        encoder.update(3, 101);
        encoder.update(3, 103);
        let (_, _, stable_count, measured_count, _, _) = encoder.values();

        assert_eq!(stable_count, 3);
        assert_eq!(measured_count, 3);
        assert_eq!(stable_count, measured_count);
    }

    #[test]
    fn T04_updateは実測カウントを更新する() {
        let mut encoder = RotaryEncoder::initial(pin(11), pin(12), 0, 100, 2);

        encoder.update(3, 101);
        let (_, _, stable_count, measured_count, _, _) = encoder.values();

        assert_eq!(stable_count, 0);
        assert_eq!(measured_count, 3);
    }

    #[test]
    fn T05_delta_fromは安定カウントとの差分を返す() {
        let encoder = RotaryEncoder::new(pin(11), pin(12), 12, 12, 100, 2);

        assert_eq!(encoder.delta_from(8), 4);
    }

    #[test]
    fn T06_detents_fromは分解能でクリック数に変換する() {
        let encoder = RotaryEncoder::new(pin(11), pin(12), 16, 16, 100, 2);

        assert_eq!(encoder.detents_from(8, 4), 2);
    }

    #[test]
    fn T07_counts_per_detentが0ならデテント数は0になる() {
        let encoder = RotaryEncoder::new(pin(11), pin(12), 16, 16, 100, 2);

        assert_eq!(encoder.detents_from(8, 0), 0);
    }
}
