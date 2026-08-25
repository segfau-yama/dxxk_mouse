use dick_mouse::device::{Button, Microphone, Speaker, button::button_change};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, channel::Channel};
use embassy_time::{Duration, Timer};
use esp_hal::{
    Async,
    gpio::{AnyPin, Input, InputConfig, Level, Pull},
    i2s::master::{I2sRx, I2sTx},
    time::Instant,
};

pub(crate) const AUDIO_FRAME_SAMPLES: usize = 48;
pub(crate) const AUDIO_FRAME_BYTES: usize = AUDIO_FRAME_SAMPLES * core::mem::size_of::<i16>();
pub(crate) type AudioFrame = [i16; AUDIO_FRAME_SAMPLES];

pub(crate) static MICROPHONE_AUDIO: Channel<CriticalSectionRawMutex, AudioFrame, 2> =
    Channel::new();
pub(crate) static SPEAKER_AUDIO: Channel<CriticalSectionRawMutex, AudioFrame, 2> = Channel::new();

pub(crate) fn bytes_to_audio_frame(bytes: &[u8]) -> AudioFrame {
    let mut frame = [0; AUDIO_FRAME_SAMPLES];

    for (sample, chunk) in frame.iter_mut().zip(bytes.chunks_exact(2)) {
        *sample = i16::from_le_bytes([chunk[0], chunk[1]]);
    }

    frame
}

#[embassy_executor::task]
pub(crate) async fn microphone_task(mut i2s_rx: I2sRx<'static, Async>, mute_gpio: AnyPin<'static>) {
    let mut microphone = Microphone::new([0; AUDIO_FRAME_SAMPLES]);
    let mute_input = Input::new(mute_gpio, InputConfig::default().with_pull(Pull::Up));
    let mut mute_button = Button::new(mute_input.level(), Level::Low, 5);
    let mut muted = false;

    loop {
        let now_ms = Instant::now().duration_since_epoch().as_millis();
        if button_change(&mut mute_button, mute_input.level(), now_ms).unwrap_or(false) {
            muted = !muted;
        }

        let mut bytes = [0; AUDIO_FRAME_BYTES];

        if i2s_rx.read_dma_async(&mut bytes).await.is_ok() {
            let frame = if muted {
                [0; AUDIO_FRAME_SAMPLES]
            } else {
                bytes_to_audio_frame(&bytes)
            };
            microphone = microphone.update(frame);
            MICROPHONE_AUDIO.send(*microphone.buffer()).await;
        }

        Timer::after(Duration::from_millis(1)).await;
    }
}

#[embassy_executor::task]
pub(crate) async fn speaker_task(mut i2s_tx: I2sTx<'static, Async>, mute_gpio: AnyPin<'static>) {
    let mut speaker = Speaker::new([0; AUDIO_FRAME_SAMPLES]);
    let mute_input = Input::new(mute_gpio, InputConfig::default().with_pull(Pull::Up));
    let mut mute_button = Button::new(mute_input.level(), Level::Low, 5);
    let mut muted = false;

    loop {
        let now_ms = Instant::now().duration_since_epoch().as_millis();
        if button_change(&mut mute_button, mute_input.level(), now_ms).unwrap_or(false) {
            muted = !muted;
        }

        let pc_frame = SPEAKER_AUDIO.receive().await;
        let now_ms = Instant::now().duration_since_epoch().as_millis();
        if button_change(&mut mute_button, mute_input.level(), now_ms).unwrap_or(false) {
            muted = !muted;
        }

        let frame = if muted {
            [0; AUDIO_FRAME_SAMPLES]
        } else {
            pc_frame
        };
        speaker = speaker.update(frame);
        let mut bytes = [0; AUDIO_FRAME_BYTES];

        for (sample, chunk) in speaker.buffer().iter().zip(bytes.chunks_exact_mut(2)) {
            chunk.copy_from_slice(&sample.to_le_bytes());
        }

        let _ = i2s_tx.write_dma_async(&mut bytes).await;
    }
}
