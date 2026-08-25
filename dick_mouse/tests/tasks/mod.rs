#![no_std]
#![no_main]
#![allow(unexpected_cfgs)]

esp_bootloader_esp_idf::esp_app_desc!();

#[path = "../../src/tasks/mod.rs"]
mod tasks;

#[cfg(test)]
#[embedded_test::tests]
mod tests {
    use core::assert_eq;

    use super::tasks;

    #[test]
    fn audio_moduleはframe変換とtask入口を持つ() {
        let frame = tasks::audio::bytes_to_audio_frame(&[0x34, 0x12, 0xfe, 0xff]);

        assert_eq!(tasks::audio::AUDIO_FRAME_BYTES, 96);
        assert_eq!(frame[0], 0x1234);
        assert_eq!(frame[1], -2);
        assert_eq!(frame[2], 0);
        let _ = tasks::audio::microphone_task;
        let _ = tasks::audio::speaker_task;
    }

    #[test]
    fn game_hid_moduleはgame入力の入口を持つ() {
        assert_eq!(tasks::game_hid::GAME_JOYSTICK_THRESHOLD, 512);
        let _ = tasks::game_hid::send_game_key;
        let _ = tasks::game_hid::clear_game_keys;
    }

    #[test]
    fn keyboard_moduleはtask入口を持つ() {
        let _ = tasks::keyboard::keyboard_task;
    }

    #[test]
    fn mode_change_moduleはtask入口を持つ() {
        let _ = tasks::mode_change::mode_change_task;
    }

    #[test]
    fn mouse_moduleはtask入口を持つ() {
        let _ = tasks::mouse::mouse_task;
    }

    #[test]
    fn usb_moduleはpoll間隔とtask入口を持つ() {
        assert_eq!(tasks::usb::USB_HID_POLL_MS, 10);
        let _ = tasks::usb::usb_task;
    }
}
