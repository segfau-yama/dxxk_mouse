#![no_std]
#![no_main]

use core::sync::atomic::{AtomicU32, Ordering};
use embassy_executor::Spawner;
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, channel::Channel};
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
    Async,
    i2s::master::{Channels, DataFormat, I2s, I2sTx, TdmConfig as I2sConfig},
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
const I2S_FRAME_BYTES: usize = 4 * 1024;
const USB_MAX_PACKET_SIZE: usize = 49 * core::mem::size_of::<i16>();
const RING_CAPACITY: usize = 2048;
const RING_START_WATERMARK: usize = 512;
const RING_TARGET: i32 = 1024;
const OUTPUT_ATTENUATION_SHIFT: u8 = 2;
const USB_CONFIG_DESCRIPTOR_SIZE: usize = 512;
const USB_BOS_DESCRIPTOR_SIZE: usize = 128;
const USB_MSOS_DESCRIPTOR_SIZE: usize = 128;
const USB_CONTROL_BUFFER_SIZE: usize = 128;
const USB_EP_OUT_BUFFER_SIZE: usize = 256;
const FEEDBACK_NOMINAL_Q14: i32 = (SAMPLE_RATE_HZ / 1000) as i32 * (1 << 14);

static SUPPORTED_SAMPLE_RATES: [u32; 1] = [SAMPLE_RATE_HZ];
static AUDIO_CHANNELS: [uac1::Channel; 1] = [uac1::Channel::LeftFront];
static SPEAKER_RING: Channel<CriticalSectionRawMutex, i16, RING_CAPACITY> = Channel::new();
static SPEAKER_FEEDBACK_Q14: AtomicU32 = AtomicU32::new(FEEDBACK_NOMINAL_Q14 as u32);
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
    let mut packet = [0u8; USB_MAX_PACKET_SIZE];
    let mut packets = 0u32;

    loop {
        stream.wait_connection().await;
        println!("speaker: USB OUT endpoint enabled");

        loop {
            match stream.read_packet(&mut packet).await {
                Ok(size) => {
                    packets = packets.wrapping_add(1);
                    let mut peak = 0i16;
                    for bytes in packet[..size].chunks_exact(2) {
                        let sample = i16::from_le_bytes([bytes[0], bytes[1]]);
                        peak = peak.max(sample.saturating_abs());
                        // Back-pressure the USB reader instead of dropping samples.
                        SPEAKER_RING.send(sample).await;
                    }
                    if packets % 1000 == 0 {
                        println!(
                            "speaker: packets={}, size={}, peak={}, ring={}",
                            packets,
                            size,
                            peak,
                            SPEAKER_RING.len()
                        );
                    }
                }
                Err(error) => {
                    println!(
                        "speaker: OUT read error = {:?}, ring={}",
                        error,
                        SPEAKER_RING.len()
                    );
                    break;
                }
            }
        }
    }
}

fn fill_tx_bytes(buf: &mut [u8], last_sample: &mut i16) -> usize {
    let bytes = buf.len() - (buf.len() % 4);
    for chunk in buf[..bytes].chunks_exact_mut(4) {
        let sample = match SPEAKER_RING.try_receive() {
            Ok(sample) => sample >> OUTPUT_ATTENUATION_SHIFT,
            // ponytail: hold the last sample only for a genuine producer underrun;
            // normal operation is paced by the ring and never inserts a zero block.
            Err(_) => *last_sample,
        };
        *last_sample = sample;
        let sample = sample.to_le_bytes();
        chunk[..2].copy_from_slice(&sample);
        chunk[2..].copy_from_slice(&sample);
    }
    bytes
}

fn update_feedback() {
    let ring = SPEAKER_RING.len() as u32;
    let target = RING_TARGET as u32;
    let nominal = FEEDBACK_NOMINAL_Q14 as u32;
    let delta = if ring >= target {
        (ring - target).saturating_mul(4)
    } else {
        (target - ring).saturating_mul(4)
    };
    let mut desired = if ring >= target {
        nominal.saturating_sub(delta)
    } else {
        nominal.saturating_add(delta)
    };
    let minimum = nominal - 2048;
    let maximum = nominal + 2048;
    if desired < minimum {
        desired = minimum;
    } else if desired > maximum {
        desired = maximum;
    }
    let current = SPEAKER_FEEDBACK_Q14.load(Ordering::Relaxed);
    let next = if desired >= current {
        current + (desired - current) / 8
    } else {
        current - (current - desired) / 8
    };
    SPEAKER_FEEDBACK_Q14.store(next, Ordering::Relaxed);
}

#[embassy_executor::task]
async fn speaker_output(mut i2s_tx: I2sTx<'static, Async>) {
    // ponytail: the esp-hal 1.2 stream-buffer path raises DescriptorError on
    // this ESP32-S3 board; re-arm the known-good finite DMA buffer instead.
    let mut dma_buffer =
        esp_hal::dma_tx_buffer!(I2S_FRAME_BYTES).expect("failed to allocate speaker DMA buffer");
    let mut bytes = [0u8; I2S_FRAME_BYTES];
    let mut last_sample = 0i16;

    loop {
        while SPEAKER_RING.len() < RING_START_WATERMARK {
            Timer::after(Duration::from_millis(1)).await;
        }

        fill_tx_bytes(&mut bytes, &mut last_sample);
        dma_buffer.fill(&bytes);
        let transfer = match i2s_tx.write(dma_buffer) {
            Ok(transfer) => transfer,
            Err((error, tx, buffer)) => {
                println!("speaker: i2s DMA setup error = {:?}", error);
                i2s_tx = tx;
                dma_buffer = buffer;
                Timer::after(Duration::from_millis(1)).await;
                continue;
            }
        };
        let (result, tx, buffer) = transfer.wait_async().await;
        i2s_tx = tx;
        dma_buffer = buffer;
        if result.is_ok() {
            update_feedback();
        } else {
            Timer::after(Duration::from_millis(1)).await;
        }
    }
}

#[embassy_executor::task]
async fn speaker_feedback(mut feedback: speaker::Feedback<'static, UsbDriver<'static>>) {
    loop {
        feedback.wait_connection().await;
        println!("speaker: feedback endpoint enabled");

        loop {
            let value = SPEAKER_FEEDBACK_Q14.load(Ordering::Relaxed) & 0x00ff_ffff;
            let packet = [value as u8, (value >> 8) as u8, (value >> 16) as u8];
            match feedback.write_packet(&packet).await {
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

    let i2s_tx = I2s::new(
        peripherals.I2S1,
        peripherals.DMA_CH1,
        I2sConfig::new_tdm_philips()
            .with_sample_rate(Rate::from_hz(SAMPLE_RATE_HZ))
            .with_data_format(DataFormat::Data16Channel16)
            .with_channels(Channels::STEREO),
    )
    .expect("failed to create speaker I2S")
    .into_async()
    .i2s_tx
    .with_bclk(peripherals.GPIO35)
    .with_ws(peripherals.GPIO36)
    .with_dout(peripherals.GPIO37)
    .build();
    println!("speaker: i2s tx ready: bclk=GPIO35 ws=GPIO36 dout=GPIO37");

    let usb = Usb::new_fs(peripherals.USB_FS, peripherals.GPIO20, peripherals.GPIO19);
    let driver = UsbDriver::new(
        usb,
        USB_EP_OUT_BUFFER.init([0; USB_EP_OUT_BUFFER_SIZE]),
        UsbDriverConfig::default(),
    );

    let mut config = UsbConfig::new(0xc0de, 0x0006);
    config.manufacturer = Some("dick mouse");
    config.product = Some("UAC1 speaker ring-buffer sample");
    config.serial_number = Some("speaker-ring-0001");

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
    spawner.spawn(speaker_stream(speaker.stream).expect("failed to spawn speaker USB task"));
    spawner.spawn(speaker_output(i2s_tx).expect("failed to spawn speaker I2S task"));
    spawner.spawn(speaker_feedback(speaker.feedback).expect("failed to spawn feedback task"));
    println!("speaker: I2S+USB ring-buffer UAC1 48k ready (UART0 115200 8N1)");

    core::future::pending::<()>().await;
}
