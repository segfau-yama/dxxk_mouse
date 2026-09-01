use embassy_usb::{
    Builder, Handler,
    descriptor::{SynchronizationType, UsageType},
    driver::{Driver, Endpoint, EndpointError, EndpointIn, EndpointType},
    types::InterfaceNumber,
};

const USB_AUDIO_CLASS: u8 = 0x01;
const USB_AUDIOCONTROL_SUBCLASS: u8 = 0x01;
const USB_AUDIOSTREAMING_SUBCLASS: u8 = 0x02;
const PROTOCOL_NONE: u8 = 0x00;
const CS_INTERFACE: u8 = 0x24;
const CS_ENDPOINT: u8 = 0x25;
const HEADER_SUBTYPE: u8 = 0x01;
const INPUT_TERMINAL: u8 = 0x02;
const OUTPUT_TERMINAL: u8 = 0x03;
const AS_GENERAL: u8 = 0x01;
const FORMAT_TYPE: u8 = 0x02;
const FORMAT_TYPE_I: u8 = 0x01;
const EP_GENERAL: u8 = 0x01;
const ADC_VERSION: u16 = 0x0100;
const PCM: u16 = 0x0001;
const INPUT_UNIT_ID: u8 = 0x01;
const OUTPUT_UNIT_ID: u8 = 0x02;
const TERMINAL_USB_STREAMING: u16 = 0x0101;
const TERMINAL_MICROPHONE: u16 = 0x0201;
const CHANNELS: u8 = 1;
const CHANNEL_CONFIG_MONO: u16 = 0x0000;
const SAMPLE_WIDTH_BYTES: u8 = 2;
const SAMPLE_WIDTH_BITS: u8 = 16;
const SAMPLE_RATE_48K: [u8; 3] = [0x80, 0xbb, 0x00];

pub struct Microphone<'d, D: Driver<'d>> {
    pub stream: Stream<'d, D>,
    pub feedback: Stream<'d, D>,
    pub handler: ControlHandler,
}

pub struct Stream<'d, D: Driver<'d>> {
    endpoint: D::EndpointIn,
}

impl<'d, D: Driver<'d>> Stream<'d, D> {
    pub async fn write(&mut self, buffer: &[u8]) -> Result<(), EndpointError> {
        self.endpoint.write(buffer).await
    }

    pub async fn wait_enabled(&mut self) {
        self.endpoint.wait_enabled().await
    }
}

/// The minimal fixed-rate microphone exposes no Feature Unit and no endpoint controls.
/// Keep a handler value so existing composite-device construction can register it without
/// intercepting requests belonging to other USB classes.
pub struct ControlHandler;

impl Handler for ControlHandler {}

impl<'d, D: Driver<'d>> Microphone<'d, D> {
    pub fn new(
        builder: &mut Builder<'d, D>,
        max_packet_size: u16,
        feedback_interval_ms: u8,
    ) -> Self {
        let mut function =
            builder.function(USB_AUDIO_CLASS, USB_AUDIOCONTROL_SUBCLASS, PROTOCOL_NONE);

        let mut control_interface = function.interface();
        let control_interface_number = control_interface.interface_number();
        let stream_interface_number = InterfaceNumber(u8::from(control_interface_number) + 1);
        let mut control_alt = control_interface.alt_setting(
            USB_AUDIO_CLASS,
            USB_AUDIOCONTROL_SUBCLASS,
            PROTOCOL_NONE,
            None,
        );

        // UAC1 Appendix-B style topology:
        // Microphone Input Terminal -> USB Streaming Output Terminal.
        // No Feature Unit is needed for the fixed-rate bring-up sample.
        const AC_HEADER_LENGTH: u16 = 9;
        const INPUT_TERMINAL_LENGTH: u16 = 12;
        const OUTPUT_TERMINAL_LENGTH: u16 = 9;
        const TOTAL_AC_LENGTH: u16 =
            AC_HEADER_LENGTH + INPUT_TERMINAL_LENGTH + OUTPUT_TERMINAL_LENGTH;

        control_alt.descriptor(
            CS_INTERFACE,
            &[
                HEADER_SUBTYPE,
                ADC_VERSION as u8,
                (ADC_VERSION >> 8) as u8,
                TOTAL_AC_LENGTH as u8,
                (TOTAL_AC_LENGTH >> 8) as u8,
                1,
                u8::from(stream_interface_number),
            ],
        );
        control_alt.descriptor(
            CS_INTERFACE,
            &[
                INPUT_TERMINAL,
                INPUT_UNIT_ID,
                TERMINAL_MICROPHONE as u8,
                (TERMINAL_MICROPHONE >> 8) as u8,
                0,
                CHANNELS,
                CHANNEL_CONFIG_MONO as u8,
                (CHANNEL_CONFIG_MONO >> 8) as u8,
                0,
                0,
            ],
        );
        control_alt.descriptor(
            CS_INTERFACE,
            &[
                OUTPUT_TERMINAL,
                OUTPUT_UNIT_ID,
                TERMINAL_USB_STREAMING as u8,
                (TERMINAL_USB_STREAMING >> 8) as u8,
                0,
                INPUT_UNIT_ID,
                0,
            ],
        );

        let mut stream_interface = function.interface();
        let _inactive = stream_interface.alt_setting(
            USB_AUDIO_CLASS,
            USB_AUDIOSTREAMING_SUBCLASS,
            PROTOCOL_NONE,
            None,
        );
        let mut active = stream_interface.alt_setting(
            USB_AUDIO_CLASS,
            USB_AUDIOSTREAMING_SUBCLASS,
            PROTOCOL_NONE,
            None,
        );

        active.descriptor(
            CS_INTERFACE,
            &[AS_GENERAL, OUTPUT_UNIT_ID, 0, PCM as u8, (PCM >> 8) as u8],
        );
        active.descriptor(
            CS_INTERFACE,
            &[
                FORMAT_TYPE,
                FORMAT_TYPE_I,
                CHANNELS,
                SAMPLE_WIDTH_BYTES,
                SAMPLE_WIDTH_BITS,
                1,
                SAMPLE_RATE_48K[0],
                SAMPLE_RATE_48K[1],
                SAMPLE_RATE_48K[2],
            ],
        );

        let audio_endpoint =
            active.alloc_endpoint_in(EndpointType::Isochronous, None, max_packet_size, 1);
        let feedback_endpoint =
            active.alloc_endpoint_in(EndpointType::Isochronous, None, 4, feedback_interval_ms);

        // Fixed-rate async microphone: explicit feedback endpoint, no sampling-frequency control.
        active.endpoint_descriptor(
            audio_endpoint.info(),
            SynchronizationType::Asynchronous,
            UsageType::DataEndpoint,
            &[feedback_interval_ms, feedback_endpoint.info().addr.into()],
        );
        // Fixed 48 kHz is already declared by the Format Type descriptor, so advertise
        // no Sampling Frequency Control and no lock delay.
        active.descriptor(CS_ENDPOINT, &[EP_GENERAL, 0, 0, 0, 0]);
        active.endpoint_descriptor(
            feedback_endpoint.info(),
            SynchronizationType::NoSynchronization,
            UsageType::FeedbackEndpoint,
            &[],
        );

        Self {
            stream: Stream {
                endpoint: audio_endpoint,
            },
            feedback: Stream {
                endpoint: feedback_endpoint,
            },
            handler: ControlHandler,
        }
    }
}
