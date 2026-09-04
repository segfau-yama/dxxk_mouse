#![no_std]
#![no_main]

use embassy_executor::Spawner;
use embassy_usb::{
    Builder as UsbBuilder, Config as UsbConfig, UsbDevice,
    class::uac1::{
        SampleWidth,
        source::{AudioSource, AudioSourceControlHandler, AudioSourceEpIn},
    },
};
use esp_backtrace as _;
use esp_hal::{
    timer::timg::TimerGroup,
    usb::otg::{
        Usb,
        embassy_usb_device::{Config as UsbDriverConfig, Driver as UsbDriver},
    },
};
use esp_println::println;
use static_cell::StaticCell;

esp_bootloader_esp_idf::esp_app_desc!();

const AUDIO_FRAME_SAMPLES: usize = 48;
const USB_AUDIO_CHANNELS: usize = 2;
const USB_AUDIO_FRAME_BYTES: usize =
    AUDIO_FRAME_SAMPLES * USB_AUDIO_CHANNELS * core::mem::size_of::<i16>();
const USB_CONFIG_DESCRIPTOR_SIZE: usize = 512;
const USB_BOS_DESCRIPTOR_SIZE: usize = 128;
const USB_MSOS_DESCRIPTOR_SIZE: usize = 128;
const USB_CONTROL_BUFFER_SIZE: usize = 64;
const USB_EP_OUT_BUFFER_SIZE: usize = 256;

static USB_MICROPHONE_SAMPLE_RATES: [u32; 1] = [48_000];
static USB_EP_OUT_BUFFER: StaticCell<[u8; USB_EP_OUT_BUFFER_SIZE]> = StaticCell::new();
static USB_CONFIG_DESCRIPTOR: StaticCell<[u8; USB_CONFIG_DESCRIPTOR_SIZE]> = StaticCell::new();
static USB_BOS_DESCRIPTOR: StaticCell<[u8; USB_BOS_DESCRIPTOR_SIZE]> = StaticCell::new();
static USB_MSOS_DESCRIPTOR: StaticCell<[u8; USB_MSOS_DESCRIPTOR_SIZE]> = StaticCell::new();
static USB_CONTROL_BUFFER: StaticCell<[u8; USB_CONTROL_BUFFER_SIZE]> = StaticCell::new();
static USB_MICROPHONE_HANDLER: StaticCell<AudioSourceControlHandler> = StaticCell::new();

#[embassy_executor::task]
async fn usb_task(mut device: UsbDevice<'static, UsbDriver<'static>>) {
    device.run().await;
}

#[embassy_executor::task]
async fn microphone_stream(mut audio: AudioSourceEpIn<'static, UsbDriver<'static>>) {
    // The packet is generated locally, so this test has no I2S clock or DMA activity.
    let packet = [0u8; USB_AUDIO_FRAME_BYTES];

    loop {
        audio.wait_enabled().await;
        println!("microphone: USB IN endpoint enabled");

        loop {
            if let Err(error) = audio.write(&packet).await {
                println!("microphone: IN write error = {:?}", error);
                break;
            }
        }
    }
}

#[esp_rtos::main]
async fn main(spawner: Spawner) {
    let peripherals = esp_hal::init(esp_hal::Config::default());

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0, peripherals.FROM_CPU_INTR0);

    let usb = Usb::new_fs(peripherals.USB_FS, peripherals.GPIO20, peripherals.GPIO19);
    let driver = UsbDriver::new(
        usb,
        USB_EP_OUT_BUFFER.init([0; USB_EP_OUT_BUFFER_SIZE]),
        UsbDriverConfig::default(),
    );

    let mut config = UsbConfig::new(0xc0de, 0x0003);
    config.manufacturer = Some("dick mouse");
    config.product = Some("UAC1 USB-only microphone sample");
    config.serial_number = Some("microphone-usb-only-0001");

    let mut builder = UsbBuilder::new(
        driver,
        config,
        USB_CONFIG_DESCRIPTOR.init([0; USB_CONFIG_DESCRIPTOR_SIZE]),
        USB_BOS_DESCRIPTOR.init([0; USB_BOS_DESCRIPTOR_SIZE]),
        USB_MSOS_DESCRIPTOR.init([0; USB_MSOS_DESCRIPTOR_SIZE]),
        USB_CONTROL_BUFFER.init([0; USB_CONTROL_BUFFER_SIZE]),
    );

    let AudioSource {
        audio_ep_in,
        handler,
    } = AudioSource::new(
        &mut builder,
        &USB_MICROPHONE_SAMPLE_RATES,
        SampleWidth::Width2Byte,
        None,
    );
    builder.handler(USB_MICROPHONE_HANDLER.init(handler));
    let device = builder.build();

    spawner.spawn(usb_task(device).expect("failed to spawn USB task"));
    spawner.spawn(microphone_stream(audio_ep_in).expect("failed to spawn microphone task"));
    println!("microphone: USB-only UAC1 48k S16_LE ready (UART0 115200 8N1)");

    core::future::pending::<()>().await;
}
