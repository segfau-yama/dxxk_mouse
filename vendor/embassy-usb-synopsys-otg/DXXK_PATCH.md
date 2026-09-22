# DXXK IN lifecycle and diagnostic patch

Base: embassy-rs/embassy commit
`cd7570483a7036f23a9925a339152396bec4c041`. Dependency revisions are unchanged.

## IN lifecycle

- Every reset, enable/disable (including repeated Alt 1), power removal and
  deinit invalidates sleeping IN writes using a wrapping generation counter.
  `write` checks generation/enabled before any register access on each poll.
  Previous-transfer wait, FIFO wait and reservation share one critical section,
  so there is no unchecked gap after the last wait. A stale write returns
  `EndpointError::Disabled`, even after a false -> true enabled transition.
- Reconfiguration always stops the previous transfer before flushing its FIFO.
  NAK must be effective, and EPENA must actually clear after EPDIS. Stale EPDISD
  alone is not proof of a stop. Clear the affected endpoint's FIFO-empty mask,
  pending interrupts and transfer size only after the stop/flush succeeds.
- Stop/flush errors are propagated and counted, without UART output. On local
  failure the endpoint stays disabled; a subsequent Alt setting/reset retries
  initialization. No controller-wide automatic reset/re-enumeration is added.
  A still-busy shared GRSTCTL flush command is not overwritten by another TX/RX
  flush. A controller-wide stuck flush is not promised local recovery.
- Bus reset invalidates all writes and stops IN transfers **before** flushing
  their dedicated TX FIFOs. It keeps the immutable FIFO layout established by
  the first initialization (USB bus reset is not a core soft reset). Never use
  flush-all under a failed active transfer. Stop/flush failures are excluded
  from the ready mask; healthy endpoints including EP0 can still initialize.
  A later Alt selection restores type/MPS/FIFO as well as flushing the failed
  endpoint, so it can retry even if its Reset-time configuration was skipped.
- `fifo_layout_ready` is cleared by deinit/core soft reset. On the very first
  FIFO allocation, every IN transfer must stop before shared RAM can be resized.
  If that prerequisite or EP0's own stop/flush fails, enumeration cannot be
  guaranteed; the next host reset can retry. This is distinct from isolating
  a failed microphone stop during normal reconnect with a known FIFO layout.
- Incomplete-ISO cleanup is serialized with endpoint changes and ignores
  disabled endpoints and reset/suspend/disconnect events. An expired packet is
  stopped and its dedicated FIFO flushed, rather than re-enabled with residual
  TSIZ/FIFO state. Wake the next write to install fresh data, TSIZ and frame
  parity. Successful cleanup preserves the Alt setting/generation; failure
  disables only the affected endpoint. This applies to ISO feedback IN too;
  bulk/interrupt/control IN and audio OUT are not changed by this recovery.
- An ISO write with EPENA=0, PKTCNT=0, XFRSIZ=0 but insufficient FIFO space
  cannot make progress just by waiting for TXFE. Reclaim only that idle FIFO,
  recheck space and then submit the fresh packet. Never flush a live transfer
  or a bulk/interrupt/control endpoint through this path. Flush failure returns
  `Disabled` and increments the existing failure/cancellation counters.
- No elapsed-time watchdog assumes the host is polling the microphone.
  Discarding an expired ISO packet is exceptional recovery, not a normal
  audio rate-matching/sample-drop policy. Correlate counters with usbmon:
  sustained incomplete packets or zero-length captures still fail acceptance.
- OUT stop/prime behavior is unchanged. UAC descriptors, packetizer, I2S DMA,
  ring sizes and sample gain are unchanged.

Production waits retain the base's 10 ms limit when `embassy-time` is enabled
(enabled by esp-hal's OTG `host` feature in DXXK). Without that feature the base
wait remains unbounded. RAM-backed unit tests use a deterministic iteration
limit instead of waiting for nonexistent hardware.

## Counter semantics

- `EndpointIn::write`: increment the endpoint's queued count after FIFO submission,
  inside the existing critical section.
- IN interrupt handler: increment completed only for `DIEPINT.XFRC`, immediately
  before the existing write-one-to-clear acknowledgement.
- `State::in_transfer_counts`: read a consistent pair using the existing mutex.
- `in_started` counts valid-size write attempts when their future is first
  polled, not each repoll. `in_cancelled` counts attempts returning `Disabled`.
- `in_phase`: 0 idle/returned, 1 wait_enabled, 2 previous EPENA, 3 FIFO space.
  It is the most recently polled phase, not a hardware endpoint state. There
  is no separate "queued means completed" phase. A cancelled/dropped future
  that was never repolled can leave the previous phase visible.
- Timeout counters and `last_error` are historical and sticky; a successful
  later session does not erase them. Inspect deltas and `enabled` as well.
- `incomplete` counts selected expired ISO transfers, including failed cleanup,
  not successful delivery. The earlier patch retried them in place; this patch
  discards them and wakes the next write. The MIC2 layout is unchanged.

Counters persist across USB resets/Alt changes, wrap modulo 2^32, and restart
with the MCU. `queued - completed` is not the number of currently outstanding
packets. XFRC means ISR-observed hardware completion, not nonempty PCM received
by the host: compare usbmon ISO `actual_length` and the recording waveform.

## Read-only EP0 protocol

The firmware handler uses vendor/device IN (`bmRequestType=0xc0`, request `0x5a`,
value `0x4d49`). Words below are little-endian u32. No request clears counters,
acknowledges interrupt bits or reads the FIFO. The HAL bridge queries the
allocated IN endpoint, not a hardcoded EP1. EP0/out/unallocated queries fail.

- Index 0, length 32: existing `MIC1` response, unchanged.
- Index 1, length 112: `MIC2`, within DXXK's 128-byte control buffer.
- Wrong lengths/unknown index: STALL. Other vendor/class requests remain routed
  to their original handlers.

| Word | MIC2 field |
|---|---|
| 0 | ASCII `MIC2` (MIC1 uses `MIC1`) |
| 1–7 | endpoint, queued, XFRC, reset notifications, Alt 0 notifications, Alt 1 notifications, last Alt |
| 8–13 | write started, cancelled, generation, phase, in_enabled (0/1), incomplete-ISO events |
| 14–17 | NAK timeouts, disable timeouts, flush timeouts, last error (0 none / 1 NAK / 2 disable / 3 flush) |
| 18–21 | DIEPCTL, DIEPTSIZ, DTXFSTS, DIEPINT for the queried endpoint |
| 22–27 | DIEPMSK, DAINTMSK, DIEPEMPMSK, GINTSTS, GINTMSK, DSTS |

`scripts/microphone_usb_stats.py` requests MIC2, falling back to MIC1 only on
EPIPE/STALL. Timeout/disconnect/I/O errors are not silently treated as old firmware.
Register samples and counters are serialized against the software IRQ/write
path, but hardware can progress between register reads: this is not an atomic
hardware snapshot. DSTS/SOF progress does not prove host IN polling.

## Regression checks (from repository root)

```sh
cargo +esp test --manifest-path vendor/embassy-usb-synopsys-otg/Cargo.toml --locked --offline --lib
cargo +esp test --manifest-path dick_mouse/tests/host_audio/Cargo.toml --locked --offline
python3 scripts/microphone_usb_stats.py --self-test
```

Use the same `+esp` toolchain as the firmware. The installed stable 1.94 cannot
compile the pinned xarxa dependency's `cfg_select!` use. The separate driver
test lockfile does not participate in firmware dependency resolution.

The driver tests poll real endpoint futures/IRQ and teardown functions with
RAM-backed registers. They cover stale write cancellation (both waits and
generation wrap), queue/XFRC separation, read-only snapshots, repeated Alt 1
stop-before-flush, timeout failure, power/deinit cancellation, reset/ISO rearm
guards and EP0 configuration/response with the mic excluded from the ready mask.
The 22:18 wait_fifo regression also models NAK/EPDIS and a successful or failed
single-endpoint flush, with the microphone's actual 25-word FIFO allocation.
It checks the observed five-free-word state, 94/96/98-byte reservations, both
frame parities, waking the next write without a fake XFRC, failure propagation,
and exclusion of live/non-ISO transfers. The scoped model advances only at
handshake polls and explicitly discards TSIZ on modeled abort: it tests that
recovery no longer depends on retaining that register, not that every chip
revision behaves that way. It does **not** emulate the complete W1C/FIFO/bus
hardware or verify timing. Physical reconnect and speaker/HID tests remain
required. The diagnostic protocol and script need no update for this patch.
