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
    if len(data) != 32 or data[:4] != b"MIC1":
        raise ValueError("Invalid diagnostic response; flash the diagnostic firmware")
    return struct.unpack("<7I", data[4:])


def read_snapshot(path):
    data = ctypes.create_string_buffer(32)
    transfer = ControlTransfer(0xc0, 0x5a, 0x4d49, 0, 32, 1000, ctypes.addressof(data))
    # Linux USBDEVFS_CONTROL = _IOWR('U', 0, struct usbdevfs_ctrltransfer).
    request = 0xc0005500 | (ctypes.sizeof(ControlTransfer) << 16)
    fd = os.open(path, os.O_RDWR | os.O_CLOEXEC)
    try:
        size = fcntl.ioctl(fd, request, transfer)
    finally:
        os.close(fd)
    return decode(data.raw[:size])


def deltas(now, previous):
    if previous is None:
        return None
    values = tuple((a - b) & 0xffffffff for a, b in zip(now, previous))
    # A backward counter jump is an MCU restart, not billions of new packets.
    return None if any(n >= 0x80000000 for n in values) else values


def self_test():
    sample = b"MIC1" + struct.pack("<7I", 0x81, 5001, 4999, 2, 3, 4, 1)
    assert decode(sample) == (0x81, 5001, 4999, 2, 3, 4, 1)
    assert deltas((5001, 4999), (4001, 3999)) == (1000, 1000)
    assert deltas((5001, 4999), (5001, 4999)) == (0, 0)
    assert deltas((5, 4), (0xfffffffe, 0xfffffffd)) == (7, 7)
    assert deltas((0, 0), (5001, 4999)) is None
    for bad in (b"", sample[:31], b"BAD!" + sample[4:]):
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
            ep, queued, xfrc, resets, alt0, alt1, alt = read_snapshot(path)
            current_identity = (path, ep, resets)
            if current_identity != identity:
                previous = None
            now = time.monotonic()
            delta = deltas((queued, xfrc), previous[:2] if previous else None)
            rate = "baseline" if delta is None else (
                f"dqueued={delta[0]} dxfrc={delta[1]} dt={now - previous[2]:.3f}s")
            print(f"{time.strftime('%H:%M:%S')} {path.name} ep=0x{ep:02x} alt={alt} "
                  f"queued={queued} xfrc={xfrc} {rate} "
                  f"resets={resets} alt0={alt0} alt1={alt1}", flush=True)
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
