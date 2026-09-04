use core::sync::atomic::Ordering;
use embassy_futures::join::{join, join4};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, channel::Channel};
use embassy_time::{Duration, Timer};
use embassy_usb::{
    Builder as UsbBuilder, Config as UsbConfig,
    class::{
        hid::{
            Config as UsbHidConfig, HidBootProtocol, HidSubclass, HidWriter, State as UsbHidState,
        },
        uac1::{
            Channel as UsbAudioChannel, FeedbackRefresh, SampleWidth,
            source::{
                AudioSource as UsbMicrophoneClass,
                AudioSourceControlHandler as UsbMicrophoneControlHandler,
            },
            speaker::{Speaker as UsbSpeakerClass, State as UsbSpeakerState},
        },
    },
};
use esp_hal::usb::otg::{
    Usb,
    embassy_usb_device::{Config as UsbDriverConfig, Driver as UsbDriver},
};
use static_cell::StaticCell;
use usbd_hid::descriptor::{KeyboardReport, MouseReport};

use super::audio::{
    MICROPHONE_ALT0, MICROPHONE_ALT1, MICROPHONE_PACKET_47, MICROPHONE_PACKET_48,
    MICROPHONE_PACKET_49, MICROPHONE_RING, MICROPHONE_RING_HYSTERESIS,
    MICROPHONE_RING_LOW_WATERMARK, MICROPHONE_RING_MAX, MICROPHONE_RING_MIN, MICROPHONE_STREAMING,
    MICROPHONE_UNDERFLOWS, MICROPHONE_USB_ERRORS, MICROPHONE_USB_PACKETS, SPEAKER_ALT0,
    SPEAKER_ALT1, SPEAKER_FEEDBACK_Q14, SPEAKER_OVERFLOWS, SPEAKER_RING, SPEAKER_RING_MAX,
    SPEAKER_RING_MIN, SPEAKER_USB_ERRORS, SPEAKER_USB_PACKETS, reset_speaker_feedback,
    update_speaker_feedback,
};

pub(crate) const USB_HID_POLL_MS: u8 = 10;
const USB_HID_REPORT_BYTES: usize = 9;
const USB_MICROPHONE_CHANNELS: usize = 2;
const MICROPHONE_STARTUP_PACKETS: u8 = 8;
const USB_MICROPHONE_MAX_PACKET_BYTES: usize = 49 * USB_MICROPHONE_CHANNELS * 2;
// One mono 16-bit sample per USB frame: 48 nominal, 49 worst case.
const USB_SPEAKER_MAX_PACKET_BYTES: usize = 49 * core::mem::size_of::<i16>();
const USB_EP_OUT_BUFFER_SIZE: usize = 256;
const USB_CONFIG_DESCRIPTOR_SIZE: usize = 512;
const USB_BOS_DESCRIPTOR_SIZE: usize = 128;
const USB_MSOS_DESCRIPTOR_SIZE: usize = 128;
const USB_CONTROL_BUFFER_SIZE: usize = 128;
const USB_KEYBOARD_REPORT_ID: u8 = 1;
const USB_MOUSE_REPORT_ID: u8 = 2;
const USB_KEYBOARD_MOUSE_REPORT_DESCRIPTOR: &[u8] = &[
    0x05, 0x01, 0x09, 0x06, 0xa1, 0x01, 0x85, 0x01, 0x05, 0x07, 0x19, 0xe0, 0x29, 0xe7, 0x15, 0x00,
    0x25, 0x01, 0x75, 0x01, 0x95, 0x08, 0x81, 0x02, 0x19, 0x00, 0x29, 0xff, 0x26, 0xff, 0x00, 0x75,
    0x08, 0x95, 0x01, 0x81, 0x03, 0x05, 0x08, 0x19, 0x01, 0x29, 0x05, 0x25, 0x01, 0x75, 0x01, 0x95,
    0x05, 0x91, 0x02, 0x95, 0x03, 0x91, 0x03, 0x05, 0x07, 0x19, 0x00, 0x29, 0xdd, 0x26, 0xff, 0x00,
    0x75, 0x08, 0x95, 0x06, 0x81, 0x00, 0xc0, 0x05, 0x01, 0x09, 0x02, 0xa1, 0x01, 0x85, 0x02, 0x09,
    0x01, 0xa1, 0x00, 0x05, 0x09, 0x19, 0x01, 0x29, 0x08, 0x15, 0x00, 0x25, 0x01, 0x75, 0x01, 0x95,
    0x08, 0x81, 0x02, 0x05, 0x01, 0x09, 0x30, 0x17, 0x81, 0xff, 0xff, 0xff, 0x25, 0x7f, 0x75, 0x08,
    0x95, 0x01, 0x81, 0x06, 0x09, 0x31, 0x81, 0x06, 0x09, 0x38, 0x81, 0x06, 0x05, 0x0c, 0x0a, 0x38,
    0x02, 0x81, 0x06, 0xc0, 0xc0,
];

static USB_MICROPHONE_SAMPLE_RATES: [u32; 1] = [48_000];

pub(crate) enum UsbHidReport {
    Keyboard(KeyboardReport),
    Mouse(MouseReport),
}

pub(crate) static USB_HID_REPORTS: Channel<CriticalSectionRawMutex, UsbHidReport, 4> =
    Channel::new();
static USB_EP_OUT_BUFFER: StaticCell<[u8; USB_EP_OUT_BUFFER_SIZE]> = StaticCell::new();
static USB_CONFIG_DESCRIPTOR: StaticCell<[u8; USB_CONFIG_DESCRIPTOR_SIZE]> = StaticCell::new();
static USB_BOS_DESCRIPTOR: StaticCell<[u8; USB_BOS_DESCRIPTOR_SIZE]> = StaticCell::new();
static USB_MSOS_DESCRIPTOR: StaticCell<[u8; USB_MSOS_DESCRIPTOR_SIZE]> = StaticCell::new();
static USB_CONTROL_BUFFER: StaticCell<[u8; USB_CONTROL_BUFFER_SIZE]> = StaticCell::new();
static USB_HID_STATE: StaticCell<UsbHidState<'static>> = StaticCell::new();
static USB_MICROPHONE_HANDLER: StaticCell<UsbMicrophoneControlHandler> = StaticCell::new();
static USB_SPEAKER_STATE: StaticCell<UsbSpeakerState<'static>> = StaticCell::new();

#[embassy_executor::task]
pub async fn usb_task(usb: Usb<'static>) {
    let driver = UsbDriver::new(
        usb,
        USB_EP_OUT_BUFFER.init([0; USB_EP_OUT_BUFFER_SIZE]),
        UsbDriverConfig::default(),
    );

    let mut config = UsbConfig::new(0xc0de, 0x0001);
    config.manufacturer = Some("dick mouse");
    config.product = Some("DXXK USB Audio");
    config.serial_number = Some("0001");

    let mut builder = UsbBuilder::new(
        driver,
        config,
        USB_CONFIG_DESCRIPTOR.init([0; USB_CONFIG_DESCRIPTOR_SIZE]),
        USB_BOS_DESCRIPTOR.init([0; USB_BOS_DESCRIPTOR_SIZE]),
        USB_MSOS_DESCRIPTOR.init([0; USB_MSOS_DESCRIPTOR_SIZE]),
        USB_CONTROL_BUFFER.init([0; USB_CONTROL_BUFFER_SIZE]),
    );

    let microphone = UsbMicrophoneClass::new(
        &mut builder,
        &USB_MICROPHONE_SAMPLE_RATES,
        SampleWidth::Width2Byte,
        None,
    );

    let speaker = UsbSpeakerClass::new(
        &mut builder,
        USB_SPEAKER_STATE.init(UsbSpeakerState::new()),
        USB_SPEAKER_MAX_PACKET_BYTES as u16,
        SampleWidth::Width2Byte,
        &[48_000],
        &[UsbAudioChannel::LeftFront],
        FeedbackRefresh::Period32Frames,
    );

    // Speaker::new registers its own handler. Register the microphone handler after it so
    // speaker class requests are handled before AudioSource's control handler sees them.
    builder.handler(USB_MICROPHONE_HANDLER.init(microphone.handler));

    let mut hid_writer = HidWriter::<_, USB_HID_REPORT_BYTES>::new(
        &mut builder,
        USB_HID_STATE.init(UsbHidState::new()),
        UsbHidConfig {
            report_descriptor: USB_KEYBOARD_MOUSE_REPORT_DESCRIPTOR,
            request_handler: None,
            poll_ms: USB_HID_POLL_MS,
            max_packet_size: USB_HID_REPORT_BYTES as u16,
            hid_subclass: HidSubclass::No,
            hid_boot_protocol: HidBootProtocol::None,
        },
    );
    let mut device = builder.build();
    let mut speaker_stream = speaker.stream;
    let mut speaker_feedback = speaker.feedback;
    let mut microphone_audio = microphone.audio_ep_in;

    join(
        device.run(),
        join4(
            async move {
                loop {
                    speaker_stream.wait_connection().await;
                    SPEAKER_ALT1.fetch_add(1, Ordering::Relaxed);
                    SPEAKER_RING.clear();
                    reset_speaker_feedback();

                    loop {
                        let mut packet = [0; USB_SPEAKER_MAX_PACKET_BYTES];

                        match speaker_stream.read_packet(&mut packet).await {
                            Ok(size) if size > 0 => {
                                for chunk in packet[..size].chunks_exact(2) {
                                    let sample = i16::from_le_bytes([chunk[0], chunk[1]]);
                                    if SPEAKER_RING.try_send(sample).is_err() {
                                        SPEAKER_OVERFLOWS.fetch_add(1, Ordering::Relaxed);
                                    }
                                }
                                SPEAKER_USB_PACKETS.fetch_add(1, Ordering::Relaxed);
                                let ring = SPEAKER_RING.len() as u32;
                                SPEAKER_RING_MIN.fetch_min(ring, Ordering::Relaxed);
                                SPEAKER_RING_MAX.fetch_max(ring, Ordering::Relaxed);
                            }
                            Ok(_) => {}
                            Err(_) => {
                                SPEAKER_USB_ERRORS.fetch_add(1, Ordering::Relaxed);
                                SPEAKER_ALT0.fetch_add(1, Ordering::Relaxed);
                                break;
                            }
                        }
                    }
                }
            },
            async move {
                loop {
                    speaker_feedback.wait_connection().await;
                    loop {
                        update_speaker_feedback();
                        let value = SPEAKER_FEEDBACK_Q14.load(Ordering::Relaxed) & 0x00ff_ffff;
                        let packet = [value as u8, (value >> 8) as u8, (value >> 16) as u8];
                        if speaker_feedback.write_packet(&packet).await.is_err() {
                            SPEAKER_USB_ERRORS.fetch_add(1, Ordering::Relaxed);
                            break;
                        }
                        Timer::after(Duration::from_millis(
                            FeedbackRefresh::Period32Frames.frame_count() as u64,
                        ))
                        .await;
                    }
                }
            },
            async move {
                loop {
                    microphone_audio.wait_enabled().await;
                    MICROPHONE_ALT1.fetch_add(1, Ordering::Relaxed);
                    // The producer continuously drains I2S, but samples captured while
                    // the host had Alt 0 are not part of the next recording.
                    MICROPHONE_STREAMING.store(false, Ordering::Release);
                    MICROPHONE_RING.clear();
                    MICROPHONE_STREAMING.store(true, Ordering::Release);
                    let mut startup_packets = MICROPHONE_STARTUP_PACKETS;
                    let mut last_sample = 0i16;

                    loop {
                        let ring = MICROPHONE_RING.len();
                        let sample_count = if startup_packets != 0 {
                            // Do not use 47-sample packets while the freshly-cleared ring is
                            // being brought online.
                            MICROPHONE_PACKET_48.fetch_add(1, Ordering::Relaxed);
                            48
                        } else if ring
                            > super::audio::MICROPHONE_RING_TARGET + MICROPHONE_RING_HYSTERESIS
                        {
                            MICROPHONE_PACKET_49.fetch_add(1, Ordering::Relaxed);
                            49
                        } else if ring >= MICROPHONE_RING_LOW_WATERMARK
                            && ring + MICROPHONE_RING_HYSTERESIS
                                < super::audio::MICROPHONE_RING_TARGET
                        {
                            MICROPHONE_PACKET_47.fetch_add(1, Ordering::Relaxed);
                            47
                        } else {
                            MICROPHONE_PACKET_48.fetch_add(1, Ordering::Relaxed);
                            48
                        };
                        let packet_bytes = sample_count * USB_MICROPHONE_CHANNELS * 2;
                        let mut bytes = [0; USB_MICROPHONE_MAX_PACKET_BYTES];

                        // Embassy's current UAC1 AudioSource advertises two channels.
                        // Duplicate the mono microphone frame into left and right channels.
                        for chunk in bytes[..packet_bytes].chunks_exact_mut(4) {
                            let sample = match MICROPHONE_RING.try_receive() {
                                Ok(sample) => {
                                    last_sample = sample;
                                    sample
                                }
                                Err(_) => {
                                    MICROPHONE_UNDERFLOWS.fetch_add(1, Ordering::Relaxed);
                                    last_sample
                                }
                            };
                            let sample = sample.to_le_bytes();
                            chunk[..2].copy_from_slice(&sample);
                            chunk[2..].copy_from_slice(&sample);
                        }

                        match microphone_audio.write(&bytes[..packet_bytes]).await {
                            Ok(()) => {
                                startup_packets = startup_packets.saturating_sub(1);
                                MICROPHONE_USB_PACKETS.fetch_add(1, Ordering::Relaxed);
                                let ring = MICROPHONE_RING.len() as u32;
                                MICROPHONE_RING_MIN.fetch_min(ring, Ordering::Relaxed);
                                MICROPHONE_RING_MAX.fetch_max(ring, Ordering::Relaxed);
                            }
                            Err(_) => {
                                MICROPHONE_USB_ERRORS.fetch_add(1, Ordering::Relaxed);
                                MICROPHONE_ALT0.fetch_add(1, Ordering::Relaxed);
                                MICROPHONE_STREAMING.store(false, Ordering::Release);
                                MICROPHONE_RING.clear();
                                break;
                            }
                        }
                    }
                }
            },
            async move {
                loop {
                    hid_writer.ready().await;

                    loop {
                        match USB_HID_REPORTS.receive().await {
                            UsbHidReport::Keyboard(report) => {
                                let bytes = [
                                    USB_KEYBOARD_REPORT_ID,
                                    report.modifier,
                                    report.reserved,
                                    report.keycodes[0],
                                    report.keycodes[1],
                                    report.keycodes[2],
                                    report.keycodes[3],
                                    report.keycodes[4],
                                    report.keycodes[5],
                                ];

                                if hid_writer.write(&bytes).await.is_err() {
                                    break;
                                }
                            }
                            UsbHidReport::Mouse(report) => {
                                let bytes = [
                                    USB_MOUSE_REPORT_ID,
                                    report.buttons,
                                    report.x as u8,
                                    report.y as u8,
                                    report.wheel as u8,
                                    report.pan as u8,
                                ];

                                if hid_writer.write(&bytes).await.is_err() {
                                    break;
                                }
                            }
                        }
                    }
                }
            },
        ),
    )
    .await;
}
