use dick_mouse::device::{Button, button::button_change};
use embassy_time::{Duration, Timer};
use esp_hal::{
    gpio::{AnyPin, Input, InputConfig, Level, Pull},
    time::Instant,
};

use super::{
    hid::{KEYBOARD_REPORTS, KeyboardInput},
    usb::USB_HID_POLL_MS,
};

#[embassy_executor::task]
pub(crate) async fn keyboard_task(
    joystick_button_gpio: AnyPin<'static>,
    back_gpio: AnyPin<'static>,
    forward_gpio: AnyPin<'static>,
) {
    let joystick_button_input = Input::new(
        joystick_button_gpio,
        InputConfig::default().with_pull(Pull::Up),
    );
    let back_input = Input::new(back_gpio, InputConfig::default().with_pull(Pull::Up));
    let forward_input = Input::new(forward_gpio, InputConfig::default().with_pull(Pull::Up));
    let mut joystick_button = Button::new(joystick_button_input.level(), Level::Low, 5);
    let mut back_button = Button::new(back_input.level(), Level::Low, 5);
    let mut forward_button = Button::new(forward_input.level(), Level::Low, 5);
    let mut first = true;

    loop {
        let now_ms = Instant::now().duration_since_epoch().as_millis();
        let joystick_changed =
            button_change(&mut joystick_button, joystick_button_input.level(), now_ms).is_some();
        let back_changed = button_change(&mut back_button, back_input.level(), now_ms).is_some();
        let forward_changed =
            button_change(&mut forward_button, forward_input.level(), now_ms).is_some();

        if first || joystick_changed || back_changed || forward_changed {
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
