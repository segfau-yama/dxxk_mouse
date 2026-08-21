#![no_std]
#![no_main]
#![allow(unexpected_cfgs)]

esp_bootloader_esp_idf::esp_app_desc!();

#[cfg(test)]
#[embedded_test::tests]
mod tests {
    use core::assert_eq;

    use dick_mouse::device::Joystick;

    #[test]
    fn updateは中心からの差分を軸値にする() {
        let joystick = Joystick::new(1_000, 1_000).update(1_120, 880);

        assert_eq!(joystick.x(), 120);
        assert_eq!(joystick.y(), -120);
    }

    #[test]
    fn updateはi16範囲に収める() {
        let high = Joystick::new(0, 0).update(u16::MAX, u16::MAX);
        let low = Joystick::new(u16::MAX, u16::MAX).update(0, 0);

        assert_eq!(high.x(), i16::MAX);
        assert_eq!(high.y(), i16::MAX);
        assert_eq!(low.x(), i16::MIN);
        assert_eq!(low.y(), i16::MIN);
    }
}
