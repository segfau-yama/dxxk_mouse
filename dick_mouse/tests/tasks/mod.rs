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
    use usbd_hid::descriptor::KeyboardUsage;

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
    fn hid_moduleは通常入力とgame入力をreportへ変換する() {
        let keyboard = tasks::hid::KeyboardInput {
            joystick_pressed: true,
            back_pressed: true,
            forward_pressed: true,
        };
        let normal = tasks::hid::keyboard_report(false, keyboard, Default::default());
        assert_eq!(normal.modifier, 0x04);
        assert_eq!(
            normal.keycodes,
            [
                KeyboardUsage::KeyboardPrintScreen as u8,
                KeyboardUsage::KeyboardLeftArrow as u8,
                KeyboardUsage::KeyboardRightArrow as u8,
                0,
                0,
                0,
            ]
        );
        let remaining = tasks::hid::keyboard_report(
            false,
            tasks::hid::KeyboardInput {
                back_pressed: true,
                ..Default::default()
            },
            Default::default(),
        );
        assert_eq!(remaining.modifier, 0x04);
        assert_eq!(
            remaining.keycodes,
            [KeyboardUsage::KeyboardLeftArrow as u8, 0, 0, 0, 0, 0]
        );

        let game_mouse = tasks::hid::MouseInput {
            left_pressed: true,
            right_pressed: true,
            joystick_x: -513,
            joystick_y: -513,
            wheel: 1,
        };
        let game = tasks::hid::keyboard_report(true, keyboard, game_mouse);
        assert_eq!(
            game.keycodes,
            [
                KeyboardUsage::KeyboardUpArrow as u8,
                KeyboardUsage::KeyboardLeftArrow as u8,
                KeyboardUsage::KeyboardSs as u8,
                KeyboardUsage::KeyboardAa as u8,
                KeyboardUsage::KeyboardDd as u8,
                KeyboardUsage::KeyboardSpacebar as u8,
            ]
        );
        let game_remaining = tasks::hid::keyboard_report(
            true,
            tasks::hid::KeyboardInput {
                forward_pressed: true,
                ..Default::default()
            },
            tasks::hid::MouseInput {
                joystick_x: 513,
                joystick_y: 513,
                ..Default::default()
            },
        );
        assert_eq!(
            game_remaining.keycodes,
            [
                KeyboardUsage::KeyboardDownArrow as u8,
                KeyboardUsage::KeyboardRightArrow as u8,
                KeyboardUsage::KeyboardEnter as u8,
                0,
                0,
                0,
            ]
        );

        let mouse = tasks::hid::mouse_report(tasks::hid::MouseInput {
            left_pressed: true,
            right_pressed: true,
            joystick_x: 512,
            joystick_y: -512,
            wheel: -2,
        });
        assert_eq!(mouse.buttons, 3);
        assert_eq!(mouse.x, 2);
        assert_eq!(mouse.y, -2);
        assert_eq!(mouse.wheel, -2);
        assert_eq!(tasks::hid::GAME_JOYSTICK_THRESHOLD, 512);
        let _ = tasks::hid::hid_task;
    }

    #[test]
    fn keyboard_moduleはtask入口を持つ() {
        let _ = tasks::keyboard::keyboard_task;
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
