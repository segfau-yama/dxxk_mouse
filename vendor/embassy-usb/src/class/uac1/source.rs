//! Fixed 48 kHz asynchronous capture with no feedback or hardware controls.

use super::SampleWidth;
use super::class_codes::*;
use super::terminal_type::TerminalType;
use crate::control::{InResponse, OutResponse, Recipient, Request, RequestType};
use crate::descriptor::{SynchronizationType, UsageType};
use crate::driver::{Driver, Endpoint, EndpointAddress, EndpointError, EndpointIn, EndpointType};
use crate::types::InterfaceNumber;
use crate::{Builder, Handler};

const INPUT_ID: u8 = 1;
const OUTPUT_ID: u8 = 2;

/// Audio capture endpoint. A successful write queues a packet; the host may not
/// have received it yet.
pub struct AudioSourceEpIn<'d, D: Driver<'d>> {
    ep: D::EndpointIn,
}

impl<'d, D: Driver<'d>> AudioSourceEpIn<'d, D> {
    /// Queue one isochronous packet.
    pub async fn write(&mut self, buf: &[u8]) -> Result<(), EndpointError> {
        self.ep.write(buf).await
    }

    /// Wait for the streaming alternate setting to be enabled.
    pub async fn wait_enabled(&mut self) {
        self.ep.wait_enabled().await
    }
}

/// Fixed-rate UAC1 capture function.
pub struct AudioSource<'d, D: Driver<'d>> {
    /// Audio data endpoint.
    pub audio_ep_in: AudioSourceEpIn<'d, D>,
    /// Register with the USB builder.
    pub handler: AudioSourceControlHandler,
}

impl<'d, D: Driver<'d>> AudioSource<'d, D> {
    /// Stereo constructor retained for existing standalone examples.
    pub fn new(
        builder: &mut Builder<'d, D>,
        rates: &'static [u32],
        width: SampleWidth,
        terminal: Option<TerminalType>,
    ) -> Self {
        Self::build(builder, rates, width, terminal, 2)
    }

    /// Mono capture for a single I2S microphone.
    pub fn new_mono(
        builder: &mut Builder<'d, D>,
        rates: &'static [u32],
        width: SampleWidth,
        terminal: Option<TerminalType>,
    ) -> Self {
        Self::build(builder, rates, width, terminal, 1)
    }

    fn build(
        builder: &mut Builder<'d, D>,
        rates: &'static [u32],
        width: SampleWidth,
        terminal: Option<TerminalType>,
        channels: u8,
    ) -> Self {
        assert_eq!(rates, &[48_000], "AudioSource requires fixed 48 kHz");
        let mut function =
            builder.function(USB_AUDIO_CLASS, USB_AUDIOCONTROL_SUBCLASS, PROTOCOL_NONE);
        let mut control = function.interface();
        let control_number = control.interface_number();
        let mut alt = control.alt_setting(
            USB_AUDIO_CLASS,
            USB_AUDIOCONTROL_SUBCLASS,
            PROTOCOL_NONE,
            None,
        );
        // UAC1 4.3.2: HEADER (9) + INPUT_TERMINAL (12) + OUTPUT_TERMINAL (9).
        alt.descriptor(
            CS_INTERFACE,
            &[
                HEADER_SUBTYPE,
                0x00,
                0x01,
                30,
                0,
                1,
                u8::from(control_number) + 1,
            ],
        );
        let terminal: u16 = terminal.unwrap_or(TerminalType::InMicrophone).into();
        alt.descriptor(
            CS_INTERFACE,
            &[
                INPUT_TERMINAL,
                INPUT_ID,
                terminal as u8,
                (terminal >> 8) as u8,
                0,
                channels,
                if channels == 2 { 3 } else { 0 },
                0,
                0,
                0,
            ],
        );
        // No Feature Unit: do not advertise controls that do not affect PCM.
        alt.descriptor(
            CS_INTERFACE,
            &[OUTPUT_TERMINAL, OUTPUT_ID, 0x01, 0x01, 0, INPUT_ID, 0],
        );

        let mut streaming = function.interface();
        let stream_number = streaming.interface_number();
        streaming.alt_setting(
            USB_AUDIO_CLASS,
            USB_AUDIOSTREAMING_SUBCLASS,
            PROTOCOL_NONE,
            None,
        );
        let mut active = streaming.alt_setting(
            USB_AUDIO_CLASS,
            USB_AUDIOSTREAMING_SUBCLASS,
            PROTOCOL_NONE,
            None,
        );
        active.descriptor(CS_INTERFACE, &[AS_GENERAL, OUTPUT_ID, 0, 1, 0]);
        active.descriptor(
            CS_INTERFACE,
            &[
                FORMAT_TYPE,
                FORMAT_TYPE_I,
                channels,
                width as u8,
                width.in_bit() as u8,
                1,
                0x80,
                0xbb,
                0x00,
            ],
        );
        // 49 frames leave space for the device clock's positive rate error.
        let ep = active.alloc_endpoint_in(
            EndpointType::Isochronous,
            None,
            49 * channels as u16 * width as u16,
            1,
        );
        active.endpoint_descriptor(
            ep.info(),
            SynchronizationType::Asynchronous,
            UsageType::DataEndpoint,
            &[0, 0],
        );
        active.descriptor(CS_ENDPOINT, &[EP_GENERAL, 0, 0, 0, 0]);
        let endpoint = ep.info().addr;
        Self {
            audio_ep_in: AudioSourceEpIn { ep },
            handler: AudioSourceControlHandler {
                control_number,
                stream_number,
                endpoint,
            },
        }
    }
}

/// Reject unsupported controls only for this function; let other composite
/// functions handle their requests regardless of handler order.
pub struct AudioSourceControlHandler {
    control_number: InterfaceNumber,
    stream_number: InterfaceNumber,
    endpoint: EndpointAddress,
}

impl AudioSourceControlHandler {
    fn owns(&self, req: Request) -> bool {
        req.request_type == RequestType::Class
            && match req.recipient {
                Recipient::Interface => {
                    req.index as u8 == u8::from(self.control_number)
                        || req.index as u8 == u8::from(self.stream_number)
                }
                Recipient::Endpoint => req.index as u8 == u8::from(self.endpoint),
                _ => false,
            }
    }

    /// Address of the capture endpoint.
    pub fn get_audio_ep_addr(&self) -> u8 {
        self.endpoint.into()
    }
    /// Number of the audio control interface.
    pub fn get_ctrl_iface_num(&self) -> u8 {
        self.control_number.into()
    }
    /// Number of the audio streaming interface.
    pub fn get_stream_iface_num(&self) -> u8 {
        self.stream_number.into()
    }
}

impl Handler for AudioSourceControlHandler {
    fn control_out(&mut self, req: Request, _buf: &[u8]) -> Option<OutResponse> {
        self.owns(req).then_some(OutResponse::Rejected)
    }

    fn control_in<'a>(&'a mut self, req: Request, _buf: &'a mut [u8]) -> Option<InResponse<'a>> {
        self.owns(req).then_some(InResponse::Rejected)
    }
}
