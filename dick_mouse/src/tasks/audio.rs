use crate::device::{Button, RotaryEncoder};
use core::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, channel::Channel};
use embassy_time::{Duration, Timer};
use esp_hal::{
    Async,
    dma::DmaTxStreamBuf,
    gpio::{AnyPin, Input, InputConfig, Level, Pull},
    i2s::master::{I2sRx, I2sTx},
    pcnt::{channel, unit::Unit},
    time::Instant,
};
use esp_println::println;

pub(crate) const AUDIO_FRAME_SAMPLES: usize = 48;
// USB and application frames contain 16-bit PCM samples.
#[allow(dead_code)]
pub(crate) const AUDIO_FRAME_BYTES: usize = AUDIO_FRAME_SAMPLES * core::mem::size_of::<i16>();
// The INMP441 is read as one 32-bit slot per sample.
pub const I2S_FRAME_BYTES: usize = AUDIO_FRAME_SAMPLES * core::mem::size_of::<i32>();

// These are sample rings, not frame queues. USB and I2S run from independent clocks.
pub(crate) const AUDIO_RING_CAPACITY: usize = 8192;
pub(crate) const MICROPHONE_RING_TARGET: usize = AUDIO_RING_CAPACITY / 2;
pub(crate) const SPEAKER_RING_TARGET: usize = AUDIO_RING_CAPACITY / 2;
const MICROPHONE_DMA_BUFFER_BYTES: usize = I2S_FRAME_BYTES * 16;
const SPEAKER_DMA_BUFFER_BYTES: usize = 4096;
pub(crate) const SPEAKER_DMA_CHUNK_BYTES: usize = 512;

fn reset_speaker_dma_buffer(buffer: DmaTxStreamBuf) -> DmaTxStreamBuf {
    let (descriptors, buffer) = buffer.split();
    DmaTxStreamBuf::new(descriptors, buffer).expect("failed to reset speaker DMA buffer")
}

// Transport validation must not clip the INMP441 signal.
const MICROPHONE_GAIN: i16 = 1;
pub(crate) const DEFAULT_VOLUME_PERCENT: u8 = 100;
pub(crate) const VOLUME_STEP_PERCENT: i32 = 5;
const COUNTS_PER_DETENT: i32 = 4;

pub(crate) static MICROPHONE_RING: Channel<CriticalSectionRawMutex, i16, AUDIO_RING_CAPACITY> =
    Channel::new();
pub(crate) static SPEAKER_RING: Channel<CriticalSectionRawMutex, i16, AUDIO_RING_CAPACITY> =
    Channel::new();
pub(crate) static MICROPHONE_STREAMING: AtomicBool = AtomicBool::new(false);
pub(crate) static SPEAKER_FEEDBACK_Q14: AtomicU32 = AtomicU32::new(48 << 14);
static SPEAKER_FEEDBACK_INTEGRAL: AtomicI32 = AtomicI32::new(0);

pub(crate) static MICROPHONE_ALT1: AtomicU32 = AtomicU32::new(0);
pub(crate) static MICROPHONE_ALT0: AtomicU32 = AtomicU32::new(0);
pub(crate) static MICROPHONE_USB_PACKETS: AtomicU32 = AtomicU32::new(0);
pub(crate) static MICROPHONE_PACKET_47: AtomicU32 = AtomicU32::new(0);
pub(crate) static MICROPHONE_PACKET_48: AtomicU32 = AtomicU32::new(0);
pub(crate) static MICROPHONE_PACKET_49: AtomicU32 = AtomicU32::new(0);
pub(crate) static MICROPHONE_UNDERFLOWS: AtomicU32 = AtomicU32::new(0);
pub(crate) static MICROPHONE_OVERFLOWS: AtomicU32 = AtomicU32::new(0);
pub(crate) static MICROPHONE_DMA_RESTARTS: AtomicU32 = AtomicU32::new(0);
pub(crate) static MICROPHONE_USB_ERRORS: AtomicU32 = AtomicU32::new(0);
pub(crate) static MICROPHONE_RING_MIN: AtomicU32 = AtomicU32::new(AUDIO_RING_CAPACITY as u32);
pub(crate) static MICROPHONE_RING_MAX: AtomicU32 = AtomicU32::new(0);

pub(crate) static SPEAKER_ALT1: AtomicU32 = AtomicU32::new(0);
pub(crate) static SPEAKER_ALT0: AtomicU32 = AtomicU32::new(0);
pub(crate) static SPEAKER_USB_PACKETS: AtomicU32 = AtomicU32::new(0);
pub(crate) static SPEAKER_UNDERFLOWS: AtomicU32 = AtomicU32::new(0);
pub(crate) static SPEAKER_OVERFLOWS: AtomicU32 = AtomicU32::new(0);
pub(crate) static SPEAKER_DMA_RESTARTS: AtomicU32 = AtomicU32::new(0);
pub(crate) static SPEAKER_USB_ERRORS: AtomicU32 = AtomicU32::new(0);
pub(crate) static SPEAKER_RING_MIN: AtomicU32 = AtomicU32::new(AUDIO_RING_CAPACITY as u32);
pub(crate) static SPEAKER_RING_MAX: AtomicU32 = AtomicU32::new(0);

fn setup_volume_encoder<const NUM: usize>(
    unit: &Unit<'static, NUM>,
    gpio_a: AnyPin<'static>,
    gpio_b: AnyPin<'static>,
) -> (RotaryEncoder, i32) {
    let input_a = Input::new(gpio_a, InputConfig::default().with_pull(Pull::Up));
    let input_b = Input::new(gpio_b, InputConfig::default().with_pull(Pull::Up));
    let signal_a = input_a.peripheral_input();
    let signal_b = input_b.peripheral_input();

    unit.set_filter(Some(800)).expect("invalid pcnt filter");

    let ch0 = &unit.channel0;
    ch0.set_ctrl_signal(signal_a.clone());
    ch0.set_edge_signal(signal_b.clone());
    ch0.set_ctrl_mode(channel::CtrlMode::Reverse, channel::CtrlMode::Keep);
    ch0.set_input_mode(channel::EdgeMode::Increment, channel::EdgeMode::Decrement);

    let ch1 = &unit.channel1;
    ch1.set_ctrl_signal(signal_b.clone());
    ch1.set_edge_signal(signal_a.clone());
    ch1.set_ctrl_mode(channel::CtrlMode::Reverse, channel::CtrlMode::Keep);
    ch1.set_input_mode(channel::EdgeMode::Decrement, channel::EdgeMode::Increment);

    let count = unit.value() as i32;
    let now_ms = Instant::now().duration_since_epoch().as_millis();
    (RotaryEncoder::new(count, now_ms, 2), count)
}

fn encoder_detents<const NUM: usize>(
    unit: &Unit<'static, NUM>,
    encoder: &mut RotaryEncoder,
    reported_count: &mut i32,
    now_ms: u64,
) -> i32 {
    *encoder = encoder.update(unit.value() as i32, now_ms);
    let detents = encoder.stable_count().saturating_sub(*reported_count) / COUNTS_PER_DETENT;

    if detents != 0 {
        *reported_count = reported_count.saturating_add(detents.saturating_mul(COUNTS_PER_DETENT));
    }

    detents
}

#[embassy_executor::task]
pub async fn microphone_task(
    mut i2s_rx: I2sRx<'static, Async>,
    mute_gpio: AnyPin<'static>,
    volume_unit: Unit<'static, 1>,
    volume_gpio_a: AnyPin<'static>,
    volume_gpio_b: AnyPin<'static>,
) {
    let mute_input = Input::new(mute_gpio, InputConfig::default().with_pull(Pull::Up));
    let mut mute_button = Button::new(mute_input.level(), Level::Low, 5);
    let mut muted = false;
    let (mut volume_encoder, mut reported_count) =
        setup_volume_encoder(&volume_unit, volume_gpio_a, volume_gpio_b);
    let mut volume = DEFAULT_VOLUME_PERCENT;
    let mut dma_buffer =
        esp_hal::dma_rx_stream_buffer!(MICROPHONE_DMA_BUFFER_BYTES, I2S_FRAME_BYTES);
    let mut bytes = [0; I2S_FRAME_BYTES];
    let mut captured_frames = 0u32;
    let mut raw_peak = 0i16;
    let mut usb_peak = 0i16;

    loop {
        let mut transfer = match i2s_rx.read(dma_buffer) {
            Ok(transfer) => transfer,
            Err((_, rx, buffer)) => {
                MICROPHONE_DMA_RESTARTS.fetch_add(1, Ordering::Relaxed);
                i2s_rx = rx;
                dma_buffer = buffer;
                Timer::after(Duration::from_millis(1)).await;
                continue;
            }
        };

        loop {
            // Drain all completed descriptors before handling an error. In particular,
            // DescriptorEmpty means the stream filled up; those samples are still valid.
            let wait_error = transfer.wait_for_available_async().await.is_err();

            while transfer.available_bytes() >= I2S_FRAME_BYTES {
                if transfer.pop(&mut bytes) != bytes.len() {
                    break;
                }

                let now_ms = Instant::now().duration_since_epoch().as_millis();
                mute_button = mute_button.update(mute_input.level(), now_ms);
                if mute_button.changed() && mute_button.is_pressed() {
                    muted = !muted;
                }
                volume = i32::from(volume)
                    .saturating_add(
                        encoder_detents(
                            &volume_unit,
                            &mut volume_encoder,
                            &mut reported_count,
                            now_ms,
                        )
                        .saturating_mul(VOLUME_STEP_PERCENT),
                    )
                    .clamp(0, 100) as u8;

                let volume = if muted { 0 } else { volume };
                for chunk in bytes.chunks_exact(4) {
                    // INMP441 data is left-aligned in each 32-bit I2S slot.
                    let raw = i32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
                    let raw_sample = (raw >> 16) as i16;
                    raw_peak = raw_peak.max(raw_sample.saturating_abs());
                    let sample = (i32::from(raw_sample.saturating_mul(MICROPHONE_GAIN))
                        * i32::from(volume)
                        / 100) as i16;
                    usb_peak = usb_peak.max(sample.saturating_abs());

                    if MICROPHONE_STREAMING.load(Ordering::Acquire) {
                        if MICROPHONE_RING.try_send(sample).is_err() {
                            // Keep the newest sample and record the exceptional overflow.
                            MICROPHONE_OVERFLOWS.fetch_add(1, Ordering::Relaxed);
                            let _ = MICROPHONE_RING.try_receive();
                            let _ = MICROPHONE_RING.try_send(sample);
                        }
                    }
                }

                let ring = MICROPHONE_RING.len() as u32;
                MICROPHONE_RING_MIN.fetch_min(ring, Ordering::Relaxed);
                MICROPHONE_RING_MAX.fetch_max(ring, Ordering::Relaxed);
                captured_frames = captured_frames.wrapping_add(1);
                if captured_frames % 10_000 == 0 {
                    println!(
                        "microphone: i2s raw_peak={}, usb_peak={}, ring={}, streaming={}",
                        raw_peak,
                        usb_peak,
                        ring,
                        MICROPHONE_STREAMING.load(Ordering::Acquire)
                    );
                    raw_peak = 0;
                    usb_peak = 0;
                }
            }

            if wait_error {
                MICROPHONE_DMA_RESTARTS.fetch_add(1, Ordering::Relaxed);
                let (rx, buffer) = transfer.stop();
                i2s_rx = rx;
                dma_buffer = buffer;
                Timer::after(Duration::from_millis(1)).await;
                break;
            }
        }
    }
}

pub(crate) fn update_speaker_feedback() {
    const NOMINAL_Q14: u32 = 48 << 14;
    // Low-bandwidth occupancy controller. The I2S stream is continuous, so this
    // observes a smooth sample ring instead of the old 20 ms dequeue burst.
    let ring = SPEAKER_RING.len() as i32;
    let error = ring - SPEAKER_RING_TARGET as i32;
    let integral = SPEAKER_FEEDBACK_INTEGRAL
        .load(Ordering::Relaxed)
        .saturating_add(error / 8)
        .clamp(-2048, 2048);
    SPEAKER_FEEDBACK_INTEGRAL.store(integral, Ordering::Relaxed);
    let correction = (error / 8 + integral / 8).clamp(-2048, 2048);
    let desired = (NOMINAL_Q14 as i32 - correction)
        .clamp((NOMINAL_Q14 - 2048) as i32, (NOMINAL_Q14 + 2048) as i32) as u32;
    let current = SPEAKER_FEEDBACK_Q14.load(Ordering::Relaxed);
    let next = if desired >= current {
        current + (desired - current) / 8
    } else {
        current - (current - desired) / 8
    };
    SPEAKER_FEEDBACK_Q14.store(next, Ordering::Relaxed);
}

pub(crate) fn reset_speaker_feedback() {
    SPEAKER_FEEDBACK_INTEGRAL.store(0, Ordering::Relaxed);
    SPEAKER_FEEDBACK_Q14.store(48 << 14, Ordering::Relaxed);
}

#[embassy_executor::task]
pub async fn speaker_task(
    mut i2s_tx: I2sTx<'static, Async>,
    mute_gpio: AnyPin<'static>,
    volume_unit: Unit<'static, 2>,
    volume_gpio_a: AnyPin<'static>,
    volume_gpio_b: AnyPin<'static>,
) {
    let mute_input = Input::new(mute_gpio, InputConfig::default().with_pull(Pull::Up));
    let mut mute_button = Button::new(mute_input.level(), Level::Low, 5);
    let mut muted = false;
    let (mut volume_encoder, mut reported_count) =
        setup_volume_encoder(&volume_unit, volume_gpio_a, volume_gpio_b);
    let mut volume = DEFAULT_VOLUME_PERCENT;
    let mut dma_buffer =
        esp_hal::dma_tx_stream_buffer!(SPEAKER_DMA_BUFFER_BYTES, SPEAKER_DMA_CHUNK_BYTES);
    // Start the clock with a complete buffer. At least two descriptors must be
    // ready before the ESP32-S3 GDMA transfer starts.
    let _ = dma_buffer.push_with(|buffer| {
        buffer.fill(0);
        buffer.len()
    });
    let mut last_sample = 0i16;

    loop {
        let mut transfer = match i2s_tx.write(dma_buffer) {
            Ok(transfer) => transfer,
            Err((_, tx, buffer)) => {
                SPEAKER_DMA_RESTARTS.fetch_add(1, Ordering::Relaxed);
                i2s_tx = tx;
                dma_buffer = reset_speaker_dma_buffer(buffer);
                let _ = dma_buffer.push_with(|buffer| {
                    buffer.fill(0);
                    buffer.len()
                });
                continue;
            }
        };

        loop {
            if transfer.available_bytes() == 0 {
                if transfer.wait_for_available_async().await.is_err() {
                    SPEAKER_DMA_RESTARTS.fetch_add(1, Ordering::Relaxed);
                    let (tx, buffer) = transfer.stop();
                    i2s_tx = tx;
                    dma_buffer = reset_speaker_dma_buffer(buffer);
                    let _ = dma_buffer.push_with(|buffer| {
                        buffer.fill(0);
                        buffer.len()
                    });
                    Timer::after(Duration::from_millis(1)).await;
                    break;
                }
            }

            let now_ms = Instant::now().duration_since_epoch().as_millis();
            mute_button = mute_button.update(mute_input.level(), now_ms);
            if mute_button.changed() && mute_button.is_pressed() {
                muted = !muted;
            }
            volume = i32::from(volume)
                .saturating_add(
                    encoder_detents(
                        &volume_unit,
                        &mut volume_encoder,
                        &mut reported_count,
                        now_ms,
                    )
                    .saturating_mul(VOLUME_STEP_PERCENT),
                )
                .clamp(0, 100) as u8;
            let volume = if muted { 0 } else { volume };

            while transfer.available_bytes() > 0 {
                let _ = transfer.push_with(|buffer| {
                    let mut written = 0;
                    for chunk in buffer.chunks_exact_mut(4) {
                        let sample = match SPEAKER_RING.try_receive() {
                            Ok(sample) => {
                                last_sample = sample;
                                sample
                            }
                            Err(_) => {
                                SPEAKER_UNDERFLOWS.fetch_add(1, Ordering::Relaxed);
                                last_sample
                            }
                        };
                        let sample = (i32::from(sample) * i32::from(volume) / 100) as i16;
                        let sample = sample.to_le_bytes();
                        chunk[..2].copy_from_slice(&sample);
                        chunk[2..].copy_from_slice(&sample);
                        written += 4;
                    }
                    written
                });
            }
            let ring = SPEAKER_RING.len() as u32;
            SPEAKER_RING_MIN.fetch_min(ring, Ordering::Relaxed);
            SPEAKER_RING_MAX.fetch_max(ring, Ordering::Relaxed);
        }
    }
}
