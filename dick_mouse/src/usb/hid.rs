pub use keyboard::{Report as KeyboardReport, Shortcut, shortcut_report};
pub use mouse::Report as MouseReport;

pub mod keyboard {
    pub use usbd_hid::descriptor::KeyboardReport as Report;

    use usbd_hid::descriptor::KeyboardUsage;

    #[derive(Debug, Clone, Copy, Eq, PartialEq)]
    pub enum Shortcut {
        Back,
        Forward,
    }

    pub fn shortcut_report(shortcut: Shortcut) -> Report {
        let (modifier, keycode) = match shortcut {
            Shortcut::Back => (0x04, KeyboardUsage::KeyboardLeftArrow as u8),
            Shortcut::Forward => (0x04, KeyboardUsage::KeyboardRightArrow as u8),
        };

        Report {
            modifier,
            reserved: 0,
            leds: 0,
            keycodes: [keycode, 0, 0, 0, 0, 0],
        }
    }
}

pub mod mouse {
    pub use usbd_hid::descriptor::MouseReport as Report;
}
