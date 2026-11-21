"""
Read USBPcap captures (.pcapng from Wireshark on Windows) and turn them into G6 protocol events.

Pure Python, no tshark needed. What it understands:
- HID host->device: control SET_REPORT (bmRequestType 0x21, bRequest 0x09) to interface 4, 64-byte payload
- HID device->host: interrupt IN on endpoint 0x85, 64-byte payload
- USB Audio Class requests to the AudioControl interface: SET_CUR (0x21/0x01) and GET_CUR (0xA1/0x81)
Everything else (descriptors, isochronous audio, ...) is skipped.
"""
from __future__ import annotations

import struct
from collections.abc import Iterable, Iterator
from dataclasses import dataclass, field
from pathlib import Path
from typing import Optional

LINKTYPE_USBPCAP = 249

# USBPcap transfer types
XFER_ISO, XFER_INTERRUPT, XFER_CONTROL, XFER_BULK = 0, 1, 2, 3

HID_INTERFACE = 4
HID_IN_ENDPOINT = 0x85

# Packet prefixes the spec (g6_spec/) uses to *change* settings. The "commit" packets of those commands
# (5a XX 01 01 ...) double as read queries and are not listed. Used to refuse replaying writes.
KNOWN_WRITE_PREFIXES: tuple[bytes, ...] = tuple(bytes.fromhex(h) for h in (
    '5a1207',  # SBX / recording / decoder data
    '5a2c05',  # output speakers/headphones
    '5a3903',  # direct mode, SPDIF direct, volume ring
    '5a3a0206',  # lighting on/off
    '5a3a06',  # lighting pre-set
    '5a3a09',  # RGB colour
    '5a3c04',  # mic boost
    '5a6c03',  # DAC filter
))


def is_known_write(payload: bytes) -> bool:
    return any(payload.startswith(p) for p in KNOWN_WRITE_PREFIXES)


def trim(payload: bytes) -> bytes:
    """Drop the zero padding of a 64-byte HID report (keeps at least 1 byte)."""
    return payload.rstrip(b'\0') or payload[:1]


@dataclass
class UsbPacket:
    index: int
    ts: float  # seconds
    irp_id: int
    device: int
    endpoint: int
    transfer: int
    from_device: bool
    data: bytes  # for control setup stage: 8 setup bytes + data
    stage: Optional[int] = None  # control transfers only


@dataclass
class Event:
    """A decoded G6 protocol event."""
    index: int
    ts: float
    kind: str  # 'hid_out' | 'hid_in' | 'uac_set_cur' | 'uac_get_cur'
    device: int
    payload: bytes
    interface: Optional[int] = None
    w_value: Optional[int] = None
    w_index: Optional[int] = None
    response: Optional[bytes] = field(default=None, repr=False)  # hid_out: first hid_in after it

    @property
    def is_write(self) -> bool:
        return self.kind == 'hid_out' and is_known_write(self.payload)

    def describe(self) -> str:
        if self.kind in ('hid_out', 'hid_in'):
            arrow = '->' if self.kind == 'hid_out' else '<-'
            flag = '  [WRITE]' if self.is_write else ''
            return f'{self.ts:10.4f}  HID {arrow} {trim(self.payload).hex()}{flag}'
        cs, ch = (self.w_value or 0) >> 8, (self.w_value or 0) & 0xFF
        entity, iface = (self.w_index or 0) >> 8, (self.w_index or 0) & 0xFF
        name = 'SET_CUR' if self.kind == 'uac_set_cur' else 'GET_CUR'
        return (f'{self.ts:10.4f}  UAC {name} iface={iface} entity=0x{entity:02x} selector=0x{cs:02x} '
                f'channel={ch} data={self.payload.hex()}')


# --- pcapng -------------------------------------------------------------------------------------------------------

def read_packets(path: str | Path) -> Iterator[UsbPacket]:
    data = Path(path).read_bytes()
    if len(data) < 12 or struct.unpack_from('<I', data, 0)[0] != 0x0A0D0D0A:
        raise ValueError(f'{path}: not a pcapng file')
    endian = '<' if struct.unpack_from('<I', data, 8)[0] == 0x1A2B3C4D else '>'
    linktypes: list[int] = []
    tsres: list[float] = []
    offset, index = 0, 0
    while offset + 12 <= len(data):
        btype, blen = struct.unpack_from(endian + 'II', data, offset)
        if blen < 12 or offset + blen > len(data):
            break
        body = data[offset + 8:offset + blen - 4]
        if btype == 0x0A0D0D0A:  # section header: interface ids restart
            linktypes, tsres = [], []
        elif btype == 1:  # interface description
            linktypes.append(struct.unpack_from(endian + 'H', body, 0)[0])
            tsres.append(_ts_resolution(body[8:], endian))
        elif btype == 6:  # enhanced packet
            iface, ts_hi, ts_lo, cap_len, _orig = struct.unpack_from(endian + 'IIIII', body, 0)
            if iface < len(linktypes) and linktypes[iface] == LINKTYPE_USBPCAP:
                pkt = _parse_usbpcap(body[20:20 + cap_len], index, ((ts_hi << 32) | ts_lo) * tsres[iface])
                if pkt:
                    yield pkt
            index += 1
        offset += blen


def _ts_resolution(options: bytes, endian: str) -> float:
    i = 0
    while i + 4 <= len(options):
        code, length = struct.unpack_from(endian + 'HH', options, i)
        if code == 0:
            break
        if code == 9 and length >= 1:  # if_tsresol
            v = options[i + 4]
            return 2.0 ** -(v & 0x7F) if v & 0x80 else 10.0 ** -v
        i += 4 + ((length + 3) & ~3)
    return 1e-6


def _parse_usbpcap(p: bytes, index: int, ts: float) -> Optional[UsbPacket]:
    if len(p) < 27:
        return None
    hlen, irp, _status, _fn, info, _bus, dev, ep, xfer, dlen = struct.unpack_from('<HQIHBHHBBI', p, 0)
    stage = p[27] if xfer == XFER_CONTROL and hlen >= 28 else None
    return UsbPacket(index=index, ts=ts, irp_id=irp, device=dev, endpoint=ep, transfer=xfer,
                     from_device=bool(info & 1), data=p[hlen:hlen + dlen], stage=stage)


# --- decoding -----------------------------------------------------------------------------------------------------

def decode(packets: Iterable[UsbPacket], device: Optional[int] = None) -> list[Event]:
    """Turn raw USB packets into protocol events. ``device`` filters by USB address (default: auto-detect)."""
    packets = list(packets)
    if device is None:
        device = guess_g6_device(packets)
    events: list[Event] = []
    pending_get: dict[int, Event] = {}  # irp id -> GET_CUR request waiting for its data
    for pkt in packets:
        if device is not None and pkt.device != device:
            continue
        if pkt.transfer == XFER_CONTROL and not pkt.from_device and len(pkt.data) >= 8:
            bm, breq, wv, wi, wl = struct.unpack_from('<BBHHH', pkt.data, 0)
            body = pkt.data[8:8 + wl]
            if bm == 0x21 and breq == 0x09 and (wi & 0xFF) == HID_INTERFACE:
                events.append(Event(pkt.index, pkt.ts, 'hid_out', pkt.device, body, HID_INTERFACE, wv, wi))
            elif bm == 0x21 and breq == 0x01:
                events.append(Event(pkt.index, pkt.ts, 'uac_set_cur', pkt.device, body, wi & 0xFF, wv, wi))
            elif bm == 0xA1 and breq == 0x81:
                ev = Event(pkt.index, pkt.ts, 'uac_get_cur', pkt.device, b'', wi & 0xFF, wv, wi)
                pending_get[pkt.irp_id] = ev
                events.append(ev)
        elif pkt.transfer == XFER_CONTROL and pkt.from_device and pkt.irp_id in pending_get and pkt.data:
            pending_get.pop(pkt.irp_id).payload = pkt.data
        elif (pkt.transfer == XFER_INTERRUPT and pkt.from_device and pkt.endpoint == HID_IN_ENDPOINT
              and pkt.data):
            events.append(Event(pkt.index, pkt.ts, 'hid_in', pkt.device, pkt.data, HID_INTERFACE))

    # timestamps relative to the first event
    if events:
        t0 = events[0].ts
        for ev in events:
            ev.ts -= t0

    # pair each HID OUT with the first HID IN that follows it
    last_out: Optional[Event] = None
    for ev in events:
        if ev.kind == 'hid_out':
            last_out = ev
        elif ev.kind == 'hid_in' and last_out is not None and last_out.response is None:
            last_out.response = ev.payload
            last_out = None
    return events


def guess_g6_device(packets: list[UsbPacket]) -> Optional[int]:
    """The USB address carrying the most 0x5a-prefixed HID reports."""
    counts: dict[int, int] = {}
    for pkt in packets:
        hid_out = pkt.transfer == XFER_CONTROL and len(pkt.data) > 8 and pkt.data[:2] == b'\x21\x09'
        hid_in = pkt.transfer == XFER_INTERRUPT and pkt.data[:1] == b'\x5a'
        if (hid_out and pkt.data[8:9] == b'\x5a') or hid_in:
            counts[pkt.device] = counts.get(pkt.device, 0) + 1
    return max(counts, key=counts.get) if counts else None


def load_events(path: str | Path, device: Optional[int] = None) -> list[Event]:
    return decode(read_packets(path), device=device)


@dataclass
class Query:
    payload: bytes  # trimmed
    count: int
    first_response: Optional[bytes]
    is_write: bool


def unique_hid_queries(events: Iterable[Event]) -> list[Query]:
    """Distinct host->device HID payloads in first-seen order."""
    seen: dict[bytes, Query] = {}
    for ev in events:
        if ev.kind != 'hid_out':
            continue
        key = trim(ev.payload)
        if key in seen:
            seen[key].count += 1
        else:
            seen[key] = Query(key, 1, trim(ev.response) if ev.response else None, ev.is_write)
    return list(seen.values())
