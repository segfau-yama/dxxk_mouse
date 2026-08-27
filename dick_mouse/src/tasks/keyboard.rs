use ch32_hal::{
    Peri,
    gpio::{AnyPin, Input, Level, Pull},
};
use dick_mouse::device::Button;
use embassy_time::{Duration, Instant, Timer};

use super::{
    hid::{KEYBOARD_REPORTS, KeyboardInput},
    usb::USB_HID_POLL_MS,
};

#[embassy_executor::task]
pub(crate) async fn keyboard_task(
    joystick_button_gpio: Peri<'static, AnyPin>,
    back_gpio: Peri<'static, AnyPin>,
    forward_gpio: Peri<'static, AnyPin>,
) {
    let joystick_button_input = Input::new(joystick_button_gpio, Pull::Up);
    let back_input = Input::new(back_gpio, Pull::Up);
    let forward_input = Input::new(forward_gpio, Pull::Up);
    let mut joystick_button = Button::new(joystick_button_input.get_level(), Level::Low, 5);
    let mut back_button = Button::new(back_input.get_level(), Level::Low, 5);
    let mut forward_button = Button::new(forward_input.get_level(), Level::Low, 5);
    let mut first = true;

    loop {
        let now_ms = Instant::now().as_millis();
        joystick_button = joystick_button.update(joystick_button_input.get_level(), now_ms);
        back_button = back_button.update(back_input.get_level(), now_ms);
        forward_button = forward_button.update(forward_input.get_level(), now_ms);

        if first || joystick_button.changed() || back_button.changed() || forward_button.changed() {
            first = false;
            KEYBOARD_REPORTS
                .send(KeyboardInput {
                    joystick_pressed: joystick_button.is_pressed(),
                    back_pressed: back_button.is_pressed(),
                    forward_pressed: forward_button.is_pressed(),
                })
                .await;
        }

        Timer::after(Duration::from_millis(u64::from(USB_HID_POLL_MS))).await;
    }
}
