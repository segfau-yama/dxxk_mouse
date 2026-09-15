use super::audio_format::{MicrophonePacketizer, fade_to_zero};
use super::usb_diagnostics::MicrophoneDiagnostics;
use core::sync::atomic::Ordering;
use embassy_futures::join::{join3, join4};
use embassy_sync::{blocking_mutex::raw::CriticalSectionRawMutex, channel::Channel};
use embassy_time::{Duration, Instant, Timer};
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
            speaker::{Speaker as UsbSpeakerClass, State as UsbSpeakerState, Volume},
        },
    },
};
use esp_hal::usb::otg::{
    Usb,
    embassy_usb_device::{Config as UsbDriverConfig, Driver as UsbDriver, fs_in_transfer_counts},
};
use heapless::spsc::{Consumer, Producer};
use static_cell::StaticCell;
use usbd_hid::descriptor::{KeyboardReport, MouseReport};

use super::audio::{
    MICROPHONE_STREAMING, SPEAKER_EPOCH, SPEAKER_FEEDBACK_Q14, SPEAKER_LAST_PACKET_MS,
    SPEAKER_RING_LEVEL, SPEAKER_STREAMING, SPEAKER_USB_GAIN_Q15, SpeakerSample,
    reset_speaker_feedback, update_speaker_feedback,
};

pub(crate) const USB_HID_POLL_MS: u8 = 10;
const USB_HID_REPORT_BYTES: usize = 9;
const USB_MICROPHONE_CHANNELS: usize = 1;
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
static USB_MICROPHONE_DIAGNOSTICS: StaticCell<MicrophoneDiagnostics> = StaticCell::new();
static USB_SPEAKER_STATE: StaticCell<UsbSpeakerState<'static>> = StaticCell::new();

#[embassy_executor::task]
pub async fn usb_task(
    usb: Usb<'static>,
    mut microphone_ring: Consumer<'static, i16>,
    mut speaker_ring: Producer<'static, SpeakerSample>,
) {
    let driver = UsbDriver::new(
        usb,
        USB_EP_OUT_BUFFER.init([0; USB_EP_OUT_BUFFER_SIZE]),
        UsbDriverConfig::default(),
    );

    let mut config = UsbConfig::new(0xc0de, 0x0001);
    config.manufacturer = Some("dick mouse");
    config.product = Some("DXXK USB Audio");
    config.serial_number = Some("0001");
    config.device_release = 0x0011; // Audio descriptor revision (mono microphone).

    let mut builder = UsbBuilder::new(
        driver,
        config,
        USB_CONFIG_DESCRIPTOR.init([0; USB_CONFIG_DESCRIPTOR_SIZE]),
        USB_BOS_DESCRIPTOR.init([0; USB_BOS_DESCRIPTOR_SIZE]),
        USB_MSOS_DESCRIPTOR.init([0; USB_MSOS_DESCRIPTOR_SIZE]),
        USB_CONTROL_BUFFER.init([0; USB_CONTROL_BUFFER_SIZE]),
    );

    let microphone = UsbMicrophoneClass::new_mono(
        &mut builder,
        &USB_MICROPHONE_SAMPLE_RATES,
        SampleWidth::Width2Byte,
        None,
    );

    // Capture the allocated address; EP1 is not assumed by the diagnostic reader.
    builder.handler(USB_MICROPHONE_DIAGNOSTICS.init(MicrophoneDiagnostics::new(
        microphone.handler.get_audio_ep_addr(),
        microphone.handler.get_stream_iface_num(),
        fs_in_transfer_counts,
    )));

    let speaker = UsbSpeakerClass::new(
        &mut builder,
        USB_SPEAKER_STATE.init(UsbSpeakerState::new()),
        USB_SPEAKER_MAX_PACKET_BYTES as u16,
        SampleWidth::Width2Byte,
        &[48_000],
        &[UsbAudioChannel::LeftFront],
        FeedbackRefresh::Period32Frames,
    );

    // Both handlers filter interface/endpoint ownership, including on composite devices.
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
    let speaker_control = speaker.control_monitor;

    join3(
        device.run(),
        async move {
            loop {
                let gain = match speaker_control.volume(UsbAudioChannel::LeftFront) {
                    Some(Volume::Muted) => 0,
                    Some(Volume::DeciBel(db)) => {
                        (libm::powf(10.0, db.min(0.0) / 20.0) * 32768.0) as u32
                    }
                    None => 32768,
                };
                SPEAKER_USB_GAIN_Q15.store(gain, Ordering::Release);
                speaker_control.changed().await;
            }
        },
        join4(
            async move {
                loop {
                    speaker_stream.wait_connection().await;
                    let epoch = SPEAKER_EPOCH.fetch_add(1, Ordering::AcqRel).wrapping_add(1);
                    SPEAKER_STREAMING.store(true, Ordering::Release);
                    SPEAKER_LAST_PACKET_MS
                        .store(Instant::now().as_millis() as u32, Ordering::Release);
                    reset_speaker_feedback();

                    loop {
                        let mut packet = [0; USB_SPEAKER_MAX_PACKET_BYTES];

                        match speaker_stream.read_packet(&mut packet).await {
                            Ok(size) if size > 0 => {
                                SPEAKER_LAST_PACKET_MS
                                    .store(Instant::now().as_millis() as u32, Ordering::Release);
                                for chunk in packet[..size].chunks_exact(2) {
                                    let sample = i16::from_le_bytes([chunk[0], chunk[1]]);
                                    let _ =
                                        speaker_ring.enqueue(SpeakerSample { epoch, pcm: sample });
                                }
                                let ring = speaker_ring.len() as u32;
                                SPEAKER_RING_LEVEL.store(ring, Ordering::Relaxed);
                            }
                            Ok(_) => {}
                            Err(_) => {
                                SPEAKER_STREAMING.store(false, Ordering::Release);
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
                    // The producer continuously drains I2S, but samples captured while
                    // the host had Alt 0 are not part of the next recording.
                    MICROPHONE_STREAMING.store(false, Ordering::Release);
                    let queued = microphone_ring.len();
                    for _ in 0..queued {
                        let _ = microphone_ring.dequeue();
                    }
                    MICROPHONE_STREAMING.store(true, Ordering::Release);
                    let mut last_sample = 0i16;
                    let mut packetizer = MicrophonePacketizer::new();

                    loop {
                        let (sample_count, priming) = packetizer.next(microphone_ring.len());
                        let packet_bytes = sample_count * USB_MICROPHONE_CHANNELS * 2;
                        let mut bytes = [0; USB_MICROPHONE_MAX_PACKET_BYTES];

                        if !priming {
                            for chunk in bytes[..packet_bytes].chunks_exact_mut(2) {
                                last_sample = match microphone_ring.dequeue() {
                                    Some(sample) => sample,
                                    None => fade_to_zero(last_sample),
                                };
                                chunk.copy_from_slice(&last_sample.to_le_bytes());
                            }
                        }

                        match microphone_audio.write(&bytes[..packet_bytes]).await {
                            Ok(()) => {}
                            Err(_) => {
                                MICROPHONE_STREAMING.store(false, Ordering::Release);
                                let queued = microphone_ring.len();
                                for _ in 0..queued {
                                    let _ = microphone_ring.dequeue();
                                }
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
