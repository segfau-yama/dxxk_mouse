//! Read-only EP0 diagnostics. No audio data, UART output or endpoint resets.
use embassy_usb::{Handler, control::{InResponse, Recipient, Request, RequestType},
    driver::Direction, types::InterfaceNumber};

pub(crate) const SNAPSHOT_BYTES: usize = 32;

pub(crate) struct MicrophoneDiagnostics {
    endpoint: u8,
    interface: u8,
    read_counts: fn(u8) -> Option<(u32, u32)>,
    resets: u32,
    alt0: u32,
    alt1: u32,
    alt: u32,
}

impl MicrophoneDiagnostics {
    pub(crate) fn new(endpoint: u8, interface: u8, read_counts: fn(u8) -> Option<(u32, u32)>) -> Self {
        Self { endpoint, interface, read_counts, resets: 0, alt0: 0, alt1: 0, alt: 0 }
    }
}

impl Handler for MicrophoneDiagnostics {
    fn reset(&mut self) {
        self.resets = self.resets.wrapping_add(1);
        self.alt = 0;
    }

    fn configured(&mut self, configured: bool) {
        if !configured { self.alt = 0; }
    }

    fn set_alternate_setting(&mut self, interface: InterfaceNumber, alt: u8) {
        if u8::from(interface) != self.interface { return; }
        self.alt = u32::from(alt);
        match alt {
            0 => self.alt0 = self.alt0.wrapping_add(1),
            1 => self.alt1 = self.alt1.wrapping_add(1),
            _ => {}
        }
    }

    fn control_in<'a>(&'a mut self, req: Request, buf: &'a mut [u8]) -> Option<InResponse<'a>> {
        // C0 5A 494D 0000 2000: vendor/device IN, value 0x4d49, 32 bytes.
        // Do not intercept UAC, HID, MS OS descriptors or other vendor requests.
        if req.direction != Direction::In || req.request_type != RequestType::Vendor
            || req.recipient != Recipient::Device || req.request != 0x5a
            || req.value != 0x4d49 || req.index != 0 {
            return None;
        }
        if usize::from(req.length) != SNAPSHOT_BYTES || buf.len() < SNAPSHOT_BYTES {
            return Some(InResponse::Rejected);
        }
        let Some((queued, completed)) = (self.read_counts)(self.endpoint) else {
            return Some(InResponse::Rejected);
        };
        let words = [u32::from_le_bytes(*b"MIC1"), u32::from(self.endpoint),
            queued, completed, self.resets, self.alt0, self.alt1, self.alt];
        for (chunk, word) in buf[..SNAPSHOT_BYTES].chunks_exact_mut(4).zip(words) {
            chunk.copy_from_slice(&word.to_le_bytes());
        }
        Some(InResponse::Accepted(&buf[..SNAPSHOT_BYTES]))
    }
}
