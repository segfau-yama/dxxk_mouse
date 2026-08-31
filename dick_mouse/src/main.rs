#![no_std]
#![no_main]

use embassy_executor::Spawner;
use esp_backtrace as _;
use esp_hal::{
    gpio::Pin,
    i2s::master::{Channels, Config as I2sConfig, DataFormat, I2s},
    interrupt::software::SoftwareInterruptControl,
    otg_fs::Usb,
    pcnt::Pcnt,
    time::Rate,
    timer::timg::TimerGroup,
};

esp_bootloader_esp_idf::esp_app_desc!();

mod tasks;

use tasks::audio::I2S_FRAME_BYTES;

#[esp_rtos::main]
async fn main(spawner: Spawner) {
    let peripherals = esp_hal::init(esp_hal::Config::default());

    let sw_int = SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0, sw_int.software_interrupt0);

    let pcnt = Pcnt::new(peripherals.PCNT);
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
    let i2s_rx = i2s
        .i2s_rx
        .with_bclk(peripherals.GPIO5)
        .with_ws(peripherals.GPIO6)
        .with_din(peripherals.GPIO7)
        .build(rx_descriptors);
    let i2s_tx = i2s
        .i2s_tx
        .with_bclk(peripherals.GPIO35)
        .with_ws(peripherals.GPIO36)
        .with_dout(peripherals.GPIO37)
        .build(tx_descriptors);
    let usb = Usb::new(peripherals.USB0, peripherals.GPIO20, peripherals.GPIO19);

    spawner.spawn(
        tasks::hid::hid_task(peripherals.GPIO21.degrade()).expect("failed to create HID task"),
    );
    spawner.spawn(
        tasks::mouse::mouse_task(
            pcnt.unit0,
            peripherals.GPIO9.degrade(),
            peripherals.GPIO10.degrade(),
            peripherals.ADC1,
            peripherals.GPIO1,
            peripherals.GPIO2,
            peripherals.GPIO47.degrade(),
            peripherals.GPIO48.degrade(),
        )
        .expect("failed to create mouse task"),
    );
    spawner.spawn(
        tasks::audio::microphone_task(
            i2s_rx,
            peripherals.GPIO18.degrade(),
            pcnt.unit1,
            peripherals.GPIO13.degrade(),
            peripherals.GPIO14.degrade(),
        )
        .expect("failed to create microphone task"),
    );
    spawner.spawn(tasks::usb::usb_task(usb).expect("failed to create usb task"));
    spawner.spawn(
        tasks::audio::speaker_task(
            i2s_tx,
            peripherals.GPIO17.degrade(),
            pcnt.unit2,
            peripherals.GPIO11.degrade(),
            peripherals.GPIO12.degrade(),
        )
        .expect("failed to create speaker task"),
    );
    spawner.spawn(
        tasks::keyboard::keyboard_task(
            peripherals.GPIO40.degrade(),
            peripherals.GPIO41.degrade(),
            peripherals.GPIO42.degrade(),
        )
        .expect("failed to create keyboard task"),
    );

    core::future::pending::<()>().await;
}
