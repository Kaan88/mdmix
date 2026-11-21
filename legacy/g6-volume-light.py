#!/usr/bin/env python3
"""
g6-volume-light.py
Toggle the volume ring light on the SoundBlaster X G6.

Reverse-engineered from USBPcap captures.
Command 0x0e controls the volume ring LED:
  byte[5] = 0x00  → enabled
  byte[5] = 0x01  → disabled

Usage:
  python3 g6-volume-light.py on
  python3 g6-volume-light.py off
"""

import sys

VENDOR_ID  = 0x041E
PRODUCT_ID = 0x3256
INTERFACE  = 4  # HID control interface on G6


def build_data_packet(enabled: bool) -> bytes:
    buf = bytearray(64)
    buf[0] = 0x5A  # magic
    buf[1] = 0x39  # magic
    buf[2] = 0x03  # DATA packet type
    buf[3] = 0x00
    buf[4] = 0x0E  # command: volume ring light
    buf[5] = 0x00 if enabled else 0x01
    return bytes(buf)


def build_commit_packet() -> bytes:
    buf = bytearray(64)
    buf[0] = 0x5A
    buf[1] = 0x39
    buf[2] = 0x01  # COMMIT packet type
    buf[3] = 0x01
    return bytes(buf)


def send(enabled: bool, dry_run: bool = False) -> None:
    data_pkt   = build_data_packet(enabled)
    commit_pkt = build_commit_packet()

    print(f"DATA  : {data_pkt.hex()}")
    print(f"COMMIT: {commit_pkt.hex()}")

    if dry_run:
        print("[dry-run] Not sending.")
        return

    try:
        import hid
    except ImportError:
        sys.exit("Error: 'hid' module not found. Install with: pip install hid")

    # Find the correct interface (interface 4)
    device_info = None
    for info in hid.enumerate(VENDOR_ID, PRODUCT_ID):
        if info.get("interface_number") == INTERFACE:
            device_info = info
            break

    if device_info is None:
        # Fall back: open first matching device
        print(f"Warning: interface {INTERFACE} not found, trying default open.")
        dev = hid.device()
        dev.open(VENDOR_ID, PRODUCT_ID)
    else:
        dev = hid.device()
        dev.open_path(device_info["path"])

    try:
        # hidapi write() prepends a 0x00 report ID byte automatically
        dev.write(b'\x00' + data_pkt)
        dev.write(b'\x00' + commit_pkt)
        print(f"Volume ring light {'enabled' if enabled else 'disabled'}.")
    finally:
        dev.close()


def main():
    args = sys.argv[1:]
    dry_run = "--dry-run" in args
    args = [a for a in args if a != "--dry-run"]

    if not args or args[0] not in ("on", "off"):
        print("Usage: g6-volume-light.py [--dry-run] on|off")
        sys.exit(1)

    send(enabled=(args[0] == "on"), dry_run=dry_run)


if __name__ == "__main__":
    main()
