use core::sync::atomic::AtomicBool;

use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, mutex::Mutex};
use usbd_hid::descriptor::{KeyboardReport, KeyboardUsage};

use super::usb::USB_KEYBOARD_REPORTS;

pub(crate) const GAME_JOYSTICK_THRESHOLD: i16 = 512;
const GAME_KEYS: [KeyboardUsage; 9] = [
    KeyboardUsage::KeyboardUpArrow,
    KeyboardUsage::KeyboardDownArrow,
    KeyboardUsage::KeyboardLeftArrow,
    KeyboardUsage::KeyboardRightArrow,
    KeyboardUsage::KeyboardSs,
    KeyboardUsage::KeyboardAa,
    KeyboardUsage::KeyboardDd,
    KeyboardUsage::KeyboardSpacebar,
    KeyboardUsage::KeyboardEnter,
];

pub(crate) static GAME_MODE: AtomicBool = AtomicBool::new(false);
static GAME_BUTTON_BITS: Mutex<CriticalSectionRawMutex, usize> = Mutex::new(0);

pub(crate) async fn send_game_key(key: KeyboardUsage, pressed: bool) {
    let Some(key_index) = GAME_KEYS.iter().position(|game_key| *game_key == key) else {
        return;
    };

    let mut pressed_buttons = GAME_BUTTON_BITS.lock().await;
    let mask = 1usize << key_index;
    if ((*pressed_buttons & mask) != 0) == pressed {
        return;
    }

    if pressed {
        *pressed_buttons |= mask;
    } else {
        *pressed_buttons &= !mask;
    }

    let mut report = KeyboardReport::default();
    let mut keycode_index = 0;

    for (index, key) in GAME_KEYS.iter().copied().enumerate() {
        if *pressed_buttons & (1usize << index) != 0 && keycode_index < report.keycodes.len() {
            report.keycodes[keycode_index] = key as u8;
            keycode_index += 1;
        }
    }

    USB_KEYBOARD_REPORTS.send(report).await;
}

pub(crate) async fn clear_game_keys() {
    {
        let mut pressed_buttons = GAME_BUTTON_BITS.lock().await;
        *pressed_buttons = 0;
    }
    USB_KEYBOARD_REPORTS.send(KeyboardReport::default()).await;
}
