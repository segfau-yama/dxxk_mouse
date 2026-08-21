use dick_mouse::device::{Button, Speaker};
use esp_hal::{
    Async,
    gpio::{AnyPin, Input, InputConfig, Level, Pull},
    i2s::master::I2sTx,
    time::Instant,
};

use crate::{AUDIO_FRAME_BYTES, AUDIO_FRAME_SAMPLES, SPEAKER_AUDIO, button_change};

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
