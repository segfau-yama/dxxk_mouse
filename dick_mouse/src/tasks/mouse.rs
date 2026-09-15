use crate::device::{Button, Joystick};
use embassy_time::{Duration, Timer};
use esp_hal::{
    analog::adc::{Adc, AdcCalLine, AdcConfig, Attenuation},
    gpio::{AnyPin, Input, InputConfig, Level, Pull},
    pcnt::unit::Unit,
    peripherals::{ADC1, GPIO1, GPIO2},
    time::Instant,
};

use super::{
    encoder,
    hid::{MOUSE_REPORTS, MouseInput},
    usb::USB_HID_POLL_MS,
};

#[embassy_executor::task]
pub async fn mouse_task(
    unit: Unit<'static, 0>,
    encoder_gpio_a: AnyPin<'static>,
    encoder_gpio_b: AnyPin<'static>,
    adc: ADC1<'static>,
    gpio_x: GPIO1<'static>,
    gpio_y: GPIO2<'static>,
    left_gpio: AnyPin<'static>,
    right_gpio: AnyPin<'static>,
) {
    let (mut encoder, mut reported_count) = encoder::setup(&unit, encoder_gpio_a, encoder_gpio_b);
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

    loop {
        let now_ms = Instant::now().duration_since_epoch().as_millis();
        let detents = encoder::detents(&unit, &mut encoder, &mut reported_count, now_ms);
        left_button = left_button.update(left_input.level(), now_ms);
        right_button = right_button.update(right_input.level(), now_ms);
        joystick = joystick.update(adc.read_blocking(&mut x_pin), adc.read_blocking(&mut y_pin));

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
