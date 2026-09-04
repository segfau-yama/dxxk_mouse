use crate::device::{Button, Microphone, RotaryEncoder};
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, channel::Channel};
use embassy_time::{Duration, Timer};
use esp_hal::{
    Async,
    gpio::{AnyPin, Input, InputConfig, Level, Pull},
    i2s::master::{I2sRx, I2sTx},
    pcnt::{channel, unit::Unit},
    time::Instant,
};
use esp_println::println;

pub(crate) const AUDIO_FRAME_SAMPLES: usize = 48;
// USB and application frames contain 16-bit PCM samples.
pub(crate) const AUDIO_FRAME_BYTES: usize = AUDIO_FRAME_SAMPLES * core::mem::size_of::<i16>();
// The INMP441 is read as one 32-bit slot per sample.
pub const I2S_FRAME_BYTES: usize = AUDIO_FRAME_SAMPLES * core::mem::size_of::<i32>();
// Keep the I2S DMA transaction size below the ESP32-S3 GDMA descriptor limit.
// The esp-hal 1.2 stream-buffer path currently raises DescriptorError on this
// board, so use the known-good finite DMA buffer and immediately re-arm it.
const SPEAKER_I2S_FRAME_SAMPLES: usize = AUDIO_FRAME_SAMPLES * 20;
const SPEAKER_I2S_FRAME_BYTES: usize =
    SPEAKER_I2S_FRAME_SAMPLES * 2 * core::mem::size_of::<i16>();
const SPEAKER_RING_TARGET_FRAMES: i32 = 16;
pub(crate) type AudioFrame = [i16; AUDIO_FRAME_SAMPLES];

const MICROPHONE_GAIN: i16 = 4;
pub(crate) const DEFAULT_VOLUME_PERCENT: u8 = 100;
pub(crate) const VOLUME_STEP_PERCENT: i32 = 5;
const COUNTS_PER_DETENT: i32 = 4;

pub(crate) static MICROPHONE_FRAMES: Channel<CriticalSectionRawMutex, AudioFrame, 32> =
    Channel::new();
pub(crate) static SPEAKER_FRAMES: Channel<CriticalSectionRawMutex, AudioFrame, 32> = Channel::new();
pub(crate) static MICROPHONE_STREAMING: AtomicBool = AtomicBool::new(false);
pub(crate) static SPEAKER_FEEDBACK_Q14: AtomicU32 = AtomicU32::new(48 << 14);

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
        esp_hal::dma_rx_buffer!(I2S_FRAME_BYTES).expect("failed to allocate microphone DMA buffer");
    let mut bytes = [0; I2S_FRAME_BYTES];
    let mut frame = [0; AUDIO_FRAME_SAMPLES];
    let mut captured_frames = 0u32;
    let mut raw_peak = 0i16;
    let mut usb_peak = 0i16;

    loop {
        let transfer = match i2s_rx.read(dma_buffer) {
            Ok(transfer) => transfer,
            Err((_, rx, buffer)) => {
                i2s_rx = rx;
                dma_buffer = buffer;
                Timer::after(Duration::from_millis(1)).await;
                continue;
            }
        };

        let (result, rx, buffer) = transfer.wait_async().await;
        i2s_rx = rx;
        dma_buffer = buffer;
        if result.is_err() || dma_buffer.read_received_data(&mut bytes) != bytes.len() {
            Timer::after(Duration::from_millis(1)).await;
            continue;
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
        for (sample, chunk) in frame.iter_mut().zip(bytes.chunks_exact(4)) {
            // INMP441 data is left-aligned in each 32-bit I2S slot.
            let raw = i32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
            let sample16 = (raw >> 16) as i16;
            raw_peak = raw_peak.max(sample16.saturating_abs());
            *sample = sample16.saturating_mul(MICROPHONE_GAIN);
        }
        let volume = if muted { 0 } else { volume };
        for sample in &mut frame {
            *sample = (i32::from(*sample) * i32::from(volume) / 100) as i16;
            usb_peak = usb_peak.max(sample.saturating_abs());
        }
        captured_frames = captured_frames.wrapping_add(1);
        if captured_frames % 1_000 == 0 {
            println!(
                "microphone: i2s raw_peak={}, usb_peak={}, streaming={}",
                raw_peak,
                usb_peak,
                MICROPHONE_STREAMING.load(Ordering::Acquire)
            );
            raw_peak = 0;
            usb_peak = 0;
        }
        if MICROPHONE_STREAMING.load(Ordering::Acquire) {
            let microphone = Microphone::new(frame);
            MICROPHONE_FRAMES.send(*microphone.buffer()).await;
        }
    }
}

fn update_speaker_feedback() {
    const NOMINAL_Q14: u32 = 48 << 14;
    let ring = SPEAKER_FRAMES.len() as u32;
    let target = SPEAKER_RING_TARGET_FRAMES as u32;
    let delta = if ring >= target {
        (ring - target).saturating_mul(AUDIO_FRAME_SAMPLES as u32 * 4)
    } else {
        (target - ring).saturating_mul(AUDIO_FRAME_SAMPLES as u32 * 4)
    };
    let mut desired = if ring >= target {
        NOMINAL_Q14.saturating_sub(delta)
    } else {
        NOMINAL_Q14.saturating_add(delta)
    };
    let minimum = NOMINAL_Q14 - 2048;
    let maximum = NOMINAL_Q14 + 2048;
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
    let mut dma_buffer = esp_hal::dma_tx_buffer!(SPEAKER_I2S_FRAME_BYTES)
        .expect("failed to allocate speaker DMA buffer");

    loop {
        let mut samples = [0i16; SPEAKER_I2S_FRAME_SAMPLES];
        for frame in samples.chunks_exact_mut(AUDIO_FRAME_SAMPLES) {
            frame.copy_from_slice(&SPEAKER_FRAMES.receive().await);
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
        let mut bytes = [0u8; SPEAKER_I2S_FRAME_BYTES];
        for (sample, chunk) in samples.iter_mut().zip(bytes.chunks_exact_mut(4)) {
            *sample = (i32::from(*sample) * i32::from(volume) / 100) as i16;
            let sample = sample.to_le_bytes();
            chunk[..2].copy_from_slice(&sample);
            chunk[2..].copy_from_slice(&sample);
        }
        dma_buffer.fill(&bytes);

        let transfer = match i2s_tx.write(dma_buffer) {
            Ok(transfer) => transfer,
            Err((_, tx, buffer)) => {
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
            update_speaker_feedback();
        } else {
            Timer::after(Duration::from_millis(1)).await;
        }
    }
}
