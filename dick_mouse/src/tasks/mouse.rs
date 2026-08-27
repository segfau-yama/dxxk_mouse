#![allow(clippy::too_many_arguments)]

use ch32_hal::{
    Peri,
    adc::{Adc, Pga, SampleTime},
    gpio::{AnyPin, Input, Level, Pull},
    pac::timer::vals::FilterValue,
    peripherals::{ADC1, PA0, PA1, PA6, PA7, TIM3},
    timer::{
        Channel,
        low_level::{InputCaptureMode, InputTISelection, Timer as HalTimer},
    },
};
use dick_mouse::device::{Button, Joystick, RotaryEncoder};
use embassy_time::{Duration, Instant, Timer};

use super::{
    hid::{MOUSE_REPORTS, MouseInput},
    usb::USB_HID_POLL_MS,
};

#[embassy_executor::task]
pub(crate) async fn mouse_task(
    timer: Peri<'static, TIM3>,
    encoder_gpio_a: Peri<'static, PA6>,
    encoder_gpio_b: Peri<'static, PA7>,
    adc: Peri<'static, ADC1>,
    gpio_x: Peri<'static, PA0>,
    gpio_y: Peri<'static, PA1>,
    left_gpio: Peri<'static, AnyPin>,
    right_gpio: Peri<'static, AnyPin>,
) {
    let _encoder_input_a = Input::new(encoder_gpio_a, Pull::Up);
    let _encoder_input_b = Input::new(encoder_gpio_b, Pull::Up);
    let timer = HalTimer::new(timer);
    timer.regs_basic().atrlr().write_value(u16::MAX);
    timer.set_input_ti_selection(Channel::Ch1, InputTISelection::Normal);
    timer.set_input_ti_selection(Channel::Ch2, InputTISelection::Normal);
    timer.set_input_capture_mode(Channel::Ch1, InputCaptureMode::Rising);
    timer.set_input_capture_mode(Channel::Ch2, InputCaptureMode::Rising);
    timer.set_input_capture_filter(Channel::Ch1, FilterValue::FDTS_DIV32_N8);
    timer.set_input_capture_filter(Channel::Ch2, FilterValue::FDTS_DIV32_N8);
    timer.enable_channel(Channel::Ch1, true);
    timer.enable_channel(Channel::Ch2, true);
    timer.regs_gp16().smcfgr().modify(|w| w.set_sms(3));
    timer.start();

    let mut raw_count = timer.regs_basic().cnt().read() as i16;
    let mut count = i32::from(raw_count);
    let now_ms = Instant::now().as_millis();
    let mut encoder = RotaryEncoder::new(count, now_ms, 2);
    let mut reported_count = count;
    let left_input = Input::new(left_gpio, Pull::Up);
    let right_input = Input::new(right_gpio, Pull::Up);
    let mut left_button = Button::new(left_input.get_level(), Level::Low, 5);
    let mut right_button = Button::new(right_input.get_level(), Level::Low, 5);
    let mut x_pin = gpio_x;
    let mut y_pin = gpio_y;
    let mut adc = Adc::new(adc, Default::default());
    let center_x = adc.convert(&mut x_pin, SampleTime::CYCLES239_5, Pga::X1);
    let center_y = adc.convert(&mut y_pin, SampleTime::CYCLES239_5, Pga::X1);
    let mut joystick = Joystick::new(center_x, center_y);

    loop {
        let now_ms = Instant::now().as_millis();
        let new_raw_count = timer.regs_basic().cnt().read() as i16;
        count = count.saturating_add(i32::from(new_raw_count.wrapping_sub(raw_count)));
        raw_count = new_raw_count;
        encoder = encoder.update(count, now_ms);
        let detents = encoder.stable_count().saturating_sub(reported_count) / 4;
        if detents != 0 {
            reported_count = reported_count.saturating_add(detents.saturating_mul(4));
        }
        left_button = left_button.update(left_input.get_level(), now_ms);
        right_button = right_button.update(right_input.get_level(), now_ms);
        let raw_x = adc.convert(&mut x_pin, SampleTime::CYCLES239_5, Pga::X1);
        let raw_y = adc.convert(&mut y_pin, SampleTime::CYCLES239_5, Pga::X1);
        joystick = joystick.update(raw_x, raw_y);

        let joystick_x = joystick.x();
        let joystick_y = joystick.y().saturating_neg();
        let wheel = detents.clamp(i32::from(i8::MIN), i32::from(i8::MAX)) as i8;

        MOUSE_REPORTS
            .send(MouseInput {
                left_pressed: left_button.is_pressed(),
                right_pressed: right_button.is_pressed(),
                joystick_x,
                joystick_y,
                wheel,
            })
            .await;

        Timer::after(Duration::from_millis(u64::from(USB_HID_POLL_MS))).await;
    }
}
