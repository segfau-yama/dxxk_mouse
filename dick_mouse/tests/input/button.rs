#![no_std]
#![no_main]
#![allow(unexpected_cfgs)]

esp_bootloader_esp_idf::esp_app_desc!();

#[cfg(test)]
#[embedded_test::tests]
mod tests {
    use core::assert_eq;

    use dick_mouse::input::Button;
    use esp_hal::gpio::Level;

    #[test]
    fn updateはデバウンス後に押下状態へ変わる() {
        let button = Button::new(Level::High, Level::Low, 5);
        let (button, changed) = button.update(Level::Low, 100);
        let (next_button, next_changed) = button.update(Level::Low, 105);

        assert_eq!(changed, false);
        assert_eq!(next_changed, true);
        assert_eq!(next_button.level(), Level::Low);
        assert!(next_button.is_pressed());
    }
}
