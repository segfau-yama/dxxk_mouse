use dick_mouse::device::{Button, Microphone};
use embassy_time::{Duration, Timer};
use esp_hal::{
    Async,
    gpio::{AnyPin, Input, InputConfig, Level, Pull},
    i2s::master::I2sRx,
    time::Instant,
};

use crate::{
    AUDIO_FRAME_BYTES, AUDIO_FRAME_SAMPLES, MICROPHONE_AUDIO, button_change, bytes_to_audio_frame,
};

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
