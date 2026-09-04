#![no_std]
#![no_main]

use core::sync::atomic::{AtomicBool, Ordering};
use embassy_executor::Spawner;
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, channel::Channel};
use embassy_time::{Duration, Timer};
use embassy_usb::{
    Builder as UsbBuilder, Config as UsbConfig, UsbDevice,
    class::uac1::{
        SampleWidth,
        source::{AudioSource, AudioSourceControlHandler, AudioSourceEpIn},
    },
};
use esp_backtrace as _;
use esp_hal::{
    Async,
    i2s::master::{Channels, DataFormat, I2s, I2sRx, TdmConfig as I2sConfig},
    time::Rate,
    timer::timg::TimerGroup,
    usb::otg::{
        Usb,
        embassy_usb_device::{Config as UsbDriverConfig, Driver as UsbDriver},
    },
};
use esp_println::println;
use static_cell::StaticCell;

esp_bootloader_esp_idf::esp_app_desc!();

const SAMPLE_RATE_HZ: u32 = 48_000;
const MICROPHONE_GAIN: i16 = 4;
const I2S_FRAME_SAMPLES: usize = 48;
const I2S_FRAME_BYTES: usize = I2S_FRAME_SAMPLES * core::mem::size_of::<i32>();
const USB_CHANNELS: usize = 2;
const USB_SAMPLE_BYTES: usize = USB_CHANNELS * core::mem::size_of::<i16>();
const USB_MIN_SAMPLES: usize = 47;
const USB_NOMINAL_SAMPLES: usize = 48;
const USB_MAX_SAMPLES: usize = 49;
const USB_MAX_PACKET_BYTES: usize = USB_MAX_SAMPLES * USB_SAMPLE_BYTES;
const RING_CAPACITY: usize = 2048;
const RING_LOW_WATERMARK: usize = 240;
const RING_HIGH_WATERMARK: usize = 272;
const USB_CONFIG_DESCRIPTOR_SIZE: usize = 512;
const USB_BOS_DESCRIPTOR_SIZE: usize = 128;
const USB_MSOS_DESCRIPTOR_SIZE: usize = 128;
const USB_CONTROL_BUFFER_SIZE: usize = 128;
const USB_EP_OUT_BUFFER_SIZE: usize = 256;

static USB_SAMPLE_RATES: [u32; 1] = [SAMPLE_RATE_HZ];
static MICROPHONE_RING: Channel<CriticalSectionRawMutex, i16, RING_CAPACITY> = Channel::new();
static MICROPHONE_STREAMING: AtomicBool = AtomicBool::new(false);
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
async fn microphone_capture(mut i2s_rx: I2sRx<'static, Async>) {
    // ponytail: the esp-hal 1.2 stream-buffer path raises DescriptorError on
    // this ESP32-S3 board; re-arm the known-good finite DMA buffer instead.
    let mut dma_buffer =
        esp_hal::dma_rx_buffer!(I2S_FRAME_BYTES).expect("failed to allocate microphone DMA buffer");
    let mut input = [0u8; I2S_FRAME_BYTES];

    loop {
        let transfer = match i2s_rx.read(dma_buffer) {
            Ok(transfer) => transfer,
            Err((error, rx, buffer)) => {
                println!("microphone: i2s read error = {:?}", error);
                i2s_rx = rx;
                dma_buffer = buffer;
                Timer::after(Duration::from_millis(1)).await;
                continue;
            }
        };
        let (result, rx, buffer) = transfer.wait_async().await;
        i2s_rx = rx;
        dma_buffer = buffer;
        if result.is_err() || dma_buffer.read_received_data(&mut input) != input.len() {
            Timer::after(Duration::from_millis(1)).await;
            continue;
        }

        for raw in input.chunks_exact(4) {
            let sample = i32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]);
            // INMP441 data is left-aligned in each 32-bit I2S slot.
            if MICROPHONE_STREAMING.load(Ordering::Acquire) {
                MICROPHONE_RING
                    .send(((sample >> 16) as i16).saturating_mul(MICROPHONE_GAIN))
                    .await;
            }
        }
    }
}

fn samples_per_packet() -> usize {
    // ponytail: packet-level rate matching; use a resampler only if clock error exceeds ±1 sample/ms.
    match MICROPHONE_RING.len() {
        n if n > RING_HIGH_WATERMARK => USB_MAX_SAMPLES,
        n if n < RING_LOW_WATERMARK => USB_MIN_SAMPLES,
        _ => USB_NOMINAL_SAMPLES,
    }
}

#[embassy_executor::task]
async fn microphone_stream(mut audio: AudioSourceEpIn<'static, UsbDriver<'static>>) {
    let mut packet = [0u8; USB_MAX_PACKET_BYTES];
    let mut last_sample = 0i16;

    loop {
        audio.wait_enabled().await;
        while MICROPHONE_RING.try_receive().is_ok() {}
        MICROPHONE_STREAMING.store(true, Ordering::Release);
        println!("microphone: USB IN endpoint enabled");

        loop {
            let sample_count = samples_per_packet();
            for chunk in
                packet[..sample_count * USB_SAMPLE_BYTES].chunks_exact_mut(USB_SAMPLE_BYTES)
            {
                let sample = match MICROPHONE_RING.try_receive() {
                    Ok(sample) => {
                        last_sample = sample;
                        sample
                    }
                    Err(_) => last_sample,
                }
                .to_le_bytes();
                chunk[..2].copy_from_slice(&sample);
                chunk[2..4].copy_from_slice(&sample);
            }

            if let Err(error) = audio
                .write(&packet[..sample_count * USB_SAMPLE_BYTES])
                .await
            {
                println!(
                    "microphone: IN write error = {:?}, ring={}",
                    error,
                    MICROPHONE_RING.len()
                );
                MICROPHONE_STREAMING.store(false, Ordering::Release);
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

    let i2s_rx = I2s::new(
        peripherals.I2S0,
        peripherals.DMA_CH0,
        I2sConfig::new_tdm_philips()
            .with_sample_rate(Rate::from_hz(SAMPLE_RATE_HZ))
            .with_data_format(DataFormat::Data32Channel32)
            .with_channels(Channels::MONO),
    )
    .expect("failed to create I2S")
    .into_async()
    .i2s_rx
    .with_bclk(peripherals.GPIO5)
    .with_ws(peripherals.GPIO6)
    .with_din(peripherals.GPIO7)
    .build();
    println!("microphone: i2s rx ready: bclk=GPIO5 ws=GPIO6 din=GPIO7");

    let usb = Usb::new_fs(peripherals.USB_FS, peripherals.GPIO20, peripherals.GPIO19);
    let driver = UsbDriver::new(
        usb,
        USB_EP_OUT_BUFFER.init([0; USB_EP_OUT_BUFFER_SIZE]),
        UsbDriverConfig::default(),
    );

    let mut config = UsbConfig::new(0xc0de, 0x0005);
    config.manufacturer = Some("dick mouse");
    config.product = Some("UAC1 microphone ring-buffer sample");
    config.serial_number = Some("microphone-ring-0001");

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
        &USB_SAMPLE_RATES,
        SampleWidth::Width2Byte,
        None,
    );
    builder.handler(USB_MICROPHONE_HANDLER.init(handler));
    let device = builder.build();

    spawner.spawn(microphone_capture(i2s_rx).expect("failed to spawn microphone task"));
    spawner.spawn(usb_task(device).expect("failed to spawn USB task"));
    spawner.spawn(microphone_stream(audio_ep_in).expect("failed to spawn USB microphone task"));
    println!("microphone: I2S+USB ring-buffer UAC1 48k ready (UART0 115200 8N1)");

    core::future::pending::<()>().await;
}
