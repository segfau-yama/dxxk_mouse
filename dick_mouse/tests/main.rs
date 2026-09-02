#![no_std]
#![no_main]
#![allow(unexpected_cfgs)]

esp_bootloader_esp_idf::esp_app_desc!();

#[cfg(test)]
#[embedded_test::tests]
mod tests {
    use esp_hal::{
        i2s::master::{Channels, DataFormat, I2s, TdmConfig as I2sConfig},
        pcnt::Pcnt,
        time::Rate,
        timer::timg::TimerGroup,
        usb::otg::Usb,
    };

    const I2S_FRAME_BYTES: usize = 48 * core::mem::size_of::<i32>();

    #[test]
    fn mainで使うperipheralを初期化できる() {
        let peripherals = esp_hal::init(esp_hal::Config::default());
        let timg0 = TimerGroup::new(peripherals.TIMG0);
        let _pcnt = Pcnt::new(peripherals.PCNT);
        let (rx_descriptors, tx_descriptors) =
            esp_hal::dma_descriptors!(I2S_FRAME_BYTES, I2S_FRAME_BYTES);
        let i2s = I2s::new(
            peripherals.I2S0,
            peripherals.DMA_CH0,
            I2sConfig::new_tdm_philips()
                .with_sample_rate(Rate::from_hz(48_000))
                .with_data_format(DataFormat::Data32Channel32)
                .with_channels(Channels::MONO),
        )
        .expect("failed to create I2S")
        .into_async();
        let _i2s_rx = i2s
            .i2s_rx
            .with_bclk(peripherals.GPIO15)
            .with_ws(peripherals.GPIO16)
            .with_din(peripherals.GPIO17)
            .build(rx_descriptors);
        let _i2s_tx = i2s
            .i2s_tx
            .with_bclk(peripherals.GPIO8)
            .with_ws(peripherals.GPIO9)
            .with_dout(peripherals.GPIO10)
            .build(tx_descriptors);
        let _usb = Usb::new_fs(peripherals.USB_FS, peripherals.GPIO20, peripherals.GPIO19);
        let _software_interrupt0 = peripherals.FROM_CPU_INTR0;
        let _timer0 = timg0.timer0;
    }
}
