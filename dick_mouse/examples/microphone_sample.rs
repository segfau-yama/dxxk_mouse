#![no_std]
#![no_main]

use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use embassy_usb::{
    Builder as UsbBuilder, Config as UsbConfig, UsbDevice,
    class::uac1::{
        FeedbackRefresh, SampleWidth,
        source::{AudioSource, AudioSourceControlHandler, AudioSourceEpIn},
    },
};
use esp_backtrace as _;
use esp_hal::{
    interrupt::software::SoftwareInterruptControl,
    otg_fs::{
        Usb,
        asynch::{Config as UsbDriverConfig, Driver as UsbDriver},
    },
    timer::timg::TimerGroup,
};
use esp_println::println;
use static_cell::StaticCell;

esp_bootloader_esp_idf::esp_app_desc!();

const USB_AUDIO_FRAME_BYTES: usize = 48 * 2 * 2;
const USB_CONFIG_DESCRIPTOR_SIZE: usize = 512;
const USB_BOS_DESCRIPTOR_SIZE: usize = 128;
const USB_MSOS_DESCRIPTOR_SIZE: usize = 128;
const USB_CONTROL_BUFFER_SIZE: usize = 64;
const USB_EP_OUT_BUFFER_SIZE: usize = 256;
const USB_AUDIO_FEEDBACK_48K: [u8; 3] = [0x00, 0x00, 0x0c];

static SUPPORTED_SAMPLE_RATES: [u32; 1] = [48_000];
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
    let mut phase = 0usize;

    loop {
        audio.wait_enabled().await;
        println!("microphone: IN endpoint enabled");
        let mut frames = 0;

        loop {
            let mut frame = [0u8; USB_AUDIO_FRAME_BYTES];
            for (sample_index, chunk) in frame.chunks_exact_mut(4).enumerate() {
                let sample = if (sample_index + phase) % 32 < 16 {
                    0x2000i16
                } else {
                    -0x2000i16
                };
                let sample = sample.to_le_bytes();
                chunk[..2].copy_from_slice(&sample);
                chunk[2..].copy_from_slice(&sample);
            }
            phase = phase.wrapping_add(48);

            match audio.write(&frame).await {
                Ok(()) => {
                    if frames == 0 {
                        println!("microphone: first frame = {} bytes", frame.len());
                    }
                    frames += 1;
                }
                Err(error) => {
                    println!("microphone: IN write error = {:?}", error);
                    break;
                }
            }
        }
    }
}

#[embassy_executor::task]
async fn microphone_feedback(mut feedback: AudioSourceEpIn<'static, UsbDriver<'static>>) {
    loop {
        feedback.wait_enabled().await;
        println!("microphone: feedback endpoint enabled");

        loop {
            match feedback.write(&USB_AUDIO_FEEDBACK_48K).await {
                Ok(()) => {
                    Timer::after(Duration::from_millis(
                        FeedbackRefresh::Period32Frames.frame_count() as u64,
                    ))
                    .await;
                }
                Err(error) => {
                    println!("microphone: feedback write error = {:?}", error);
                    break;
                }
            }
        }
    }
}

#[esp_rtos::main]
async fn main(spawner: Spawner) {
    let peripherals = esp_hal::init(esp_hal::Config::default());

    let sw_int = SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0, sw_int.software_interrupt0);

    let usb = Usb::new(peripherals.USB0, peripherals.GPIO20, peripherals.GPIO19);
    let driver = UsbDriver::new(
        usb,
        USB_EP_OUT_BUFFER.init([0; USB_EP_OUT_BUFFER_SIZE]),
        UsbDriverConfig::default(),
    );

    let mut config = UsbConfig::new(0xc0de, 0x0002);
    config.manufacturer = Some("dick mouse");
    config.product = Some("UAC1 microphone sample");
    config.serial_number = Some("microphone-0001");

    let mut builder = UsbBuilder::new(
        driver,
        config,
        USB_CONFIG_DESCRIPTOR.init([0; USB_CONFIG_DESCRIPTOR_SIZE]),
        USB_BOS_DESCRIPTOR.init([0; USB_BOS_DESCRIPTOR_SIZE]),
        USB_MSOS_DESCRIPTOR.init([0; USB_MSOS_DESCRIPTOR_SIZE]),
        USB_CONTROL_BUFFER.init([0; USB_CONTROL_BUFFER_SIZE]),
    );

    let microphone = AudioSource::new(
        &mut builder,
        &SUPPORTED_SAMPLE_RATES,
        SampleWidth::Width2Byte,
        FeedbackRefresh::Period32Frames as u8,
        None,
    );
    builder.handler(USB_MICROPHONE_HANDLER.init(microphone.handler));
    let device = builder.build();

    spawner.spawn(usb_task(device).expect("failed to spawn USB task"));
    spawner
        .spawn(microphone_stream(microphone.audio_ep_in).expect("failed to spawn microphone task"));
    spawner.spawn(
        microphone_feedback(microphone.feedback_ep_in).expect("failed to spawn feedback task"),
    );
    println!("microphone: UAC1 ready (UART0 115200 8N1)");

    core::future::pending::<()>().await;
}
