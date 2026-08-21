#![no_std]
#![no_main]
#![allow(unexpected_cfgs)]

esp_bootloader_esp_idf::esp_app_desc!();

#[cfg(test)]
#[embedded_test::tests]
mod tests {
    use esp_hal::{
        i2s::master::{Channels, Config as I2sConfig, DataFormat, I2s},
        interrupt::software::SoftwareInterruptControl,
        otg_fs::Usb,
        pcnt::Pcnt,
        time::Rate,
        timer::timg::TimerGroup,
    };

    #[test]
    fn mainで使うperipheralを初期化できる() {
        let peripherals = esp_hal::init(esp_hal::Config::default());
        let sw_int = SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
        let timg0 = TimerGroup::new(peripherals.TIMG0);
        let _pcnt = Pcnt::new(peripherals.PCNT);
        let (rx_descriptors, tx_descriptors) = esp_hal::dma_descriptors!(96, 96);
        let i2s = I2s::new(
            peripherals.I2S0,
            peripherals.DMA_CH0,
            I2sConfig::new_tdm_philips()
                .with_sample_rate(Rate::from_hz(48_000))
                .with_data_format(DataFormat::Data16Channel16)
                .with_channels(Channels::MONO),
        )
        .expect("failed to create I2S")
        .into_async();
        let _i2s_rx = i2s
            .i2s_rx
            .with_bclk(peripherals.GPIO17)
            .with_ws(peripherals.GPIO18)
            .with_din(peripherals.GPIO8)
            .build(rx_descriptors);
        let _i2s_tx = i2s
            .i2s_tx
            .with_bclk(peripherals.GPIO21)
            .with_ws(peripherals.GPIO38)
            .with_dout(peripherals.GPIO9)
            .build(tx_descriptors);
        let _usb = Usb::new(peripherals.USB0, peripherals.GPIO20, peripherals.GPIO19);
        let _software_interrupt0 = sw_int.software_interrupt0;
        let _timer0 = timg0.timer0;
    }
}
