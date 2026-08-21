pub use keyboard::{Report as KeyboardReport, Shortcut, shortcut_report};
pub use mouse::Report as MouseReport;

pub mod keyboard {
    pub use usbd_hid::descriptor::KeyboardReport as Report;

    use usbd_hid::descriptor::KeyboardUsage;

    #[derive(Debug, Clone, Copy, Eq, PartialEq)]
    pub enum Shortcut {
        Copy,
        Paste,
        Back,
        Forward,
    }

    pub fn shortcut_report(shortcut: Shortcut) -> Report {
        let (modifier, keycode) = match shortcut {
            Shortcut::Copy => (0x01, KeyboardUsage::KeyboardCc as u8),
            Shortcut::Paste => (0x01, KeyboardUsage::KeyboardVv as u8),
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

#[cfg(test)]
mod tests {
    use super::{Shortcut, shortcut_report};

    #[test]
    fn copyはctrl_cになる() {
        let report = shortcut_report(Shortcut::Copy);

        assert_eq!(report.modifier, 0x01);
        assert_eq!(report.keycodes[0], 0x06);
    }

    #[test]
    fn backはalt_leftになる() {
        let report = shortcut_report(Shortcut::Back);

        assert_eq!(report.modifier, 0x04);
        assert_eq!(report.keycodes[0], 0x50);
    }
}
