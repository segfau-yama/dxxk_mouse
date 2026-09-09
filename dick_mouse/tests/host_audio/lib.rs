#![cfg(test)]
#[path = "../../src/tasks/audio_format.rs"]
mod audio_format;
#[path = "../../src/tasks/usb_diagnostics.rs"]
mod usb_diagnostics;

use audio_format::*;
use embassy_usb::class::uac1::{SampleWidth, source::AudioSource};
use embassy_usb::control::{Recipient, Request, RequestType};
use embassy_usb::driver::*;
use embassy_usb::{Builder, Config, Handler};

#[test]
fn microphone_diagnostics_are_read_only_and_keep_queue_and_xfrc_separate() {
    use embassy_usb::{control::InResponse, types::InterfaceNumber};
    use usb_diagnostics::MicrophoneDiagnostics;
    let mut handler = MicrophoneDiagnostics::new(0x83, 5, |ep| {
        assert_eq!(ep, 0x83);
        Some((5001, 4999))
    });
    handler.reset();
    handler.set_alternate_setting(InterfaceNumber(4), 1); // another function
    handler.set_alternate_setting(InterfaceNumber(5), 0);
    handler.set_alternate_setting(InterfaceNumber(5), 1);
    let req = Request { direction: Direction::In, request_type: RequestType::Vendor,
        recipient: Recipient::Device, request: 0x5a, value: 0x4d49, index: 0, length: 32 };
    let mut buf = [0; 128];
    for _ in 0..2 {
        let Some(InResponse::Accepted(data)) = handler.control_in(req, &mut buf) else {
            panic!("diagnostic request rejected");
        };
        let words: Vec<_> = data.chunks_exact(4)
            .map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect();
        assert_eq!(words, [u32::from_le_bytes(*b"MIC1"), 0x83, 5001, 4999, 1, 1, 1, 1]);
    }
    handler.reset();
    let Some(InResponse::Accepted(data)) = handler.control_in(req, &mut buf) else { panic!() };
    assert_eq!(&data[8..16], &[0x89, 0x13, 0, 0, 0x87, 0x13, 0, 0]); // counters not reset
    assert_eq!(data[16], 2);
    assert_eq!(data[28], 0);
    assert!(handler.control_in(Request { request_type: RequestType::Class, ..req }, &mut buf).is_none());
    assert!(handler.control_in(Request { recipient: Recipient::Interface, ..req }, &mut buf).is_none());
    assert!(handler.control_in(Request { value: 0, ..req }, &mut buf).is_none());
    assert!(matches!(handler.control_in(Request { length: 64, ..req }, &mut buf), Some(InResponse::Rejected)));
    assert!(matches!(handler.control_in(req, &mut [0; 16]), Some(InResponse::Rejected)));
    assert!(handler.control_out(req, &[]).is_none());
}

// Allocation-only driver: exercise the actual descriptor builder without hardware.
struct TestDriver {
    next_in: usize,
}
struct Ep(EndpointInfo);
impl Endpoint for Ep {
    fn info(&self) -> &EndpointInfo {
        &self.0
    }
    async fn wait_enabled(&mut self) {}
}
impl EndpointIn for Ep {
    async fn write(&mut self, data: &[u8]) -> Result<(), EndpointError> {
        assert!(data.len() <= self.0.max_packet_size as usize);
        Ok(())
    }
}
impl EndpointOut for Ep {
    async fn read(&mut self, _: &mut [u8]) -> Result<usize, EndpointError> {
        unreachable!()
    }
}
struct TestBus;
impl Bus for TestBus {
    async fn enable(&mut self) {}
    async fn disable(&mut self) {}
    async fn poll(&mut self) -> Event {
        unreachable!()
    }
    fn endpoint_set_enabled(&mut self, _: EndpointAddress, _: bool) {}
    fn endpoint_set_stalled(&mut self, _: EndpointAddress, _: bool) {}
    fn endpoint_is_stalled(&mut self, _: EndpointAddress) -> bool {
        false
    }
    async fn remote_wakeup(&mut self) -> Result<(), Unsupported> {
        Err(Unsupported)
    }
}
struct TestControl;
impl ControlPipe for TestControl {
    fn max_packet_size(&self) -> usize {
        64
    }
    async fn setup(&mut self) -> [u8; 8] {
        unreachable!()
    }
    async fn data_out(&mut self, _: &mut [u8], _: bool, _: bool) -> Result<usize, EndpointError> {
        unreachable!()
    }
    async fn data_in(&mut self, _: &[u8], _: bool, _: bool) -> Result<(), EndpointError> {
        unreachable!()
    }
    async fn accept(&mut self) {}
    async fn reject(&mut self) {}
    async fn accept_set_address(&mut self, _: u8) {}
}
impl<'a> Driver<'a> for TestDriver {
    type EndpointOut = Ep;
    type EndpointIn = Ep;
    type Bus = TestBus;
    type ControlPipe = TestControl;
    fn alloc_endpoint_out(
        &mut self,
        _: EndpointType,
        _: Option<EndpointAddress>,
        _: u16,
        _: u8,
    ) -> Result<Ep, EndpointAllocError> {
        unreachable!()
    }
    fn alloc_endpoint_in(
        &mut self,
        ep_type: EndpointType,
        _: Option<EndpointAddress>,
        max_packet_size: u16,
        interval_ms: u8,
    ) -> Result<Ep, EndpointAllocError> {
        let addr = EndpointAddress::from_parts(self.next_in, Direction::In);
        self.next_in += 1;
        Ok(Ep(EndpointInfo {
            addr,
            ep_type,
            max_packet_size,
            interval_ms,
        }))
    }
    fn start(self, _: u16) -> (TestBus, TestControl) {
        (TestBus, TestControl)
    }
}

#[test]
fn mono_uac_descriptor_and_composite_request_routing() {
    let mut config = [0; 512];
    let mut bos = [0; 128];
    let mut msos = [0; 128];
    let mut control = [0; 128];
    {
        let mut builder = Builder::new(
            TestDriver { next_in: 1 },
            Config::new(0xc0de, 1),
            &mut config,
            &mut bos,
            &mut msos,
            &mut control,
        );
        // Put the mic after another function to catch hard-coded IF0/IF1 handling.
        {
            let mut function = builder.function(0xff, 0, 0);
            function.interface().alt_setting(0xff, 0, 0, None);
        }
        let source = AudioSource::new_mono(&mut builder, &[48000], SampleWidth::Width2Byte, None);
        let mut handler = source.handler;
        assert_eq!(handler.get_ctrl_iface_num(), 1);
        let own = Request {
            direction: Direction::In,
            request_type: RequestType::Class,
            recipient: Recipient::Interface,
            request: 0x81,
            value: 0x0200,
            index: 0x0201,
            length: 2,
        };
        let mut response = [0; 64];
        assert!(handler.control_in(own, &mut response).is_some());
        assert!(
            handler
                .control_in(
                    Request {
                        index: 0x0200,
                        ..own
                    },
                    &mut response
                )
                .is_none()
        );
        assert!(
            handler
                .control_out(
                    Request {
                        index: 0x0200,
                        ..own
                    },
                    &[]
                )
                .is_none()
        );
        assert!(
            handler
                .control_in(
                    Request {
                        recipient: Recipient::Endpoint,
                        index: 0x82,
                        ..own
                    },
                    &mut response
                )
                .is_none()
        );
        assert!(
            handler
                .control_out(
                    Request {
                        recipient: Recipient::Endpoint,
                        index: 0x81,
                        ..own
                    },
                    &[]
                )
                .is_some()
        );
        drop(builder.build());
    }
    let total = u16::from_le_bytes([config[2], config[3]]) as usize;
    let mut offset = 0;
    let mut endpoints = 0;
    let mut format = false;
    let mut controls = false;
    let mut interface = 0;
    while offset < total {
        let len = config[offset] as usize;
        assert!(len >= 2 && offset + len <= total);
        let d = &config[offset..offset + len];
        match d[1] {
            4 => {
                interface = d[2];
                if interface == 2 {
                    assert_eq!(d[4], d[3]);
                }
            }
            5 => {
                endpoints += 1;
                assert_eq!(d, &[9, 5, 0x81, 0x05, 98, 0, 1, 0, 0]);
            }
            0x24 if interface == 2 && d[2] == 2 => {
                assert_eq!(d, &[11, 0x24, 2, 1, 1, 2, 16, 1, 0x80, 0xbb, 0]);
                format = true;
            }
            0x24 if interface == 1 => {
                assert_ne!(d[2], 6, "must not advertise an unused Feature Unit")
            }
            0x25 => {
                assert_eq!(d, &[7, 0x25, 1, 0, 0, 0, 0]);
                controls = true;
            }
            _ => {}
        }
        offset += len;
    }
    assert_eq!(endpoints, 1);
    assert!(format && controls);
}

#[test]
fn pcm_width_sign_and_both_mute_paths() {
    for sample in [i16::MIN, -12345, -1, 0, 1, 12345, i16::MAX] {
        let bytes = speaker_frame(sample, 100, 32768);
        assert_eq!(
            i32::from_le_bytes(bytes[..4].try_into().unwrap()),
            i32::from(sample) << 16
        );
        assert_eq!(&bytes[..4], &bytes[4..]);
        assert_eq!(microphone_sample(&bytes[..4]), sample);
        assert_eq!(speaker_frame(sample, 0, 32768), [0; 8]);
        assert_eq!(speaker_frame(sample, 100, 0), [0; 8]);
    }
    assert_eq!(microphone_sample(&(-256i32).to_le_bytes()), -1);
    assert_eq!(
        speaker_frame(16000, 50, 16384),
        speaker_frame(4000, 100, 32768)
    );
    for mut held in [i16::MIN, i16::MAX] {
        for _ in 0..160 {
            held = fade_to_zero(held);
        }
        assert_eq!(held, 0);
    }
}

#[test]
fn microphone_startup_drift_and_reopen_are_bounded() {
    for drift_per_second in [-48i32, 0, 48] {
        // +/-1000 ppm
        for _reopen in 0..4 {
            let mut packetizer = MicrophonePacketizer::new();
            let mut available = 0usize;
            let mut corrections = 0;
            for ms in 0..20_000 {
                let (samples, priming) = packetizer.next(available);
                assert!((47..=49).contains(&samples));
                if ms < 4 {
                    assert_eq!((samples, priming), (48, true));
                }
                if !priming {
                    assert!(available >= samples, "underflow at {ms}");
                    available -= samples;
                    corrections += usize::from(samples != 48);
                }
                let adjustment = if ms % 1000 < drift_per_second.unsigned_abs() {
                    drift_per_second.signum()
                } else {
                    0
                };
                available += (48 + adjustment) as usize;
                assert!(available < 384, "unbounded startup/drift latency");
            }
            assert!(corrections < 2500, "must not send 47/49 continuously");
        }
    }
}

#[test]
fn spsc_preserves_order_across_wrap_and_overflow() {
    let mut ring = heapless::spsc::Queue::<i16, 2048>::new();
    let (mut producer, mut consumer) = ring.split();
    for _ in 0..10 {
        for n in 0..2047 {
            producer.enqueue(n).unwrap();
        }
        assert!(producer.enqueue(-1).is_err());
        for n in 0..2047 {
            assert_eq!(consumer.dequeue(), Some(n));
        }
        assert_eq!(consumer.dequeue(), None);
    }
}
