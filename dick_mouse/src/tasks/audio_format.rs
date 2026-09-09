// Pure PCM/packet helpers, also exercised by tests/host_audio.
pub(crate) const MICROPHONE_RING_TARGET: usize = 192;

pub(crate) fn microphone_sample(slot: &[u8]) -> i16 {
    // INMP441: signed 24-bit data at bits 31..8 of a Philips 32-bit slot.
    (i32::from_le_bytes(slot.try_into().unwrap()) >> 16) as i16
}

pub(crate) fn speaker_frame(sample: i16, volume: u8, usb_gain_q15: u32) -> [u8; 8] {
    let pcm = i32::from(sample) * i32::from(volume) / 100;
    let pcm = pcm * usb_gain_q15.min(32768) as i32 / 32768;
    // Sign-extend before shifting; 16-bit PCM occupies the most significant
    // bits of each 32-bit slot. Keep the acoustic level identical to S16 TX.
    let slot = (pcm << 16).to_le_bytes();
    [
        slot[0], slot[1], slot[2], slot[3], slot[0], slot[1], slot[2], slot[3],
    ]
}

pub(crate) fn fade_to_zero(sample: i16) -> i16 {
    let faded = i32::from(sample) * 15 / 16;
    if faded.abs() < 8 { 0 } else { faded as i16 }
}

pub(crate) struct MicrophonePacketizer {
    priming: u8,
    correction_tick: u8,
}

impl MicrophonePacketizer {
    pub(crate) const fn new() -> Self {
        Self {
            priming: 4,
            correction_tick: 0,
        }
    }

    /// Return (sample count, startup silence). Queue immediately at Alt 1,
    /// allowing a few ms of RX headroom before consuming microphone samples.
    pub(crate) fn next(&mut self, available: usize) -> (usize, bool) {
        if self.priming != 0 {
            self.priming -= 1;
            return (48, true);
        }
        self.correction_tick = (self.correction_tick + 1) % 8;
        if self.correction_tick == 0 {
            if available > MICROPHONE_RING_TARGET + 48 {
                return (49, false);
            }
            if available >= 48 && available < MICROPHONE_RING_TARGET - 48 {
                return (47, false);
            }
        }
        (48, false)
    }
}
