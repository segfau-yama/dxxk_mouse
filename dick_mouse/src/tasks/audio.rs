use super::audio_format::{fade_to_zero, microphone_sample, speaker_frame};
use crate::device::Button;
use core::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering};
use embassy_time::{Duration, Timer};
use esp_hal::{
    Async,
    dma::DmaTxStreamBuf,
    gpio::{AnyPin, Input, InputConfig, Level, Pull},
    i2s::master::{I2sRx, I2sTx},
    pcnt::unit::Unit,
    time::Instant,
};
use heapless::spsc::{Consumer, Producer, Queue};
use static_cell::StaticCell;

pub(crate) const AUDIO_FRAME_SAMPLES: usize = 48;
// USB and application frames contain 16-bit PCM samples.
#[allow(dead_code)]
pub(crate) const AUDIO_FRAME_BYTES: usize = AUDIO_FRAME_SAMPLES * core::mem::size_of::<i16>();
// The INMP441 is read as one 32-bit slot per sample.
pub const I2S_FRAME_BYTES: usize = AUDIO_FRAME_SAMPLES * core::mem::size_of::<i32>();

// These are sample rings, not frame queues. USB and I2S run from independent clocks.
// heapless reserves one slot; usable capacity is 2047 mono samples.
pub(crate) const AUDIO_RING_CAPACITY: usize = 2048;
pub(crate) const SPEAKER_RING_TARGET: usize = 256;
const MICROPHONE_DMA_BUFFER_BYTES: usize = I2S_FRAME_BYTES * 16;
// Stereo S32 occupies 8 bytes/sample; retain about 21 ms of DMA headroom.
const SPEAKER_DMA_BUFFER_BYTES: usize = 8192;
pub(crate) const SPEAKER_DMA_CHUNK_BYTES: usize = 512;

fn reset_speaker_dma_buffer(buffer: DmaTxStreamBuf) -> DmaTxStreamBuf {
    let (descriptors, buffer) = buffer.split();
    let mut buffer =
        DmaTxStreamBuf::new(descriptors, buffer).expect("failed to reset speaker DMA buffer");
    let _ = buffer.push_with(|bytes| {
        bytes.fill(0);
        bytes.len()
    });
    buffer
}

// Transport validation must not clip the INMP441 signal.
const MICROPHONE_GAIN: i16 = 1;
pub(crate) const DEFAULT_VOLUME_PERCENT: u8 = 100;
pub(crate) const VOLUME_STEP_PERCENT: i32 = 5;

pub struct SpeakerSample {
    pub(crate) epoch: u32,
    pub(crate) pcm: i16,
}

pub static MICROPHONE_RING: StaticCell<Queue<i16, AUDIO_RING_CAPACITY>> = StaticCell::new();
pub static SPEAKER_RING: StaticCell<Queue<SpeakerSample, AUDIO_RING_CAPACITY>> = StaticCell::new();
pub(crate) static SPEAKER_EPOCH: AtomicU32 = AtomicU32::new(0);
pub(crate) static SPEAKER_RING_LEVEL: AtomicU32 = AtomicU32::new(0);
pub(crate) static SPEAKER_STREAMING: AtomicBool = AtomicBool::new(false);
pub(crate) static SPEAKER_LAST_PACKET_MS: AtomicU32 = AtomicU32::new(0);
pub(crate) static SPEAKER_USB_GAIN_Q15: AtomicU32 = AtomicU32::new(32768);
pub(crate) static MICROPHONE_STREAMING: AtomicBool = AtomicBool::new(false);
pub(crate) static SPEAKER_FEEDBACK_Q14: AtomicU32 = AtomicU32::new(48 << 14);
static SPEAKER_FEEDBACK_INTEGRAL: AtomicI32 = AtomicI32::new(0);

#[embassy_executor::task]
pub async fn microphone_task(
    mut i2s_rx: I2sRx<'static, Async>,
    mut microphone_ring: Producer<'static, i16>,
    mute_gpio: AnyPin<'static>,
    volume_unit: Unit<'static, 1>,
    volume_gpio_a: AnyPin<'static>,
    volume_gpio_b: AnyPin<'static>,
) {
    let mute_input = Input::new(mute_gpio, InputConfig::default().with_pull(Pull::Up));
    let mut mute_button = Button::new(mute_input.level(), Level::Low, 5);
    let mut muted = mute_button.is_pressed();
    let (mut volume_encoder, mut reported_count) =
        super::encoder::setup(&volume_unit, volume_gpio_a, volume_gpio_b);
    let mut volume = DEFAULT_VOLUME_PERCENT;
    let mut dma_buffer =
        esp_hal::dma_rx_stream_buffer!(MICROPHONE_DMA_BUFFER_BYTES, I2S_FRAME_BYTES);
    let mut bytes = [0; I2S_FRAME_BYTES];

    loop {
        let mut transfer = match i2s_rx.read(dma_buffer) {
            Ok(transfer) => transfer,
            Err((_, rx, buffer)) => {
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
                        super::encoder::detents(
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
                    let raw_sample = microphone_sample(chunk);
                    let sample = (i32::from(raw_sample.saturating_mul(MICROPHONE_GAIN))
                        * i32::from(volume)
                        / 100) as i16;

                    if MICROPHONE_STREAMING.load(Ordering::Acquire) {
                        // A full ring drops the newest sample; only the consumer moves its cursor.
                        let _ = microphone_ring.enqueue(sample);
                    }
                }
            }

            if wait_error {
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
    let ring = SPEAKER_RING_LEVEL.load(Ordering::Relaxed) as i32;
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
    mut speaker_ring: Consumer<'static, SpeakerSample>,
    mute_gpio: AnyPin<'static>,
    volume_unit: Unit<'static, 2>,
    volume_gpio_a: AnyPin<'static>,
    volume_gpio_b: AnyPin<'static>,
) {
    let mute_input = Input::new(mute_gpio, InputConfig::default().with_pull(Pull::Up));
    let mut mute_button = Button::new(mute_input.level(), Level::Low, 5);
    let mut muted = mute_button.is_pressed();
    let (mut volume_encoder, mut reported_count) =
        super::encoder::setup(&volume_unit, volume_gpio_a, volume_gpio_b);
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
    let mut epoch = SPEAKER_EPOCH.load(Ordering::Acquire);

    loop {
        let mut transfer = match i2s_tx.write(dma_buffer) {
            Ok(transfer) => transfer,
            Err((_, tx, buffer)) => {
                i2s_tx = tx;
                dma_buffer = reset_speaker_dma_buffer(buffer);
                // A persistent DMA setup error must still yield to USB enumeration.
                Timer::after(Duration::from_millis(1)).await;
                continue;
            }
        };

        loop {
            // Check TotalEof before consuming any ring data; stopped descriptors
            // must not absorb samples that will be discarded during reset.
            if transfer.is_done() {
                let (tx, buffer) = transfer.stop();
                i2s_tx = tx;
                dma_buffer = reset_speaker_dma_buffer(buffer);
                Timer::after(Duration::from_millis(1)).await;
                break;
            }
            if transfer.available_bytes() == 0 {
                if transfer.wait_for_available_async().await.is_err() {
                    let (tx, buffer) = transfer.stop();
                    i2s_tx = tx;
                    dma_buffer = reset_speaker_dma_buffer(buffer);
                    Timer::after(Duration::from_millis(1)).await;
                    break;
                }
            }

            let current_epoch = SPEAKER_EPOCH.load(Ordering::Acquire);
            if epoch != current_epoch {
                epoch = current_epoch;
                last_sample = 0;
                // Discard only old sessions. Fresh packets queued after Alt 1
                // are retained, even if the audio task handles the change late.
                while speaker_ring
                    .peek()
                    .is_some_and(|sample| sample.epoch != epoch)
                {
                    let _ = speaker_ring.dequeue();
                }
            }
            let now_ms = Instant::now().duration_since_epoch().as_millis();
            let streaming = SPEAKER_STREAMING.load(Ordering::Acquire)
                && (now_ms as u32).wrapping_sub(SPEAKER_LAST_PACKET_MS.load(Ordering::Acquire))
                    < 100;
            if !streaming {
                // Only this consumer advances the read cursor.
                let queued = speaker_ring.len();
                for _ in 0..queued {
                    let _ = speaker_ring.dequeue();
                }
            }
            mute_button = mute_button.update(mute_input.level(), now_ms);
            if mute_button.changed() && mute_button.is_pressed() {
                muted = !muted;
            }
            volume = i32::from(volume)
                .saturating_add(
                    super::encoder::detents(
                        &volume_unit,
                        &mut volume_encoder,
                        &mut reported_count,
                        now_ms,
                    )
                    .saturating_mul(VOLUME_STEP_PERCENT),
                )
                .clamp(0, 100) as u8;
            let volume = if muted { 0 } else { volume };
            let usb_gain = SPEAKER_USB_GAIN_Q15.load(Ordering::Acquire);

            while transfer.available_bytes() > 0 {
                let _ = transfer.push_with(|buffer| {
                    let mut written = 0;
                    for chunk in buffer.chunks_exact_mut(8) {
                        let sample = match speaker_ring.dequeue() {
                            Some(sample) if sample.epoch == epoch && streaming => {
                                last_sample = sample.pcm;
                                sample.pcm
                            }
                            _ => {
                                last_sample = fade_to_zero(last_sample);
                                last_sample
                            }
                        };
                        chunk.copy_from_slice(&speaker_frame(sample, volume, usb_gain));
                        written += 8;
                    }
                    written
                });
            }
            let ring = speaker_ring.len() as u32;
            SPEAKER_RING_LEVEL.store(ring, Ordering::Relaxed);
        }
    }
}
