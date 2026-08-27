use ch32_hal::{
    Peri,
    gpio::{AnyPin, Input, Level, Pull},
};
use dick_mouse::device::Button;
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, channel::Channel};
use embassy_time::{Duration, Instant, Timer};
use usbd_hid::descriptor::{KeyboardReport, KeyboardUsage, MouseReport};

use super::usb::{USB_HID_POLL_MS, USB_HID_REPORTS, UsbHidReport};

pub(crate) const GAME_JOYSTICK_THRESHOLD: i16 = 512;

#[derive(Debug, Clone, Copy, Default, Eq, PartialEq)]
pub(crate) struct KeyboardInput {
    pub(crate) joystick_pressed: bool,
    pub(crate) back_pressed: bool,
    pub(crate) forward_pressed: bool,
}

#[derive(Debug, Clone, Copy, Default, Eq, PartialEq)]
pub(crate) struct MouseInput {
    pub(crate) left_pressed: bool,
    pub(crate) right_pressed: bool,
    pub(crate) joystick_x: i16,
    pub(crate) joystick_y: i16,
    pub(crate) wheel: i8,
}

pub(crate) static KEYBOARD_REPORTS: Channel<CriticalSectionRawMutex, KeyboardInput, 4> =
    Channel::new();
pub(crate) static MOUSE_REPORTS: Channel<CriticalSectionRawMutex, MouseInput, 4> = Channel::new();

pub(crate) fn keyboard_report(
    game_mode: bool,
    keyboard: KeyboardInput,
    mouse: MouseInput,
) -> KeyboardReport {
    let mut report = KeyboardReport::default();
    if !game_mode && (keyboard.back_pressed || keyboard.forward_pressed) {
        report.modifier = 0x04;
    }
    let mut keycode_index = 0;
    let mut push_key = |key: KeyboardUsage, pressed: bool| {
        if pressed && keycode_index < report.keycodes.len() {
            report.keycodes[keycode_index] = key as u8;
            keycode_index += 1;
        }
    };

    if game_mode {
        for (key, pressed) in [
            (
                KeyboardUsage::KeyboardUpArrow,
                mouse.joystick_y < -GAME_JOYSTICK_THRESHOLD,
            ),
            (
                KeyboardUsage::KeyboardDownArrow,
                mouse.joystick_y > GAME_JOYSTICK_THRESHOLD,
            ),
            (
                KeyboardUsage::KeyboardLeftArrow,
                mouse.joystick_x < -GAME_JOYSTICK_THRESHOLD,
            ),
            (
                KeyboardUsage::KeyboardRightArrow,
                mouse.joystick_x > GAME_JOYSTICK_THRESHOLD,
            ),
            (KeyboardUsage::KeyboardSs, keyboard.joystick_pressed),
            (KeyboardUsage::KeyboardAa, mouse.left_pressed),
            (KeyboardUsage::KeyboardDd, mouse.right_pressed),
            (KeyboardUsage::KeyboardSpacebar, keyboard.back_pressed),
            (KeyboardUsage::KeyboardEnter, keyboard.forward_pressed),
        ] {
            push_key(key, pressed);
        }
    } else {
        for (key, pressed) in [
            (
                KeyboardUsage::KeyboardPrintScreen,
                keyboard.joystick_pressed,
            ),
            (KeyboardUsage::KeyboardLeftArrow, keyboard.back_pressed),
            (KeyboardUsage::KeyboardRightArrow, keyboard.forward_pressed),
        ] {
            push_key(key, pressed);
        }
    }

    report
}

pub(crate) fn mouse_report(input: MouseInput) -> MouseReport {
    MouseReport {
        buttons: u8::from(input.left_pressed) | (u8::from(input.right_pressed) << 1),
        x: (input.joystick_x / 256) as i8,
        y: (input.joystick_y / 256) as i8,
        wheel: input.wheel,
        pan: 0,
    }
}

#[embassy_executor::task]
pub(crate) async fn hid_task(mode_gpio: Peri<'static, AnyPin>) {
    let mode_input = Input::new(mode_gpio, Pull::Up);
    let mut mode_button = Button::new(mode_input.get_level(), Level::Low, 5);
    let mut game_mode = mode_button.is_pressed();
    let mut keyboard = KeyboardInput::default();
    let mut mouse = MouseInput::default();
    let mut reported_game_keycodes = [0; 6];
    let mut reported_mouse_buttons = 0;

    loop {
        let now_ms = Instant::now().as_millis();

        mode_button = mode_button.update(mode_input.get_level(), now_ms);
        if mode_button.changed() {
            USB_HID_REPORTS
                .send(UsbHidReport::Keyboard(KeyboardReport::default()))
                .await;
            USB_HID_REPORTS
                .send(UsbHidReport::Mouse(MouseReport {
                    buttons: 0,
                    x: 0,
                    y: 0,
                    wheel: 0,
                    pan: 0,
                }))
                .await;

            game_mode = mode_button.is_pressed();
            reported_game_keycodes = [0; 6];
            reported_mouse_buttons = 0;

            let report = keyboard_report(game_mode, keyboard, mouse);
            if report.modifier != 0 || report.keycodes != [0; 6] {
                if game_mode {
                    reported_game_keycodes = report.keycodes;
                }
                USB_HID_REPORTS.send(UsbHidReport::Keyboard(report)).await;
            }

            if !game_mode {
                let report = mouse_report(MouseInput { wheel: 0, ..mouse });
                reported_mouse_buttons = report.buttons;
                if report.buttons != 0 || report.x != 0 || report.y != 0 {
                    USB_HID_REPORTS.send(UsbHidReport::Mouse(report)).await;
                }
            }
        }

        while let Ok(input) = KEYBOARD_REPORTS.try_receive() {
            keyboard = input;
            let report = keyboard_report(game_mode, keyboard, mouse);

            if game_mode {
                if report.keycodes != reported_game_keycodes {
                    reported_game_keycodes = report.keycodes;
                    USB_HID_REPORTS.send(UsbHidReport::Keyboard(report)).await;
                }
            } else {
                USB_HID_REPORTS.send(UsbHidReport::Keyboard(report)).await;
            }
        }

        while let Ok(input) = MOUSE_REPORTS.try_receive() {
            mouse = input;

            if game_mode {
                let report = keyboard_report(true, keyboard, mouse);
                if report.keycodes != reported_game_keycodes {
                    reported_game_keycodes = report.keycodes;
                    USB_HID_REPORTS.send(UsbHidReport::Keyboard(report)).await;
                }
            } else {
                let report = mouse_report(mouse);
                if report.buttons != reported_mouse_buttons
                    || report.x != 0
                    || report.y != 0
                    || report.wheel != 0
                {
                    reported_mouse_buttons = report.buttons;
                    USB_HID_REPORTS.send(UsbHidReport::Mouse(report)).await;
                }
            }
        }

        Timer::after(Duration::from_millis(u64::from(USB_HID_POLL_MS))).await;
    }
}
