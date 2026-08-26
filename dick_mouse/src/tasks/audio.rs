use dick_mouse::device::{Button, Microphone, RotaryEncoder, Speaker};
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
pub(crate) const AUDIO_FRAME_BYTES: usize = AUDIO_FRAME_SAMPLES * core::mem::size_of::<i16>();
pub(crate) type AudioFrame = [i16; AUDIO_FRAME_SAMPLES];

pub(crate) const DEFAULT_VOLUME_PERCENT: u8 = 100;
pub(crate) const VOLUME_STEP_PERCENT: i32 = 5;
const COUNTS_PER_DETENT: i32 = 4;

pub(crate) static MICROPHONE_FRAMES: Channel<CriticalSectionRawMutex, AudioFrame, 2> =
    Channel::new();
pub(crate) static SPEAKER_FRAMES: Channel<CriticalSectionRawMutex, AudioFrame, 2> = Channel::new();

pub(crate) fn bytes_to_audio_frame(bytes: &[u8]) -> AudioFrame {
    let mut frame = [0; AUDIO_FRAME_SAMPLES];

    for (sample, chunk) in frame.iter_mut().zip(bytes.chunks_exact(2)) {
        *sample = i16::from_le_bytes([chunk[0], chunk[1]]);
    }

    frame
}

pub(crate) fn volume_after_detents(volume: u8, detents: i32) -> u8 {
    i32::from(volume)
        .saturating_add(detents.saturating_mul(VOLUME_STEP_PERCENT))
        .clamp(0, 100) as u8
}

pub(crate) fn apply_volume(frame: &mut AudioFrame, volume: u8) {
    for sample in frame {
        *sample = (i32::from(*sample) * i32::from(volume) / 100) as i16;
    }
}

fn setup_volume_encoder<const NUM: usize>(
    unit: &Unit<'static, NUM>,
    gpio_a: AnyPin<'static>,
    gpio_b: AnyPin<'static>,
) -> (Input<'static>, Input<'static>, RotaryEncoder, i32) {
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
        input_a,
        input_b,
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
pub(crate) async fn microphone_task(
    mut i2s_rx: I2sRx<'static, Async>,
    mute_gpio: AnyPin<'static>,
    volume_unit: Unit<'static, 1>,
    volume_gpio_a: AnyPin<'static>,
    volume_gpio_b: AnyPin<'static>,
) {
    let mute_input = Input::new(mute_gpio, InputConfig::default().with_pull(Pull::Up));
    let mut mute_button = Button::new(mute_input.level(), Level::Low, 5);
    let mut muted = false;
    let (_volume_input_a, _volume_input_b, mut volume_encoder, mut reported_count) =
        setup_volume_encoder(&volume_unit, volume_gpio_a, volume_gpio_b);
    let mut volume = DEFAULT_VOLUME_PERCENT;

    loop {
        let now_ms = Instant::now().duration_since_epoch().as_millis();
        mute_button = mute_button.update(mute_input.level(), now_ms);
        if mute_button.changed() && mute_button.is_pressed() {
            muted = !muted;
        }
        volume = volume_after_detents(
            volume,
            encoder_detents(
                &volume_unit,
                &mut volume_encoder,
                &mut reported_count,
                now_ms,
            ),
        );

        let mut bytes = [0; AUDIO_FRAME_BYTES];

        if i2s_rx.read_dma_async(&mut bytes).await.is_ok() {
            let mut frame = bytes_to_audio_frame(&bytes);
            apply_volume(&mut frame, if muted { 0 } else { volume });
            let microphone = Microphone::new(frame);
            MICROPHONE_FRAMES.send(*microphone.buffer()).await;
        }

        Timer::after(Duration::from_millis(1)).await;
    }
}

#[embassy_executor::task]
pub(crate) async fn speaker_task(
    mut i2s_tx: I2sTx<'static, Async>,
    mute_gpio: AnyPin<'static>,
    volume_unit: Unit<'static, 2>,
    volume_gpio_a: AnyPin<'static>,
    volume_gpio_b: AnyPin<'static>,
) {
    let mute_input = Input::new(mute_gpio, InputConfig::default().with_pull(Pull::Up));
    let mut mute_button = Button::new(mute_input.level(), Level::Low, 5);
    let mut muted = false;
    let (_volume_input_a, _volume_input_b, mut volume_encoder, mut reported_count) =
        setup_volume_encoder(&volume_unit, volume_gpio_a, volume_gpio_b);
    let mut volume = DEFAULT_VOLUME_PERCENT;

    loop {
        let mut frame = SPEAKER_FRAMES.receive().await;
        let now_ms = Instant::now().duration_since_epoch().as_millis();
        mute_button = mute_button.update(mute_input.level(), now_ms);
        if mute_button.changed() && mute_button.is_pressed() {
            muted = !muted;
        }
        volume = volume_after_detents(
            volume,
            encoder_detents(
                &volume_unit,
                &mut volume_encoder,
                &mut reported_count,
                now_ms,
            ),
        );
        apply_volume(&mut frame, if muted { 0 } else { volume });
        let speaker = Speaker::new(frame);
        let mut bytes = [0; AUDIO_FRAME_BYTES];

        for (sample, chunk) in speaker.buffer().iter().zip(bytes.chunks_exact_mut(2)) {
            chunk.copy_from_slice(&sample.to_le_bytes());
        }

        let _ = i2s_tx.write_dma_async(&mut bytes).await;
    }
}
