use crate::device::{Button, Microphone, RotaryEncoder, Speaker};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, channel::Channel};
use embassy_time::{Duration, Timer};
use esp_hal::{
    Async,
    gpio::{AnyPin, Input, InputConfig, Level, Pull},
    i2s::master::{I2sRx, I2sTx},
    pcnt::{channel, unit::Unit},
    time::Instant,
};

pub(crate) const AUDIO_FRAME_SAMPLES: usize = 48;
// USB and application frames contain 16-bit PCM samples.
pub(crate) const AUDIO_FRAME_BYTES: usize = AUDIO_FRAME_SAMPLES * core::mem::size_of::<i16>();
// The INMP441 is read as one 32-bit slot per sample.
pub const I2S_FRAME_BYTES: usize = AUDIO_FRAME_SAMPLES * core::mem::size_of::<i32>();
pub(crate) type AudioFrame = [i16; AUDIO_FRAME_SAMPLES];

pub(crate) const DEFAULT_VOLUME_PERCENT: u8 = 100;
pub(crate) const VOLUME_STEP_PERCENT: i32 = 5;
const COUNTS_PER_DETENT: i32 = 4;

pub(crate) static MICROPHONE_FRAMES: Channel<CriticalSectionRawMutex, AudioFrame, 2> =
    Channel::new();
pub(crate) static SPEAKER_FRAMES: Channel<CriticalSectionRawMutex, AudioFrame, 2> = Channel::new();

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
    (
        RotaryEncoder::new(count, now_ms, 2),
        count,
    )
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

    loop {
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
        let mut bytes = [0; I2S_FRAME_BYTES];

        match i2s_rx.read_dma_async(&mut bytes).await {
            Ok(()) => {
                let mut frame = [0; AUDIO_FRAME_SAMPLES];
                for (sample, chunk) in frame.iter_mut().zip(bytes.chunks_exact(4)) {
                    // INMP441 data is left-aligned in each 32-bit I2S slot; keep its top 16 bits.
                    let raw = i32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
                    *sample = (raw >> 16) as i16;
                }
                let volume = if muted { 0 } else { volume };
                for sample in &mut frame {
                    *sample = (i32::from(*sample) * i32::from(volume) / 100) as i16;
                }
                let microphone = Microphone::new(frame);
                MICROPHONE_FRAMES.send(*microphone.buffer()).await;
            }
            Err(_) => Timer::after(Duration::from_millis(1)).await,
        }
    }
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

    loop {
        let mut frame = SPEAKER_FRAMES.receive().await;
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
        for sample in &mut frame {
            *sample = (i32::from(*sample) * i32::from(volume) / 100) as i16;
        }
        let speaker = Speaker::new(frame);
        let mut bytes = [0; I2S_FRAME_BYTES];

        for (sample, chunk) in speaker.buffer().iter().zip(bytes.chunks_exact_mut(4)) {
            let sample = i32::from(*sample) << 16;
            chunk.copy_from_slice(&sample.to_le_bytes());
        }

        let _ = i2s_tx.write_dma_async(&mut bytes).await;
    }
}
