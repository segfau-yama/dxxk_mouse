use embassy_usb::{
    Builder, Handler,
    control::{InResponse, OutResponse, Recipient, Request, RequestType},
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
const FEATURE_UNIT: u8 = 0x06;
const AS_GENERAL: u8 = 0x01;
const FORMAT_TYPE: u8 = 0x02;
const FORMAT_TYPE_I: u8 = 0x01;
const EP_GENERAL: u8 = 0x01;
const ADC_VERSION: u16 = 0x0100;
const PCM: u16 = 0x0001;
const INPUT_UNIT_ID: u8 = 0x01;
const FEATURE_UNIT_ID: u8 = 0x02;
const OUTPUT_UNIT_ID: u8 = 0x03;
const TERMINAL_USB_STREAMING: u16 = 0x0101;
const TERMINAL_MICROPHONE: u16 = 0x0201;
const CHANNELS: u8 = 2;
const CHANNEL_LEFT_RIGHT: u16 = 0x0003;
const SAMPLE_WIDTH_BYTES: u8 = 2;
const SAMPLE_WIDTH_BITS: u8 = 16;
const SAMPLE_RATE_48K: [u8; 3] = [0x80, 0xbb, 0x00];
const SET_CUR: u8 = 0x01;
const GET_CUR: u8 = 0x81;
const GET_MIN: u8 = 0x82;
const GET_MAX: u8 = 0x83;
const GET_RES: u8 = 0x84;
const MUTE_CONTROL: u8 = 0x01;
const VOLUME_CONTROL: u8 = 0x02;
const SAMPLING_FREQ_CONTROL: u8 = 0x01;

pub struct Microphone<'d, D: Driver<'d>> {
    pub stream: Stream<'d, D>,
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

pub struct ControlHandler {
    control_interface: InterfaceNumber,
    audio_endpoint: u8,
    volume: [i16; 3],
    mute: [u8; 3],
}

impl ControlHandler {
    fn new(control_interface: InterfaceNumber, audio_endpoint: u8) -> Self {
        Self {
            control_interface,
            audio_endpoint,
            volume: [0; 3],
            mute: [0; 3],
        }
    }

    fn is_audio_endpoint(&self, request: Request) -> bool {
        let endpoint = (request.index & 0xff) as u8;
        endpoint == self.audio_endpoint || endpoint == (self.audio_endpoint & 0x7f)
    }

    fn is_sample_rate(request: Request) -> bool {
        (request.value >> 8) as u8 == SAMPLING_FREQ_CONTROL
    }

    fn control_out_interface(&mut self, request: Request, data: &[u8]) -> OutResponse {
        let interface = (request.index & 0xff) as u8;
        let entity = (request.index >> 8) as u8;
        let control = (request.value >> 8) as u8;
        let channel = (request.value & 0xff) as usize;

        if interface != u8::from(self.control_interface) {
            return OutResponse::Rejected;
        }

        if entity != FEATURE_UNIT_ID || request.request != SET_CUR || channel >= self.volume.len() {
            return OutResponse::Rejected;
        }

        match control {
            MUTE_CONTROL if !data.is_empty() => {
                self.mute[channel] = data[0];
                OutResponse::Accepted
            }
            VOLUME_CONTROL if data.len() >= 2 => {
                self.volume[channel] = i16::from_le_bytes([data[0], data[1]]);
                OutResponse::Accepted
            }
            _ => OutResponse::Rejected,
        }
    }

    fn control_in_interface<'a>(
        &'a mut self,
        request: Request,
        buffer: &'a mut [u8],
    ) -> InResponse<'a> {
        let interface = (request.index & 0xff) as u8;
        let entity = (request.index >> 8) as u8;
        let control = (request.value >> 8) as u8;
        let channel = (request.value & 0xff) as usize;

        if interface != u8::from(self.control_interface) || entity != FEATURE_UNIT_ID {
            return InResponse::Rejected;
        }

        match (request.request, control) {
            (GET_CUR, MUTE_CONTROL) if channel < self.mute.len() => {
                buffer[0] = self.mute[channel];
                InResponse::Accepted(&buffer[..1])
            }
            (GET_CUR, VOLUME_CONTROL) if channel < self.volume.len() => {
                buffer[..2].copy_from_slice(&self.volume[channel].to_le_bytes());
                InResponse::Accepted(&buffer[..2])
            }
            (GET_MIN, VOLUME_CONTROL) if channel < self.volume.len() => {
                buffer[..2].copy_from_slice(&(-12_750i16).to_le_bytes());
                InResponse::Accepted(&buffer[..2])
            }
            (GET_MAX, VOLUME_CONTROL) if channel < self.volume.len() => {
                buffer[..2].copy_from_slice(&0i16.to_le_bytes());
                InResponse::Accepted(&buffer[..2])
            }
            (GET_RES, VOLUME_CONTROL) if channel < self.volume.len() => {
                buffer[..2].copy_from_slice(&256i16.to_le_bytes());
                InResponse::Accepted(&buffer[..2])
            }
            _ => InResponse::Rejected,
        }
    }

    fn control_out_endpoint(&self, request: Request, data: &[u8]) -> OutResponse {
        if request.request == SET_CUR && Self::is_sample_rate(request) && data == SAMPLE_RATE_48K {
            OutResponse::Accepted
        } else {
            OutResponse::Rejected
        }
    }

    fn control_in_endpoint<'a>(&self, request: Request, buffer: &'a mut [u8]) -> InResponse<'a> {
        if !Self::is_sample_rate(request) {
            return InResponse::Rejected;
        }

        match request.request {
            GET_CUR | GET_MIN | GET_MAX => {
                buffer[..3].copy_from_slice(&SAMPLE_RATE_48K);
                InResponse::Accepted(&buffer[..3])
            }
            GET_RES => {
                buffer[..3].copy_from_slice(&[0, 0, 0]);
                InResponse::Accepted(&buffer[..3])
            }
            _ => InResponse::Rejected,
        }
    }
}

impl Handler for ControlHandler {
    fn set_alternate_setting(&mut self, _interface: InterfaceNumber, _alternate_setting: u8) {}

    fn control_out(&mut self, request: Request, data: &[u8]) -> Option<OutResponse> {
        if request.request_type != RequestType::Class {
            return None;
        }

        match request.recipient {
            Recipient::Interface
                if (request.index & 0xff) as u8 == u8::from(self.control_interface) =>
            {
                Some(self.control_out_interface(request, data))
            }
            Recipient::Endpoint if self.is_audio_endpoint(request) => {
                Some(self.control_out_endpoint(request, data))
            }
            _ => None,
        }
    }

    fn control_in<'a>(
        &'a mut self,
        request: Request,
        buffer: &'a mut [u8],
    ) -> Option<InResponse<'a>> {
        if request.request_type != RequestType::Class {
            return None;
        }

        match request.recipient {
            Recipient::Interface
                if (request.index & 0xff) as u8 == u8::from(self.control_interface) =>
            {
                Some(self.control_in_interface(request, buffer))
            }
            Recipient::Endpoint if self.is_audio_endpoint(request) => {
                Some(self.control_in_endpoint(request, buffer))
            }
            _ => None,
        }
    }
}

impl<'d, D: Driver<'d>> Microphone<'d, D> {
    pub fn new(builder: &mut Builder<'d, D>, max_packet_size: u16) -> Self {
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

        let feature_unit = [
            FEATURE_UNIT,
            FEATURE_UNIT_ID,
            INPUT_UNIT_ID,
            1,
            MUTE_CONTROL | VOLUME_CONTROL,
            MUTE_CONTROL | VOLUME_CONTROL,
            MUTE_CONTROL | VOLUME_CONTROL,
            0,
        ];
        let total_length = 9 + 12 + (feature_unit.len() + 2) as u16 + 9;
        control_alt.descriptor(
            CS_INTERFACE,
            &[
                HEADER_SUBTYPE,
                ADC_VERSION as u8,
                (ADC_VERSION >> 8) as u8,
                total_length as u8,
                (total_length >> 8) as u8,
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
                CHANNEL_LEFT_RIGHT as u8,
                (CHANNEL_LEFT_RIGHT >> 8) as u8,
                0,
                0,
            ],
        );
        control_alt.descriptor(CS_INTERFACE, &feature_unit);
        control_alt.descriptor(
            CS_INTERFACE,
            &[
                OUTPUT_TERMINAL,
                OUTPUT_UNIT_ID,
                TERMINAL_USB_STREAMING as u8,
                (TERMINAL_USB_STREAMING >> 8) as u8,
                0,
                FEATURE_UNIT_ID,
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

        let endpoint =
            active.alloc_endpoint_in(EndpointType::Isochronous, None, max_packet_size, 1);
        let endpoint_address = endpoint.info().addr.into();
        active.endpoint_descriptor(
            endpoint.info(),
            SynchronizationType::Asynchronous,
            UsageType::DataEndpoint,
            &[0, 0],
        );
        active.descriptor(
            CS_ENDPOINT,
            &[EP_GENERAL, SAMPLING_FREQ_CONTROL, 0x02, 0, 0],
        );

        Self {
            stream: Stream { endpoint },
            handler: ControlHandler::new(control_interface_number, endpoint_address),
        }
    }
}
