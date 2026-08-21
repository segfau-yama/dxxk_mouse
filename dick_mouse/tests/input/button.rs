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
    use esp_hal::gpio::Level;

    #[test]
    fn T01_Buttonはactive_levelとdebounceを保持する() {
        let button = Button::new(Level::High, Level::Low, 5);

        assert_eq!(button.active_level(), Level::Low);
        assert_eq!(button.debounce_ms(), 5);
    }

    #[test]
    fn T02_Button_updateは次のButtonと変化有無を返す() {
        let button = Button::new(Level::High, Level::Low, 5);
        let (button, changed) = button.update(Level::Low, 100);
        let (next_button, next_changed) = button.update(Level::Low, 105);

        assert_eq!(next_button.active_level(), Level::Low);
        assert_eq!(next_button.debounce_ms(), 5);
        assert_eq!(changed, false);
        assert_eq!(next_changed, true);
        assert_eq!(next_button.level(), Level::Low);
    }

    #[test]
    fn T03_Button_is_pressedは安定Levelとactive_levelの比較結果を返す() {
        let button = Button::new(Level::Low, Level::Low, 5);

        assert_eq!(button.is_pressed(), button.level() == button.active_level());
    }
}
