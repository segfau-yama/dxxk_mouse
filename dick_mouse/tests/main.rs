#![no_std]
#![no_main]
#![allow(non_snake_case)]
#![allow(unexpected_cfgs)]

esp_bootloader_esp_idf::esp_app_desc!();

#[cfg(test)]
#[embedded_test::tests]
mod tests {
    use core::assert_eq;

    use dick_mouse::input::{Button, RotaryEncoder};
    use esp_hal::{
        gpio::Level,
        interrupt::software::SoftwareInterruptControl,
        pcnt::{Pcnt, channel},
        timer::timg::TimerGroup,
    };

    #[test]
    fn T01_mainで使用するRTOS用peripheralを初期化できる() {
        let peripherals = esp_hal::init(esp_hal::Config::default());

        let sw_int = SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
        let timg0 = TimerGroup::new(peripherals.TIMG0);

        let _software_interrupt0 = sw_int.software_interrupt0;
        let _timer0 = timg0.timer0;
    }

    #[test]
    fn T02_mainで使用する左ボタンGPIOをButtonとして初期化できる() {
        let peripherals = esp_hal::init(esp_hal::Config::default());
        let button = Button::new(peripherals.GPIO41, Level::Low, 5);
        let (_, _, active_level, pending_since_ms, debounce_ms) = button.values();

        assert_eq!(active_level, Level::Low);
        assert_eq!(pending_since_ms, None);
        assert_eq!(debounce_ms, 5);
    }

    #[test]
    fn T03_mainで使用する右ボタンGPIOをButtonとして初期化できる() {
        let peripherals = esp_hal::init(esp_hal::Config::default());
        let button = Button::new(peripherals.GPIO42, Level::Low, 5);
        let (_, _, active_level, pending_since_ms, debounce_ms) = button.values();

        assert_eq!(active_level, Level::Low);
        assert_eq!(pending_since_ms, None);
        assert_eq!(debounce_ms, 5);
    }

    #[test]
    fn T04_mainで使用するスクロールホイールPCNTを初期化できる() {
        let peripherals = esp_hal::init(esp_hal::Config::default());
        let pcnt = Pcnt::new(peripherals.PCNT);
        let unit = pcnt.unit0;

        unit.set_filter(Some(800)).expect("invalid pcnt filter");

        let initial_count = unit.value() as i32;
        let encoder =
            RotaryEncoder::initial(peripherals.GPIO11, peripherals.GPIO12, initial_count, 0, 2);
        let (input_a, input_b, stable_count, measured_count, _, debounce_ms) = encoder.values();
        let signal_a = input_a.peripheral_input();
        let signal_b = input_b.peripheral_input();

        let ch0 = &unit.channel0;
        ch0.set_ctrl_signal(signal_a.clone());
        ch0.set_edge_signal(signal_b.clone());
        ch0.set_ctrl_mode(channel::CtrlMode::Reverse, channel::CtrlMode::Keep);
        ch0.set_input_mode(channel::EdgeMode::Increment, channel::EdgeMode::Decrement);

        let ch1 = &unit.channel1;
        ch1.set_ctrl_signal(signal_b);
        ch1.set_edge_signal(signal_a);
        ch1.set_ctrl_mode(channel::CtrlMode::Reverse, channel::CtrlMode::Keep);
        ch1.set_input_mode(channel::EdgeMode::Decrement, channel::EdgeMode::Increment);

        assert_eq!(stable_count, initial_count);
        assert_eq!(measured_count, initial_count);
        assert_eq!(debounce_ms, 2);
    }
}
