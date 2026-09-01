use embassy_futures::join::{join, join5};
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
            speaker::{Speaker as UsbSpeakerClass, State as UsbSpeakerState},
        },
    },
};
use esp_hal::otg_fs::{
    Usb,
    asynch::{Config as UsbDriverConfig, Driver as UsbDriver},
};
use static_cell::StaticCell;
use usbd_hid::descriptor::{KeyboardReport, MouseReport};

use super::{
    audio::{AUDIO_FRAME_BYTES, AUDIO_FRAME_SAMPLES, MICROPHONE_FRAMES, SPEAKER_FRAMES},
    usb_microphone::{
        ControlHandler as UsbMicrophoneControlHandler, Microphone as UsbMicrophoneClass,
    },
};

pub(crate) const USB_HID_POLL_MS: u8 = 10;
const USB_HID_REPORT_BYTES: usize = 9;
const USB_MICROPHONE_PACKET_BYTES: usize = AUDIO_FRAME_BYTES;
const USB_SPEAKER_MAX_PACKET_BYTES: usize = AUDIO_FRAME_BYTES * 2;
const USB_AUDIO_FEEDBACK_48K: [u8; 3] = [0x00, 0x00, 0x0c];
const USB_MICROPHONE_FEEDBACK_REFRESH: FeedbackRefresh = FeedbackRefresh::Period8Frames;
const USB_EP_OUT_BUFFER_SIZE: usize = 256;
const USB_CONFIG_DESCRIPTOR_SIZE: usize = 512;
const USB_BOS_DESCRIPTOR_SIZE: usize = 128;
const USB_MSOS_DESCRIPTOR_SIZE: usize = 128;
const USB_CONTROL_BUFFER_SIZE: usize = 64;
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
        USB_MICROPHONE_PACKET_BYTES as u16,
        USB_MICROPHONE_FEEDBACK_REFRESH as u8,
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
    let mut microphone_audio = microphone.stream;
    let mut microphone_feedback = microphone.feedback;

    join(
        device.run(),
        join5(
            async move {
                loop {
                    speaker_stream.wait_connection().await;

                    loop {
                        let mut packet = [0; USB_SPEAKER_MAX_PACKET_BYTES];

                        match speaker_stream.read_packet(&mut packet).await {
                            Ok(size) if size > 0 => {
                                let mut frame = [0; AUDIO_FRAME_SAMPLES];
                                for (sample, chunk) in
                                    frame.iter_mut().zip(packet[..size].chunks_exact(2))
                                {
                                    *sample = i16::from_le_bytes([chunk[0], chunk[1]]);
                                }
                                let _ = SPEAKER_FRAMES.try_send(frame);
                            }
                            Ok(_) => {}
                            Err(_) => break,
                        }
                    }
                }
            },
            async move {
                loop {
                    speaker_feedback.wait_connection().await;

                    while speaker_feedback
                        .write_packet(&USB_AUDIO_FEEDBACK_48K)
                        .await
                        .is_ok()
                    {
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

                    loop {
                        let frame = MICROPHONE_FRAMES.receive().await;
                        let mut bytes = [0; USB_MICROPHONE_PACKET_BYTES];

                        for (sample, chunk) in frame.iter().zip(bytes.chunks_exact_mut(2)) {
                            chunk.copy_from_slice(&sample.to_le_bytes());
                        }

                        if microphone_audio.write(&bytes).await.is_err() {
                            break;
                        }
                    }
                }
            },
            async move {
                loop {
                    microphone_feedback.wait_enabled().await;

                    while microphone_feedback
                        .write(&USB_AUDIO_FEEDBACK_48K)
                        .await
                        .is_ok()
                    {
                        Timer::after(Duration::from_millis(
                            USB_MICROPHONE_FEEDBACK_REFRESH.frame_count() as u64,
                        ))
                        .await;
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
