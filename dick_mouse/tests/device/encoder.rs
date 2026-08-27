#![no_std]
#![no_main]
#![allow(unexpected_cfgs)]

#[cfg(test)]
#[embedded_test::tests]
mod tests {
    use core::assert_eq;

    use dick_mouse::device::RotaryEncoder;

    #[test]
    fn updateはデバウンス時間未満なら安定カウントを変えない() {
        let encoder = RotaryEncoder::new(0, 100, 2);

        let encoder = encoder.update(3, 101);
        let encoder = encoder.update(3, 102);

        assert_eq!(encoder.stable_count(), 0);
        assert_eq!(encoder.measured_count(), 3);
    }

    #[test]
    fn updateはデバウンス時間後に安定カウントを変える() {
        let encoder = RotaryEncoder::new(0, 100, 2);

        let encoder = encoder.update(3, 101);
        let encoder = encoder.update(3, 103);

        assert_eq!(encoder.stable_count(), 3);
        assert_eq!(encoder.measured_count(), 3);
    }
}
