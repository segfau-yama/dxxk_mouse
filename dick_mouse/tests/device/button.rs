#![no_std]
#![no_main]
#![allow(unexpected_cfgs)]

#[cfg(test)]
#[embedded_test::tests]
mod tests {
    use core::assert_eq;

    use ch32_hal::gpio::Level;
    use dick_mouse::device::Button;

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
