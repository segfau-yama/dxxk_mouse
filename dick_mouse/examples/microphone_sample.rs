#![no_std]
#![no_main]

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

const AUDIO_FRAME_SAMPLES: usize = 48;
const MICROPHONE_GAIN: i16 = 4;
const USB_AUDIO_CHANNELS: usize = 2;
const I2S_FRAME_BYTES: usize = AUDIO_FRAME_SAMPLES * core::mem::size_of::<i32>();
const USB_AUDIO_FRAME_BYTES: usize =
    AUDIO_FRAME_SAMPLES * USB_AUDIO_CHANNELS * core::mem::size_of::<i16>();
const USB_CONFIG_DESCRIPTOR_SIZE: usize = 512;
const USB_BOS_DESCRIPTOR_SIZE: usize = 128;
const USB_MSOS_DESCRIPTOR_SIZE: usize = 128;
const USB_CONTROL_BUFFER_SIZE: usize = 128;
const USB_EP_OUT_BUFFER_SIZE: usize = 256;
const MICROPHONE_QUEUE_DEPTH: usize = 8;

static USB_MICROPHONE_SAMPLE_RATES: [u32; 1] = [48_000];

type AudioFrame = [i16; AUDIO_FRAME_SAMPLES];

static MICROPHONE_FRAMES: Channel<CriticalSectionRawMutex, AudioFrame, MICROPHONE_QUEUE_DEPTH> =
    Channel::new();
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
    let mut dma_buffer =
        esp_hal::dma_rx_buffer!(I2S_FRAME_BYTES).expect("failed to allocate microphone DMA buffer");

    loop {
        let mut input = [0u8; I2S_FRAME_BYTES];

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
        if let Err(error) = result {
            println!("microphone: i2s read error = {:?}", error);
            Timer::after(Duration::from_millis(1)).await;
            continue;
        }
        if dma_buffer.read_received_data(&mut input) != input.len() {
            println!("microphone: short I2S frame");
            continue;
        }

        let mut frame = [0i16; AUDIO_FRAME_SAMPLES];

        for (sample, raw) in frame.iter_mut().zip(input.chunks_exact(4)) {
            let raw = i32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]);
            *sample = ((raw >> 16) as i16).saturating_mul(MICROPHONE_GAIN);
        }

        MICROPHONE_FRAMES.send(frame).await;
    }
}

#[embassy_executor::task]
async fn microphone_stream(mut audio: AudioSourceEpIn<'static, UsbDriver<'static>>) {
    loop {
        audio.wait_enabled().await;
        let mut frames = 0u32;

        loop {
            let samples = MICROPHONE_FRAMES.receive().await;
            let mut packet = [0u8; USB_AUDIO_FRAME_BYTES];

            // Embassy's UAC1 AudioSource currently advertises two channels.
            // Duplicate the mono INMP441 sample into left and right channels.
            for (sample, chunk) in samples.iter().zip(packet.chunks_exact_mut(4)) {
                let sample = sample.to_le_bytes();
                chunk[..2].copy_from_slice(&sample);
                chunk[2..].copy_from_slice(&sample);
            }

            match audio.write(&packet).await {
                Ok(()) => frames = frames.wrapping_add(1),
                Err(error) => {
                    println!(
                        "microphone: IN write error = {:?}, frames={}",
                        error, frames
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

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0, peripherals.FROM_CPU_INTR0);

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
    .build();
    println!("microphone: i2s rx ready: bclk=GPIO5 ws=GPIO6 din=GPIO7");

    let usb = Usb::new_fs(peripherals.USB_FS, peripherals.GPIO20, peripherals.GPIO19);
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

    spawner.spawn(microphone_capture(i2s_rx).expect("failed to spawn microphone capture task"));
    spawner.spawn(usb_task(device).expect("failed to spawn USB task"));
    spawner.spawn(microphone_stream(audio_ep_in).expect("failed to spawn microphone USB task"));
    println!("microphone: Embassy UAC1 source 48k S16_LE ready (UART0 115200 8N1)");

    core::future::pending::<()>().await;
}
