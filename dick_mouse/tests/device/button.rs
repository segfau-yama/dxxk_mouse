#![no_std]
#![no_main]
#![allow(unexpected_cfgs)]

esp_bootloader_esp_idf::esp_app_desc!();

#[cfg(test)]
#[embedded_test::tests]
mod tests {
    use core::assert_eq;

    use dick_mouse::device::Button;
    use esp_hal::gpio::Level;

    #[test]
    fn updateはデバウンス後に押下状態へ変わる() {
        let button = Button::new(Level::High, Level::Low, 5);
        let button = button.update(Level::Low, 100);

        assert!(!button.changed());

        let button = button.update(Level::Low, 105);

        assert!(button.changed());
        assert_eq!(button.level(), Level::Low);
        assert!(button.is_pressed());

        let button = button.update(Level::Low, 106);
        assert!(!button.changed());
    }
}
