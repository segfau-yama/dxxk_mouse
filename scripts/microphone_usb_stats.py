#!/usr/bin/env python3
"""Read DXXK's counters via Linux usbfs; no pip package or driver detach needed."""
import argparse
import ctypes
import errno
import fcntl
import os
from pathlib import Path
import struct
import time


class ControlTransfer(ctypes.Structure):
    _fields_ = [("request_type", ctypes.c_uint8), ("request", ctypes.c_uint8),
                ("value", ctypes.c_uint16), ("index", ctypes.c_uint16),
                ("length", ctypes.c_uint16), ("timeout", ctypes.c_uint32),
                ("data", ctypes.c_void_p)]


def find_device(bus=None, address=None):
    matches = []
    for device in Path("/sys/bus/usb/devices").iterdir():
        try:
            if (device / "idVendor").read_text().strip() != "c0de":
                continue
            if (device / "idProduct").read_text().strip() != "0001":
                continue
            b = int((device / "busnum").read_text())
            a = int((device / "devnum").read_text())
        except (FileNotFoundError, NotADirectoryError):
            continue
        if (bus is None or b == bus) and (address is None or a == address):
            matches.append(Path(f"/dev/bus/usb/{b:03d}/{a:03d}"))
    if not matches:
        raise FileNotFoundError("DXXK c0de:0001 is not connected")
    if len(matches) != 1:
        raise RuntimeError("Multiple DXXK devices: select --bus and --address")
    return matches[0]


def decode(data):
    if (data[:4], len(data)) not in ((b"MIC1", 32), (b"MIC2", 112)):
        raise ValueError("Invalid diagnostic response; flash the diagnostic firmware")
    return struct.unpack(f"<{len(data) // 4 - 1}I", data[4:])


def read_control(path, index, length):
    data = ctypes.create_string_buffer(length)
    transfer = ControlTransfer(0xc0, 0x5a, 0x4d49, index, length, 1000, ctypes.addressof(data))
    # Linux USBDEVFS_CONTROL = _IOWR('U', 0, struct usbdevfs_ctrltransfer).
    request = 0xc0005500 | (ctypes.sizeof(ControlTransfer) << 16)
    fd = os.open(path, os.O_RDWR | os.O_CLOEXEC)
    try:
        size = fcntl.ioctl(fd, request, transfer)
    finally:
        os.close(fd)
    result = decode(data.raw[:size])
    if size != length:
        raise ValueError(f"Short diagnostic response: {size}, expected {length}")
    return result


def read_snapshot(path):
    try:
        return read_control(path, 1, 112)
    except OSError as error:
        if error.errno != errno.EPIPE:
            raise
        # Old firmware does not implement index 1. Only STALL means fallback;
        # timeout, disconnect and permission failures must remain visible.
        return read_control(path, 0, 32)


def format_details(words):
    if not words:
        return " legacy=MIC1"
    (started, cancelled, generation, phase, enabled, incomplete, nak_timeout,
     disable_timeout, flush_timeout, last_error, *registers) = words
    phase = {0: "idle", 1: "wait_enabled", 2: "wait_previous", 3: "wait_fifo"}.get(phase, str(phase))
    error = {0: "none", 1: "nak_timeout", 2: "disable_timeout", 3: "flush_timeout"}.get(last_error, str(last_error))
    names = ("DIEPCTL", "DIEPTSIZ", "DTXFSTS", "DIEPINT", "DIEPMSK",
             "DAINTMSK", "DIEPEMPMSK", "GINTSTS", "GINTMSK", "DSTS")
    raw = " ".join(f"{name}=0x{value:08x}" for name, value in zip(names, registers))
    ctl, tsiz, fifo, interrupt = registers[:4]
    return (f" started={started} cancelled={cancelled} gen={generation} phase={phase}"
            f" enabled={enabled} incomplete={incomplete} timeouts={nak_timeout}/{disable_timeout}/{flush_timeout}"
            f" last_error={error} USBAEP={(ctl >> 15) & 1} EPENA={(ctl >> 31) & 1}"
            f" NAKSTS={(ctl >> 17) & 1} remaining={tsiz & 0x7ffff} fifo_words={fifo & 0xffff}"
            f" XFRC={interrupt & 1} {raw}")


def deltas(now, previous):
    if previous is None:
        return None
    values = tuple((a - b) & 0xffffffff for a, b in zip(now, previous))
    # A backward counter jump is an MCU restart, not billions of new packets.
    return None if any(n >= 0x80000000 for n in values) else values


def self_test():
    sample = b"MIC1" + struct.pack("<7I", 0x81, 5001, 4999, 2, 3, 4, 1)
    assert decode(sample) == (0x81, 5001, 4999, 2, 3, 4, 1)
    detail = (5002, 3, 12, 2, 1, 4, 0, 1, 0, 2,
              0x80028000, 96, 32, 1, 0, 2, 0, 0, 0, 0)
    extended = b"MIC2" + sample[4:] + struct.pack("<20I", *detail)
    assert decode(extended)[:7] == decode(sample)
    assert decode(extended)[7:] == detail
    rendered = format_details(detail)
    for text in ("phase=wait_previous", "EPENA=1", "USBAEP=1", "NAKSTS=1",
                 "remaining=96", "fifo_words=32", "XFRC=1", "last_error=disable_timeout"):
        assert text in rendered
    assert "legacy=MIC1" in format_details(())
    from unittest.mock import patch
    with patch(__name__ + ".read_control", side_effect=[OSError(errno.EPIPE, "STALL"), decode(sample)]) as reader:
        assert read_snapshot(Path("/fake")) == decode(sample)
        assert [call.args[1:] for call in reader.call_args_list] == [(1, 112), (0, 32)]
    with patch(__name__ + ".read_control", return_value=decode(extended)) as reader:
        assert read_snapshot(Path("/fake")) == decode(extended)
        assert reader.call_count == 1
    with patch(__name__ + ".read_control", side_effect=OSError(errno.EIO, "I/O")) as reader:
        try:
            read_snapshot(Path("/fake"))
        except OSError as error:
            assert error.errno == errno.EIO
        else:
            raise AssertionError("I/O error hidden by legacy fallback")
        assert reader.call_count == 1
    assert deltas((5001, 4999), (4001, 3999)) == (1000, 1000)
    assert deltas((5001, 4999), (5001, 4999)) == (0, 0)
    assert deltas((5, 4), (0xfffffffe, 0xfffffffd)) == (7, 7)
    assert deltas((0, 0), (5001, 4999)) is None
    # Exercise the real polling loop: a new address OR reset notification must
    # start a fresh baseline, even when cumulative packet counts survive.
    from contextlib import redirect_stdout
    from io import StringIO
    unchanged = decode(sample)
    reset = (*unchanged[:3], unchanged[3] + 1, *unchanged[4:])
    paths = [Path("/fake/010"), Path("/fake/011"), Path("/fake/011"), Path("/fake/011")]
    output = StringIO()
    with (patch("sys.argv", ["microphone_usb_stats.py", "--count", "4"]),
          patch(__name__ + ".find_device", side_effect=paths),
          patch(__name__ + ".read_snapshot", side_effect=[unchanged, unchanged, reset, reset]),
          patch(__name__ + ".time.sleep"), redirect_stdout(output)):
        main()
    assert output.getvalue().count("baseline") == 3
    assert output.getvalue().count("dqueued=0 dxfrc=0") == 1
    for bad in (b"", sample[:31], b"BAD!" + sample[4:], extended[:111],
                b"MIC1" + extended[4:], b"MIC2" + sample[4:]):
        try:
            decode(bad)
        except ValueError:
            pass
        else:
            raise AssertionError("invalid response accepted")
    assert ControlTransfer.data.offset >= 12
    print("microphone USB statistics self-test: OK")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bus", type=int)
    parser.add_argument("--address", type=int, help="optional; changes on reconnect")
    parser.add_argument("--count", type=int, default=0, help="poll attempts (0: until Ctrl+C)")
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        self_test()
        return
    if args.count < 0:
        parser.error("--count must be >= 0")
    previous = None
    identity = None
    attempt = 0
    while args.count == 0 or attempt < args.count:
        attempt += 1
        try:
            path = find_device(args.bus, args.address)
            snapshot = read_snapshot(path)
            ep, queued, xfrc, resets, alt0, alt1, alt = snapshot[:7]
            current_identity = (path, ep, resets)
            if current_identity != identity:
                previous = None
            now = time.monotonic()
            delta = deltas((queued, xfrc), previous[:2] if previous else None)
            rate = "baseline" if delta is None else (
                f"dqueued={delta[0]} dxfrc={delta[1]} dt={now - previous[2]:.3f}s")
            print(f"{time.strftime('%H:%M:%S')} {path.name} ep=0x{ep:02x} alt={alt} "
                  f"queued={queued} xfrc={xfrc} {rate} "
                  f"resets={resets} alt0={alt0} alt1={alt1}"
                  f"{format_details(snapshot[7:])}", flush=True)
            previous = (queued, xfrc, now)
            identity = current_identity
        except PermissionError:
            raise SystemExit("USB access denied: run this script with sudo (no driver detach).")
        except OSError as error:
            if error.errno == errno.EPIPE:
                raise SystemExit("EP0 STALL: confirm that the diagnostic firmware was built and flashed.")
            print(f"{time.strftime('%H:%M:%S')} unavailable: {error}", flush=True)
            previous = None
            identity = None
        except (ValueError, RuntimeError) as error:
            raise SystemExit(str(error))
        if args.count == 0 or attempt < args.count:
            time.sleep(1)


if __name__ == "__main__":
    try:
        main()
    except KeyboardInterrupt:
        pass
