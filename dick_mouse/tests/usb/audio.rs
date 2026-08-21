#![no_std]
#![no_main]
#![allow(unexpected_cfgs)]

esp_bootloader_esp_idf::esp_app_desc!();

#[cfg(test)]
#[embedded_test::tests]
mod tests {
    use core::assert_eq;

    use dick_mouse::usb::audio::{Microphone, Speaker};

    #[test]
    fn microphoneはbufferを更新する() {
        let microphone = Microphone::new([0, 1]).update([2, 3]);

        assert_eq!(microphone.buffer(), &[2, 3]);
    }

    #[test]
    fn speakerはbufferを更新する() {
        let speaker = Speaker::new([0, -1]).update([-2, -3]);

        assert_eq!(speaker.buffer(), &[-2, -3]);
    }
}
