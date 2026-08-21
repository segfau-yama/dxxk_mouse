#![no_std]
#![no_main]

use core::sync::atomic::AtomicBool;

use dick_mouse::device::Button;
use embassy_executor::Spawner;
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, channel::Channel, mutex::Mutex};
use embassy_usb::class::{
    hid::State as UsbHidState,
    uac1::{
        source::AudioSourceControlHandler as UsbMicrophoneControlHandler,
        speaker::State as UsbSpeakerState,
    },
};
use esp_backtrace as _;
use esp_hal::{
    gpio::{Level, Pin},
    i2s::master::{Channels, Config as I2sConfig, DataFormat, I2s},
    interrupt::software::SoftwareInterruptControl,
    otg_fs::Usb,
    pcnt::Pcnt,
    time::Rate,
    timer::timg::TimerGroup,
};
use static_cell::StaticCell;
use usbd_hid::descriptor::{KeyboardReport, KeyboardUsage, MouseReport};

esp_bootloader_esp_idf::esp_app_desc!();

mod tasks;

const AUDIO_FRAME_SAMPLES: usize = 48;
const AUDIO_FRAME_BYTES: usize = AUDIO_FRAME_SAMPLES * core::mem::size_of::<i16>();
const USB_HID_POLL_MS: u8 = 10;
const USB_KEYBOARD_REPORT_BYTES: usize = 8;
const USB_MOUSE_REPORT_BYTES: usize = 5;
const USB_AUDIO_MAX_PACKET_BYTES: usize = AUDIO_FRAME_BYTES * 2;
const USB_AUDIO_FEEDBACK_48K: [u8; 3] = [0x00, 0x00, 0x0c];
const USB_EP_OUT_BUFFER_SIZE: usize = 256;
const USB_CONFIG_DESCRIPTOR_SIZE: usize = 512;
const USB_BOS_DESCRIPTOR_SIZE: usize = 128;
const USB_MSOS_DESCRIPTOR_SIZE: usize = 128;
const USB_CONTROL_BUFFER_SIZE: usize = 64;
const GAME_JOYSTICK_THRESHOLD: i16 = 512;
const GAME_KEYS: [KeyboardUsage; 9] = [
    KeyboardUsage::KeyboardUpArrow,
    KeyboardUsage::KeyboardDownArrow,
    KeyboardUsage::KeyboardLeftArrow,
    KeyboardUsage::KeyboardRightArrow,
    KeyboardUsage::KeyboardSs,
    KeyboardUsage::KeyboardAa,
    KeyboardUsage::KeyboardDd,
    KeyboardUsage::KeyboardSpacebar,
    KeyboardUsage::KeyboardEnter,
];

type AudioFrame = [i16; AUDIO_FRAME_SAMPLES];

static GAME_MODE: AtomicBool = AtomicBool::new(false);
static GAME_BUTTON_BITS: Mutex<CriticalSectionRawMutex, usize> = Mutex::new(0);
static MICROPHONE_AUDIO: Channel<CriticalSectionRawMutex, AudioFrame, 2> = Channel::new();
static SPEAKER_AUDIO: Channel<CriticalSectionRawMutex, AudioFrame, 2> = Channel::new();
static USB_KEYBOARD_REPORTS: Channel<CriticalSectionRawMutex, KeyboardReport, 4> = Channel::new();
static USB_MOUSE_REPORTS: Channel<CriticalSectionRawMutex, MouseReport, 4> = Channel::new();
static USB_EP_OUT_BUFFER: StaticCell<[u8; USB_EP_OUT_BUFFER_SIZE]> = StaticCell::new();
static USB_CONFIG_DESCRIPTOR: StaticCell<[u8; USB_CONFIG_DESCRIPTOR_SIZE]> = StaticCell::new();
static USB_BOS_DESCRIPTOR: StaticCell<[u8; USB_BOS_DESCRIPTOR_SIZE]> = StaticCell::new();
static USB_MSOS_DESCRIPTOR: StaticCell<[u8; USB_MSOS_DESCRIPTOR_SIZE]> = StaticCell::new();
static USB_CONTROL_BUFFER: StaticCell<[u8; USB_CONTROL_BUFFER_SIZE]> = StaticCell::new();
static USB_KEYBOARD_HID_STATE: StaticCell<UsbHidState<'static>> = StaticCell::new();
static USB_MOUSE_HID_STATE: StaticCell<UsbHidState<'static>> = StaticCell::new();
static USB_MICROPHONE_HANDLER: StaticCell<UsbMicrophoneControlHandler> = StaticCell::new();
static USB_SPEAKER_STATE: StaticCell<UsbSpeakerState<'static>> = StaticCell::new();

fn bytes_to_audio_frame(bytes: &[u8]) -> AudioFrame {
    let mut frame = [0; AUDIO_FRAME_SAMPLES];

    for (sample, chunk) in frame.iter_mut().zip(bytes.chunks_exact(2)) {
        *sample = i16::from_le_bytes([chunk[0], chunk[1]]);
    }

    frame
}

fn button_change(button: &mut Button, measured_level: Level, now_ms: u64) -> Option<bool> {
    let (next_button, changed) = button.update(measured_level, now_ms);
    *button = next_button;
    changed.then(|| button.is_pressed())
}

async fn send_game_key(key: KeyboardUsage, pressed: bool) {
    let Some(key_index) = GAME_KEYS.iter().position(|game_key| *game_key == key) else {
        return;
    };

    let mut pressed_buttons = GAME_BUTTON_BITS.lock().await;
    let mask = 1usize << key_index;
    if ((*pressed_buttons & mask) != 0) == pressed {
        return;
    }

    if pressed {
        *pressed_buttons |= mask;
    } else {
        *pressed_buttons &= !mask;
    }

    let mut report = KeyboardReport::default();
    let mut keycode_index = 0;

    for (index, key) in GAME_KEYS.iter().copied().enumerate() {
        if *pressed_buttons & (1usize << index) != 0 && keycode_index < report.keycodes.len() {
            report.keycodes[keycode_index] = key as u8;
            keycode_index += 1;
        }
    }

    USB_KEYBOARD_REPORTS.send(report).await;
}

#[esp_rtos::main]
async fn main(spawner: Spawner) {
    let peripherals = esp_hal::init(esp_hal::Config::default());

    let sw_int = SoftwareInterruptControl::new(peripherals.SW_INTERRUPT);
    let timg0 = TimerGroup::new(peripherals.TIMG0);
    esp_rtos::start(timg0.timer0, sw_int.software_interrupt0);

    let pcnt = Pcnt::new(peripherals.PCNT);
    let (rx_descriptors, tx_descriptors) =
        esp_hal::dma_descriptors!(AUDIO_FRAME_BYTES, AUDIO_FRAME_BYTES);
    let i2s = I2s::new(
        peripherals.I2S0,
        peripherals.DMA_CH0,
        I2sConfig::new_tdm_philips()
            .with_sample_rate(Rate::from_hz(48_000))
            .with_data_format(DataFormat::Data16Channel16)
            .with_channels(Channels::MONO),
    )
    .expect("failed to create I2S")
    .into_async();
    let i2s_rx = i2s
        .i2s_rx
        .with_bclk(peripherals.GPIO15)
        .with_ws(peripherals.GPIO16)
        .with_din(peripherals.GPIO17)
        .build(rx_descriptors);
    let i2s_tx = i2s
        .i2s_tx
        .with_bclk(peripherals.GPIO8)
        .with_ws(peripherals.GPIO9)
        .with_dout(peripherals.GPIO10)
        .build(tx_descriptors);
    let usb = Usb::new(peripherals.USB0, peripherals.GPIO20, peripherals.GPIO19);

    spawner.spawn(
        tasks::mode_change::mode_change_task(peripherals.GPIO21.degrade())
            .expect("failed to create mode change task"),
    );
    spawner.spawn(
        tasks::mouse::mouse_task(
            pcnt.unit0,
            peripherals.GPIO11.degrade(),
            peripherals.GPIO12.degrade(),
            peripherals.ADC1,
            peripherals.GPIO1,
            peripherals.GPIO2,
            peripherals.GPIO13.degrade(),
            peripherals.GPIO14.degrade(),
        )
        .expect("failed to create mouse task"),
    );
    spawner.spawn(
        tasks::microphone::microphone_task(i2s_rx, peripherals.GPIO4.degrade())
            .expect("failed to create microphone task"),
    );
    spawner.spawn(tasks::usb::usb_task(usb).expect("failed to create usb task"));
    spawner.spawn(
        tasks::speaker::speaker_task(i2s_tx, peripherals.GPIO5.degrade())
            .expect("failed to create speaker task"),
    );
    spawner.spawn(
        tasks::keyboard::keyboard_task(
            peripherals.GPIO18.degrade(),
            peripherals.GPIO6.degrade(),
            peripherals.GPIO7.degrade(),
        )
        .expect("failed to create keyboard task"),
    );

    core::future::pending::<()>().await;
}
