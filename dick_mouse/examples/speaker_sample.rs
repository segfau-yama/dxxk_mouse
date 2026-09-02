#![no_std]
#![no_main]

use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use embassy_usb::{
    Builder as UsbBuilder, Config as UsbConfig, UsbDevice,
    class::uac1::{
        self, FeedbackRefresh, SampleWidth,
        speaker::{self, Speaker, State},
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

const USB_MAX_PACKET_SIZE: usize = 192;
const USB_CONFIG_DESCRIPTOR_SIZE: usize = 512;
const USB_BOS_DESCRIPTOR_SIZE: usize = 128;
const USB_MSOS_DESCRIPTOR_SIZE: usize = 128;
const USB_CONTROL_BUFFER_SIZE: usize = 64;
const USB_EP_OUT_BUFFER_SIZE: usize = 256;
const USB_AUDIO_FEEDBACK_48K: [u8; 3] = [0x00, 0x00, 0x0c];

static SUPPORTED_SAMPLE_RATES: [u32; 1] = [48_000];
static AUDIO_CHANNELS: [uac1::Channel; 1] = [uac1::Channel::LeftFront];
static USB_EP_OUT_BUFFER: StaticCell<[u8; USB_EP_OUT_BUFFER_SIZE]> = StaticCell::new();
static USB_CONFIG_DESCRIPTOR: StaticCell<[u8; USB_CONFIG_DESCRIPTOR_SIZE]> = StaticCell::new();
static USB_BOS_DESCRIPTOR: StaticCell<[u8; USB_BOS_DESCRIPTOR_SIZE]> = StaticCell::new();
static USB_MSOS_DESCRIPTOR: StaticCell<[u8; USB_MSOS_DESCRIPTOR_SIZE]> = StaticCell::new();
static USB_CONTROL_BUFFER: StaticCell<[u8; USB_CONTROL_BUFFER_SIZE]> = StaticCell::new();
static USB_SPEAKER_STATE: StaticCell<State<'static>> = StaticCell::new();

#[embassy_executor::task]
async fn usb_task(mut device: UsbDevice<'static, UsbDriver<'static>>) {
    device.run().await;
}

#[embassy_executor::task]
async fn speaker_stream(mut stream: speaker::Stream<'static, UsbDriver<'static>>) {
    loop {
        stream.wait_connection().await;
        println!("speaker: OUT endpoint enabled");
        let mut packets = 0;

        loop {
            let mut packet = [0u8; USB_MAX_PACKET_SIZE];
            match stream.read_packet(&mut packet).await {
                Ok(size) => {
                    if packets == 0 {
                        println!("speaker: first packet = {} bytes", size);
                    }
                    packets += 1;
                }
                Err(error) => {
                    println!("speaker: OUT read error = {:?}", error);
                    break;
                }
            }
            // The sample is intentionally discarded: this binary checks UAC1
            // enumeration and OUT streaming without involving I2S hardware.
        }
    }
}

#[embassy_executor::task]
async fn speaker_feedback(mut feedback: speaker::Feedback<'static, UsbDriver<'static>>) {
    loop {
        feedback.wait_connection().await;
        println!("speaker: feedback endpoint enabled");

        loop {
            match feedback.write_packet(&USB_AUDIO_FEEDBACK_48K).await {
                Ok(()) => {
                    Timer::after(Duration::from_millis(
                        FeedbackRefresh::Period32Frames.frame_count() as u64,
                    ))
                    .await;
                }
                Err(error) => {
                    println!("speaker: feedback write error = {:?}", error);
                    break;
                }
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

    let mut config = UsbConfig::new(0xc0de, 0x0001);
    config.manufacturer = Some("dick mouse");
    config.product = Some("UAC1 speaker sample");
    config.serial_number = Some("speaker-0001");

    let mut builder = UsbBuilder::new(
        driver,
        config,
        USB_CONFIG_DESCRIPTOR.init([0; USB_CONFIG_DESCRIPTOR_SIZE]),
        USB_BOS_DESCRIPTOR.init([0; USB_BOS_DESCRIPTOR_SIZE]),
        USB_MSOS_DESCRIPTOR.init([0; USB_MSOS_DESCRIPTOR_SIZE]),
        USB_CONTROL_BUFFER.init([0; USB_CONTROL_BUFFER_SIZE]),
    );

    let speaker = Speaker::new(
        &mut builder,
        USB_SPEAKER_STATE.init(State::new()),
        USB_MAX_PACKET_SIZE as u16,
        SampleWidth::Width2Byte,
        &SUPPORTED_SAMPLE_RATES,
        &AUDIO_CHANNELS,
        FeedbackRefresh::Period32Frames,
    );
    let device = builder.build();

    spawner.spawn(usb_task(device).expect("failed to spawn USB task"));
    spawner.spawn(speaker_stream(speaker.stream).expect("failed to spawn speaker task"));
    spawner.spawn(speaker_feedback(speaker.feedback).expect("failed to spawn feedback task"));
    println!("speaker: UAC1 ready (UART0 115200 8N1)");

    core::future::pending::<()>().await;
}
