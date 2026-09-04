#![no_std]
#![no_main]

use embassy_executor::Spawner;
use esp_backtrace as _;
use esp_hal::{
    i2s::master::{Channels, DataFormat, I2s, TdmConfig as I2sConfig},
    time::Rate,
    timer::timg::TimerGroup,
};

esp_bootloader_esp_idf::esp_app_desc!();

const SAMPLE_RATE_HZ: u32 = 48_000;
const FRAME_SAMPLES: usize = 960;
const MONO_FRAME_BYTES: usize = FRAME_SAMPLES * core::mem::size_of::<i16>();
const STEREO_FRAME_BYTES: usize = FRAME_SAMPLES * 2 * core::mem::size_of::<i16>();
const AUDIO: &[u8] = include_bytes!("s_ks002_48k_mono_s16le.raw");

#[esp_rtos::main]
async fn main(_spawner: Spawner) {
    esp_println::println!("i2s_sine boot");

    let peripherals = esp_hal::init(esp_hal::Config::default());

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0, peripherals.FROM_CPU_INTR0);

    let mut i2s_tx = I2s::new(
        peripherals.I2S0,
        peripherals.DMA_CH0,
        I2sConfig::new_tdm_philips()
            .with_sample_rate(Rate::from_hz(SAMPLE_RATE_HZ))
            .with_data_format(DataFormat::Data16Channel16)
            .with_channels(Channels::STEREO),
    )
    .expect("failed to create I2S")
    .into_async()
    .i2s_tx
    .with_bclk(peripherals.GPIO35)
    .with_ws(peripherals.GPIO36)
    .with_dout(peripherals.GPIO37)
    .build();

    let mut dma_buffer =
        esp_hal::dma_tx_buffer!(STEREO_FRAME_BYTES).expect("failed to allocate I2S DMA buffer");

    esp_println::println!("i2s_sine ready: s_ks002.wav");

    let mut frame = [0; STEREO_FRAME_BYTES];
    loop {
        for source_frame in AUDIO.chunks_exact(MONO_FRAME_BYTES) {
            for (sample, channels) in source_frame.chunks_exact(2).zip(frame.chunks_exact_mut(4)) {
                channels[..2].copy_from_slice(sample);
                channels[2..].copy_from_slice(sample);
            }

            dma_buffer.fill(&frame);
            match i2s_tx.write(dma_buffer) {
                Ok(transfer) => {
                    let (result, tx, buffer) = transfer.wait_async().await;
                    i2s_tx = tx;
                    dma_buffer = buffer;
                    if result.is_err() {
                        esp_println::println!("i2s_sine write error");
                    }
                }
                Err((_, tx, buffer)) => {
                    i2s_tx = tx;
                    dma_buffer = buffer;
                    esp_println::println!("i2s_sine DMA setup error");
                }
            }
        }
    }
}
