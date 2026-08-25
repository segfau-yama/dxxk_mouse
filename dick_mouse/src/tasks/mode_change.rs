use core::sync::atomic::Ordering;

use dick_mouse::device::{Button, button::button_change};
use embassy_time::{Duration, Timer};
use esp_hal::{
    gpio::{AnyPin, Input, InputConfig, Level, Pull},
    time::Instant,
};
use usbd_hid::descriptor::MouseReport;

use super::{
    game_hid::{GAME_MODE, clear_game_keys},
    usb::{USB_HID_POLL_MS, USB_MOUSE_REPORTS},
};

#[embassy_executor::task]
pub(crate) async fn mode_change_task(mode_gpio: AnyPin<'static>) {
    let mode_input = Input::new(mode_gpio, InputConfig::default().with_pull(Pull::Up));
    let mut mode_button = Button::new(mode_input.level(), Level::Low, 5);
    let enabled = mode_button.is_pressed();
    if enabled {
        clear_game_keys().await;
    }
    GAME_MODE.store(enabled, Ordering::Relaxed);
    if !enabled {
        clear_game_keys().await;
    }

    loop {
        let now_ms = Instant::now().duration_since_epoch().as_millis();

        if let Some(pressed) = button_change(&mut mode_button, mode_input.level(), now_ms) {
            if pressed {
                clear_game_keys().await;
            }
            GAME_MODE.store(pressed, Ordering::Relaxed);
            if !pressed {
                clear_game_keys().await;
            }
            USB_MOUSE_REPORTS
                .send(MouseReport {
                    buttons: 0,
                    x: 0,
                    y: 0,
                    wheel: 0,
                    pan: 0,
                })
                .await;
        }

        Timer::after(Duration::from_millis(u64::from(USB_HID_POLL_MS))).await;
    }
}
