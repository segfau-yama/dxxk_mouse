#![no_std]
#![no_main]
#![allow(non_snake_case)]
#![allow(unexpected_cfgs)]

esp_bootloader_esp_idf::esp_app_desc!();

#[cfg(test)]
#[embedded_test::tests]
mod tests {
    use esp_hal::{interrupt::software::SoftwareInterruptControl, timer::timg::TimerGroup};

    #[test]
    fn T01_mainで使用するRTOS用peripheralを初期化できる() {
        let peripherals = esp_hal::init(esp_hal::Config::default());

        let sw_int = SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
        let timg0 = TimerGroup::new(peripherals.TIMG0);

        let _software_interrupt0 = sw_int.software_interrupt0;
        let _timer0 = timg0.timer0;
    }
}
