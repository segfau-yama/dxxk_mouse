#![no_std]
#![no_main]
#![allow(non_snake_case)]
#![allow(unexpected_cfgs)]

esp_bootloader_esp_idf::esp_app_desc!();

#[cfg(test)]
#[embedded_test::tests]
mod tests {
    use core::assert_eq;

    use dick_mouse::input::Button;
    use esp_hal::gpio::{AnyPin, Level};

    fn pin(number: u8) -> AnyPin<'static> {
        unsafe { AnyPin::steal(number) }
    }

    #[test]
    fn T01_Buttonはactive_levelとdebounceを保持する() {
        let button = Button::new(pin(41), Level::Low, 5);
        let (_, _, active_level, pending_since_ms, debounce_ms) = button.values();

        assert_eq!(active_level, Level::Low);
        assert_eq!(pending_since_ms, None);
        assert_eq!(debounce_ms, 5);
    }

    #[test]
    fn T02_Button_updateは次のButtonと変化有無を返す() {
        let button = Button::new(pin(41), Level::Low, 5);
        let (_, previous_level, _, _, _) = button.values();
        let (next_button, changed) = button.update(100);
        let (_, level, active_level, _, debounce_ms) = next_button.values();

        assert_eq!(active_level, Level::Low);
        assert_eq!(debounce_ms, 5);
        assert_eq!(changed, level != previous_level);
    }

    #[test]
    fn T03_Button_is_pressedは安定Levelとactive_levelの比較結果を返す() {
        let button = Button::new(pin(41), Level::Low, 5);
        let (_, level, active_level, _, _) = button.values();

        assert_eq!(button.is_pressed(), level == active_level);
    }
}
