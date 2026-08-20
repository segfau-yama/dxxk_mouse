#![no_std]
#![no_main]
#![allow(non_snake_case)]
#![allow(unexpected_cfgs)]

esp_bootloader_esp_idf::esp_app_desc!();

#[cfg(test)]
#[embedded_test::tests]
mod tests {
    use core::assert_eq;

    use dick_mouse::input::{Button, Led, Toggle};
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

    #[test]
    fn T04_Toggleは初期状態を保持する() {
        let toggle = Toggle::new(true, false);

        assert_eq!(toggle.values(), (true, false));
    }

    #[test]
    fn T05_Toggleは押下エッジで切り替わる() {
        let button = Button::new(pin(41), Level::Low, 5);
        let is_on = false;
        let was_pressed = false;
        let toggle = Toggle::new(is_on, was_pressed);
        let toggled = toggle.update(&button);
        let expected_is_on = if button.is_pressed() && !was_pressed {
            !is_on
        } else {
            is_on
        };

        assert_eq!(toggled.values(), (expected_is_on, button.is_pressed()));
    }

    #[test]
    fn T06_Ledはboolから出力Levelを更新する() {
        let led = Led::new(Level::Low, Level::High);
        let led_on = led.update(true);
        let led_off = led_on.update(false);

        assert_eq!(led_on.values().0, Level::High);
        assert_eq!(led_off.values().0, Level::Low);
    }

    #[test]
    fn T07_LedはToggleの状態から出力Levelを更新する() {
        let led = Led::new(Level::Low, Level::High);
        let led_on = led.update_with_toggle(Toggle::new(true, false));
        let led_off = led_on.update_with_toggle(Toggle::new(false, false));

        assert_eq!(led_on.values().0, Level::High);
        assert_eq!(led_off.values().0, Level::Low);
    }
}
