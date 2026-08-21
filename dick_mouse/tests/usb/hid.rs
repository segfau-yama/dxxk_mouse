#![no_std]
#![no_main]
#![allow(unexpected_cfgs)]

esp_bootloader_esp_idf::esp_app_desc!();

#[cfg(test)]
#[embedded_test::tests]
mod tests {
    use core::assert_eq;

    use dick_mouse::usb::hid::{KeyboardReport, MouseReport, Shortcut, shortcut_report};
    use usbd_hid::descriptor::{KeyboardUsage, SerializedDescriptor};

    #[test]
    fn shortcut_reportはキー入力を作る() {
        let copy = shortcut_report(Shortcut::Copy);
        let paste = shortcut_report(Shortcut::Paste);
        let back = shortcut_report(Shortcut::Back);
        let forward = shortcut_report(Shortcut::Forward);

        assert_eq!(copy.modifier, 0x01);
        assert_eq!(copy.keycodes[0], KeyboardUsage::KeyboardCc as u8);
        assert_eq!(paste.keycodes[0], KeyboardUsage::KeyboardVv as u8);
        assert_eq!(back.modifier, 0x04);
        assert_eq!(back.keycodes[0], KeyboardUsage::KeyboardLeftArrow as u8);
        assert_eq!(forward.keycodes[0], KeyboardUsage::KeyboardRightArrow as u8);
    }

    #[test]
    fn hid_reportはdescriptorを持つ() {
        assert!(!KeyboardReport::desc().is_empty());
        assert!(!MouseReport::desc().is_empty());
    }
}
