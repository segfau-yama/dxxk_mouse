#![no_std]
#![no_main]
#![allow(unexpected_cfgs)]

esp_bootloader_esp_idf::esp_app_desc!();

#[cfg(test)]
#[embedded_test::tests]
mod tests {
    use core::assert_eq;

    use dick_mouse::device::{Button, Joystick, RotaryEncoder};
    use esp_hal::gpio::Level;

    #[test]
    fn public_reexportsを利用できる() {
        let button = Button::new(Level::Low, Level::Low, 5);
        let encoder = RotaryEncoder::new(4, 0, 2);
        let joystick = Joystick::new(10, 10).update(12, 8);

        assert!(button.is_pressed());
        assert_eq!(encoder.stable_count(), 4);
        assert_eq!(joystick.x(), 2);
    }
}
