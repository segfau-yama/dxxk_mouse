//! Read-only EP0 diagnostics. No audio data, UART output or endpoint resets.
use embassy_usb::{
    Handler,
    control::{InResponse, Recipient, Request, RequestType},
    driver::Direction,
    types::InterfaceNumber,
};

pub(crate) const SNAPSHOT_BYTES: usize = 32;
pub(crate) const DETAIL_BYTES: usize = 112;

pub(crate) struct MicrophoneDiagnostics {
    endpoint: u8,
    interface: u8,
    read_snapshot: fn(u8) -> Option<[u32; 22]>,
    resets: u32,
    alt0: u32,
    alt1: u32,
    alt: u32,
}

impl MicrophoneDiagnostics {
    pub(crate) fn new(
        endpoint: u8,
        interface: u8,
        read_snapshot: fn(u8) -> Option<[u32; 22]>,
    ) -> Self {
        Self {
            endpoint,
            interface,
            read_snapshot,
            resets: 0,
            alt0: 0,
            alt1: 0,
            alt: 0,
        }
    }
}

impl Handler for MicrophoneDiagnostics {
    fn reset(&mut self) {
        self.resets = self.resets.wrapping_add(1);
        self.alt = 0;
    }

    fn configured(&mut self, configured: bool) {
        if !configured {
            self.alt = 0;
        }
    }

    fn set_alternate_setting(&mut self, interface: InterfaceNumber, alt: u8) {
        if u8::from(interface) != self.interface {
            return;
        }
        self.alt = u32::from(alt);
        match alt {
            0 => self.alt0 = self.alt0.wrapping_add(1),
            1 => self.alt1 = self.alt1.wrapping_add(1),
            _ => {}
        }
    }

    fn control_in<'a>(&'a mut self, req: Request, buf: &'a mut [u8]) -> Option<InResponse<'a>> {
        // Vendor/device IN: value 0x4d49. Index 0: MIC1/32B (unchanged).
        // Index 1: MIC2/112B = same 8 header words + 20 driver detail words.
        // Do not intercept UAC, HID, MS OS descriptors or other vendor requests.
        if req.direction != Direction::In
            || req.request_type != RequestType::Vendor
            || req.recipient != Recipient::Device
            || req.request != 0x5a
            || req.value != 0x4d49
        {
            return None;
        }
        let (size, magic) = match req.index {
            0 => (SNAPSHOT_BYTES, *b"MIC1"),
            1 => (DETAIL_BYTES, *b"MIC2"),
            _ => return Some(InResponse::Rejected),
        };
        if usize::from(req.length) != size || buf.len() < size {
            return Some(InResponse::Rejected);
        }
        let Some(snapshot) = (self.read_snapshot)(self.endpoint) else {
            return Some(InResponse::Rejected);
        };
        let words = [
            u32::from_le_bytes(magic),
            u32::from(self.endpoint),
            snapshot[0],
            snapshot[1],
            self.resets,
            self.alt0,
            self.alt1,
            self.alt,
        ];
        for (chunk, word) in buf[..SNAPSHOT_BYTES].chunks_exact_mut(4).zip(words) {
            chunk.copy_from_slice(&word.to_le_bytes());
        }
        for (chunk, word) in buf[SNAPSHOT_BYTES..size]
            .chunks_exact_mut(4)
            .zip(&snapshot[2..])
        {
            chunk.copy_from_slice(&word.to_le_bytes());
        }
        Some(InResponse::Accepted(&buf[..size]))
    }
}
