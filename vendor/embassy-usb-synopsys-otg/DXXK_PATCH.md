# Local diagnostic patch

Imported unchanged from embassy-rs/embassy commit
`cd7570483a7036f23a9925a339152396bec4c041`, then instrumented:

- `EndpointIn::write`: increment the endpoint's queued count after FIFO submission,
  inside the existing critical section.
- IN interrupt handler: increment completed only for `DIEPINT.XFRC`, immediately
  before the existing write-one-to-clear acknowledgement.
- `State::in_transfer_counts`: read a consistent pair using the existing mutex.

No change to FIFO, interrupt, rearm, abort or alternate-setting behavior. Counters
persist across USB resets and Alt changes, wrap modulo 2^32, and restart with the
MCU. EP0 is excluded from the public query because its queue path is different.
Hardware XFRC does not prove host delivery of nonzero-length PCM: compare usbmon
ISO `actual_length` as well. Cargo sibling paths point to the same pinned revision.
