use core::sync::atomic::Ordering;

use dick_mouse::device::{Button, Joystick, RotaryEncoder, button::button_change};
use embassy_time::{Duration, Timer};
use esp_hal::{
    analog::adc::{Adc, AdcCalLine, AdcConfig, Attenuation},
    gpio::{AnyPin, Input, InputConfig, Level, Pull},
    pcnt::{channel, unit::Unit},
    peripherals::{ADC1, GPIO1, GPIO2},
    time::Instant,
};
use usbd_hid::descriptor::{KeyboardUsage, MouseReport};

use super::{
    game_hid::{GAME_JOYSTICK_THRESHOLD, GAME_MODE, send_game_key},
    usb::{USB_HID_POLL_MS, USB_MOUSE_REPORTS},
};

#[embassy_executor::task]
pub(crate) async fn mouse_task(
    unit: Unit<'static, 0>,
    encoder_gpio_a: AnyPin<'static>,
    encoder_gpio_b: AnyPin<'static>,
    adc: ADC1<'static>,
    gpio_x: GPIO1<'static>,
    gpio_y: GPIO2<'static>,
    left_gpio: AnyPin<'static>,
    right_gpio: AnyPin<'static>,
) {
    let encoder_input_a = Input::new(encoder_gpio_a, InputConfig::default().with_pull(Pull::Up));
    let encoder_input_b = Input::new(encoder_gpio_b, InputConfig::default().with_pull(Pull::Up));
    let signal_a = encoder_input_a.peripheral_input();
    let signal_b = encoder_input_b.peripheral_input();

    unit.set_filter(Some(800)).expect("invalid pcnt filter");
    let ch0 = &unit.channel0;
    ch0.set_ctrl_signal(signal_a.clone());
    ch0.set_edge_signal(signal_b.clone());
    ch0.set_ctrl_mode(channel::CtrlMode::Reverse, channel::CtrlMode::Keep);
    ch0.set_input_mode(channel::EdgeMode::Increment, channel::EdgeMode::Decrement);

    let ch1 = &unit.channel1;
    ch1.set_ctrl_signal(signal_b.clone());
    ch1.set_edge_signal(signal_a.clone());
    ch1.set_ctrl_mode(channel::CtrlMode::Reverse, channel::CtrlMode::Keep);
    ch1.set_input_mode(channel::EdgeMode::Decrement, channel::EdgeMode::Increment);

    let count = unit.value() as i32;
    let now_ms = Instant::now().duration_since_epoch().as_millis();
    let mut encoder = RotaryEncoder::new(count, now_ms, 2);
    let mut reported_count = count;
    let left_input = Input::new(left_gpio, InputConfig::default().with_pull(Pull::Up));
    let right_input = Input::new(right_gpio, InputConfig::default().with_pull(Pull::Up));
    let mut left_button = Button::new(left_input.level(), Level::Low, 5);
    let mut right_button = Button::new(right_input.level(), Level::Low, 5);
    let mut adc_config = AdcConfig::new();
    let mut x_pin =
        adc_config.enable_pin_with_cal::<_, AdcCalLine<ADC1<'static>>>(gpio_x, Attenuation::_11dB);
    let mut y_pin =
        adc_config.enable_pin_with_cal::<_, AdcCalLine<ADC1<'static>>>(gpio_y, Attenuation::_11dB);
    let mut adc = Adc::new(adc, adc_config);
    let mut joystick = Joystick::new(adc.read_blocking(&mut x_pin), adc.read_blocking(&mut y_pin));
    let mut reported_buttons = 0;
    let mut game_mode_was = false;

    loop {
        let now_ms = Instant::now().duration_since_epoch().as_millis();
        let game_mode = GAME_MODE.load(Ordering::Relaxed);
        encoder = encoder.update(unit.value() as i32, now_ms);
        let detents = encoder.stable_count().saturating_sub(reported_count) / 4;
        if detents != 0 {
            reported_count = reported_count.saturating_add(detents.saturating_mul(4));
        }
        let _ = button_change(&mut left_button, left_input.level(), now_ms);
        let _ = button_change(&mut right_button, right_input.level(), now_ms);
        joystick = joystick.update(adc.read_blocking(&mut x_pin), adc.read_blocking(&mut y_pin));

        if game_mode != game_mode_was {
            reported_buttons = 0;
            game_mode_was = game_mode;
        }

        let buttons =
            u8::from(left_button.is_pressed()) | (u8::from(right_button.is_pressed()) << 1);
        let joystick_x = joystick.x();
        let joystick_y = joystick.y().saturating_neg();
        let x = (joystick_x / 256) as i8;
        let y = (joystick_y / 256) as i8;
        let wheel = detents.clamp(i32::from(i8::MIN), i32::from(i8::MAX)) as i8;

        if game_mode {
            for (key, pressed) in [
                (
                    KeyboardUsage::KeyboardUpArrow,
                    joystick_y < -GAME_JOYSTICK_THRESHOLD,
                ),
                (
                    KeyboardUsage::KeyboardDownArrow,
                    joystick_y > GAME_JOYSTICK_THRESHOLD,
                ),
                (
                    KeyboardUsage::KeyboardLeftArrow,
                    joystick_x < -GAME_JOYSTICK_THRESHOLD,
                ),
                (
                    KeyboardUsage::KeyboardRightArrow,
                    joystick_x > GAME_JOYSTICK_THRESHOLD,
                ),
                (KeyboardUsage::KeyboardAa, left_button.is_pressed()),
                (KeyboardUsage::KeyboardDd, right_button.is_pressed()),
            ] {
                send_game_key(key, pressed).await;
            }
        } else if buttons != reported_buttons || x != 0 || y != 0 || wheel != 0 {
            USB_MOUSE_REPORTS
                .send(MouseReport {
                    buttons,
                    x,
                    y,
                    wheel,
                    pan: 0,
                })
                .await;
            reported_buttons = buttons;
        }

        Timer::after(Duration::from_millis(u64::from(USB_HID_POLL_MS))).await;
    }
}
