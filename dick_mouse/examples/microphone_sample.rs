#![no_std]
#![no_main]

use embassy_executor::Spawner;
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, channel::Channel};
use embassy_time::{Duration, Timer};
use embassy_usb::{Builder as UsbBuilder, Config as UsbConfig, UsbDevice};
use esp_backtrace as _;
use esp_hal::{
    Async,
    i2s::master::{Channels, Config as I2sConfig, DataFormat, I2s, I2sRx},
    interrupt::software::SoftwareInterruptControl,
    otg_fs::{
        Usb,
        asynch::{Config as UsbDriverConfig, Driver as UsbDriver},
    },
    time::Rate,
    timer::timg::TimerGroup,
};
use esp_println::println;
use static_cell::StaticCell;

use dick_mouse::tasks::usb_microphone::{
    ControlHandler as UsbMicrophoneControlHandler, Microphone as UsbMicrophoneClass,
    Stream as UsbMicrophoneStream,
};

esp_bootloader_esp_idf::esp_app_desc!();

const AUDIO_FRAME_SAMPLES: usize = 48;
const USB_AUDIO_CHANNELS: usize = 2;
const I2S_FRAME_BYTES: usize = AUDIO_FRAME_SAMPLES * core::mem::size_of::<i32>();
const USB_AUDIO_FRAME_BYTES: usize =
    AUDIO_FRAME_SAMPLES * USB_AUDIO_CHANNELS * core::mem::size_of::<i16>();
// UAC1 Type-I packets may differ by one sample around the nominal samples/frame.
const USB_AUDIO_MAX_PACKET_BYTES: usize =
    (AUDIO_FRAME_SAMPLES + 1) * USB_AUDIO_CHANNELS * core::mem::size_of::<i16>();
const USB_CONFIG_DESCRIPTOR_SIZE: usize = 512;
const USB_BOS_DESCRIPTOR_SIZE: usize = 128;
const USB_MSOS_DESCRIPTOR_SIZE: usize = 128;
const USB_CONTROL_BUFFER_SIZE: usize = 64;
const USB_EP_OUT_BUFFER_SIZE: usize = 256;
const MICROPHONE_QUEUE_DEPTH: usize = 8;

type AudioFrame = [i16; AUDIO_FRAME_SAMPLES];

static MICROPHONE_FRAMES: Channel<CriticalSectionRawMutex, AudioFrame, MICROPHONE_QUEUE_DEPTH> =
    Channel::new();
static USB_EP_OUT_BUFFER: StaticCell<[u8; USB_EP_OUT_BUFFER_SIZE]> = StaticCell::new();
static USB_CONFIG_DESCRIPTOR: StaticCell<[u8; USB_CONFIG_DESCRIPTOR_SIZE]> = StaticCell::new();
static USB_BOS_DESCRIPTOR: StaticCell<[u8; USB_BOS_DESCRIPTOR_SIZE]> = StaticCell::new();
static USB_MSOS_DESCRIPTOR: StaticCell<[u8; USB_MSOS_DESCRIPTOR_SIZE]> = StaticCell::new();
static USB_CONTROL_BUFFER: StaticCell<[u8; USB_CONTROL_BUFFER_SIZE]> = StaticCell::new();
static USB_MICROPHONE_HANDLER: StaticCell<UsbMicrophoneControlHandler> = StaticCell::new();

#[embassy_executor::task]
async fn usb_task(mut device: UsbDevice<'static, UsbDriver<'static>>) {
    device.run().await;
}

#[embassy_executor::task]
async fn microphone_capture(mut i2s_rx: I2sRx<'static, Async>) {
    let mut frames = 0u32;
    let mut peak = 0i32;

    loop {
        let mut input = [0u8; I2S_FRAME_BYTES];

        if let Err(error) = i2s_rx.read_dma_async(&mut input).await {
            println!("microphone: i2s read error = {:?}", error);
            Timer::after(Duration::from_millis(1)).await;
            continue;
        }

        let mut frame = [0i16; AUDIO_FRAME_SAMPLES];
        let mut frame_peak = 0i32;

        for (sample, raw) in frame.iter_mut().zip(input.chunks_exact(4)) {
            let raw = i32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]);
            // INMP441 is 24-bit left-aligned in a 32-bit I2S slot.
            // Keep the upper 16 bits without extra gain while debugging clipping/alignment.
            *sample = (raw >> 16) as i16;
            frame_peak = frame_peak.max(i32::from(*sample).abs());
        }

        peak = peak.max(frame_peak);
        MICROPHONE_FRAMES.send(frame).await;
        frames += 1;

        if frames == 100 {
            println!("microphone: i2s frames=100, peak={}", peak);
            frames = 0;
            peak = 0;
        }
    }
}

#[embassy_executor::task]
async fn microphone_stream(mut audio: UsbMicrophoneStream<'static, UsbDriver<'static>>) {
    loop {
        audio.wait_enabled().await;
        println!("microphone: IN endpoint enabled");

        let mut frames = 0u32;
        let mut peak = 0i32;

        loop {
            let samples = MICROPHONE_FRAMES.receive().await;
            let mut packet = [0u8; USB_AUDIO_FRAME_BYTES];
            let mut frame_peak = 0i32;

            for (sample, chunk) in samples.iter().zip(packet.chunks_exact_mut(4)) {
                frame_peak = frame_peak.max(i32::from(*sample).abs());
                let sample = sample.to_le_bytes();
                chunk[..2].copy_from_slice(&sample);
                chunk[2..].copy_from_slice(&sample);
            }
            peak = peak.max(frame_peak);

            match audio.write(&packet).await {
                Ok(()) => {
                    if frames == 0 {
                        println!(
                            "microphone: first frame = {} bytes, peak={}",
                            packet.len(),
                            frame_peak
                        );
                    }
                    frames += 1;

                    if frames == 100 {
                        println!("microphone: USB frames=100, peak={}", peak);
                        frames = 0;
                        peak = 0;
                    }
                }
                Err(error) => {
                    println!(
                        "microphone: IN write error = {:?}, frames={}, peak={}, frame_peak={}",
                        error, frames, peak, frame_peak
                    );
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

    let (rx_descriptors, _) = esp_hal::dma_descriptors!(I2S_FRAME_BYTES, 0);
    let i2s_rx = I2s::new(
        peripherals.I2S0,
        peripherals.DMA_CH0,
        I2sConfig::new_tdm_philips()
            .with_sample_rate(Rate::from_hz(48_000))
            .with_data_format(DataFormat::Data32Channel32)
            .with_channels(Channels::MONO),
    )
    .expect("failed to create I2S")
    .into_async()
    .i2s_rx
    .with_bclk(peripherals.GPIO5)
    .with_ws(peripherals.GPIO6)
    .with_din(peripherals.GPIO7)
    .build(rx_descriptors);
    println!("microphone: i2s rx ready: bclk=GPIO5 ws=GPIO6 din=GPIO7");

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

    let microphone = UsbMicrophoneClass::new(&mut builder, USB_AUDIO_MAX_PACKET_BYTES as u16);
    builder.handler(USB_MICROPHONE_HANDLER.init(microphone.handler));
    let device = builder.build();

    spawner.spawn(usb_task(device).expect("failed to spawn USB task"));
    spawner.spawn(
        microphone_capture(i2s_rx).expect("failed to spawn microphone capture task"),
    );
    spawner.spawn(
        microphone_stream(microphone.stream).expect("failed to spawn microphone USB task"),
    );
    println!("microphone: UAC1 ready (UART0 115200 8N1)");

    core::future::pending::<()>().await;
}
