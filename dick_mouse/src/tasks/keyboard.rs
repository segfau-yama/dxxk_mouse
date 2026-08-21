use core::sync::atomic::Ordering;

use dick_mouse::device::Button;
use embassy_time::{Duration, Timer};
use esp_hal::{
    gpio::{AnyPin, Input, InputConfig, Level, Pull},
    time::Instant,
};
use usbd_hid::descriptor::{KeyboardReport, KeyboardUsage};

use crate::{GAME_MODE, USB_HID_POLL_MS, USB_KEYBOARD_REPORTS, button_change, send_game_key};

#[embassy_executor::task]
pub(crate) async fn keyboard_task(
    screenshot_gpio: AnyPin<'static>,
    back_gpio: AnyPin<'static>,
    forward_gpio: AnyPin<'static>,
) {
    let screenshot_input = Input::new(screenshot_gpio, InputConfig::default().with_pull(Pull::Up));
    let back_input = Input::new(back_gpio, InputConfig::default().with_pull(Pull::Up));
    let forward_input = Input::new(forward_gpio, InputConfig::default().with_pull(Pull::Up));
    let mut screenshot_button = Button::new(screenshot_input.level(), Level::Low, 5);
    let mut back_button = Button::new(back_input.level(), Level::Low, 5);
    let mut forward_button = Button::new(forward_input.level(), Level::Low, 5);
    let mut game_mode_was = false;

    loop {
        let now_ms = Instant::now().duration_since_epoch().as_millis();
        let game_mode = GAME_MODE.load(Ordering::Relaxed);

        if game_mode && game_mode != game_mode_was {
            for (key, pressed) in [
                (KeyboardUsage::KeyboardSs, screenshot_button.is_pressed()),
                (KeyboardUsage::KeyboardSpacebar, back_button.is_pressed()),
                (KeyboardUsage::KeyboardEnter, forward_button.is_pressed()),
            ] {
                if pressed {
                    send_game_key(key, pressed).await;
                }
            }
        }
        game_mode_was = game_mode;

        for (button, input, report, game_key) in [
            (
                &mut screenshot_button,
                &screenshot_input,
                KeyboardReport {
                    modifier: 0,
                    reserved: 0,
                    leds: 0,
                    keycodes: [KeyboardUsage::KeyboardPrintScreen as u8, 0, 0, 0, 0, 0],
                },
                KeyboardUsage::KeyboardSs,
            ),
            (
                &mut back_button,
                &back_input,
                KeyboardReport {
                    modifier: 0x04,
                    reserved: 0,
                    leds: 0,
                    keycodes: [KeyboardUsage::KeyboardLeftArrow as u8, 0, 0, 0, 0, 0],
                },
                KeyboardUsage::KeyboardSpacebar,
            ),
            (
                &mut forward_button,
                &forward_input,
                KeyboardReport {
                    modifier: 0x04,
                    reserved: 0,
                    leds: 0,
                    keycodes: [KeyboardUsage::KeyboardRightArrow as u8, 0, 0, 0, 0, 0],
                },
                KeyboardUsage::KeyboardEnter,
            ),
        ] {
            if let Some(pressed) = button_change(button, input.level(), now_ms) {
                if game_mode {
                    send_game_key(game_key, pressed).await;
                } else {
                    USB_KEYBOARD_REPORTS
                        .send(if pressed {
                            report
                        } else {
                            KeyboardReport::default()
                        })
                        .await;
                }
            }
        }

        Timer::after(Duration::from_millis(u64::from(USB_HID_POLL_MS))).await;
    }
}
