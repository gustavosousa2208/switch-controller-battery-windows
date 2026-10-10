"""Shared Windows/Bluetooth plumbing for Switch-mode controller battery tools.

Contents:
- Bluetooth devnode discovery (VID/PID match on BTHENUM\\Dev_* devices)
- devnode property read/write helpers (CfgMgr32)
- the canonical Windows battery property schema used by Settings
- HID helpers: controller selection and output-report (subcommand) framing

Windows battery schema (the same properties the Bluetooth stack fills for BLE
devices, and the ones Windows Settings displays for the paired controller):

    {104EA319-6EE2-4701-BD47-8DDBF425BBE5} PID 2 = Byte     battery percent
    {104EA319-6EE2-4701-BD47-8DDBF425BBE5} PID 3 = Boolean  charging
    {104EA319-6EE2-4701-BD47-8DDBF425BBE5} PID 7 = FileTime last update

Controller identity: Nintendo Switch "Pro Controller" mode (VID 057E,
PID 2009) as used by 8BitDo and other third-party controllers.
"""

from __future__ import annotations

import ctypes
import time
import uuid

import hid

# ---------------------------------------------------------------------------
# constants
# ---------------------------------------------------------------------------

CONTROLLER_VID = 0x057E
CONTROLLER_PID = 0x2009

PKEY_BATTERY_GUID = "{104EA319-6EE2-4701-BD47-8DDBF425BBE5}"
BATTERY_PID_LEVEL = 2       # DEVPROP_TYPE_BYTE, percent
BATTERY_PID_CHARGING = 3    # DEVPROP_TYPE_BOOLEAN
BATTERY_PID_TIMESTAMP = 7   # DEVPROP_TYPE_FILETIME

DEVPROP_TYPE_EMPTY = 0x00000000
DEVPROP_TYPE_BYTE = 0x00000003
DEVPROP_TYPE_UINT16 = 0x00000005
DEVPROP_TYPE_UINT32 = 0x00000007
DEVPROP_TYPE_FILETIME = 0x00000010
DEVPROP_TYPE_BOOLEAN = 0x00000011

CM_LOCATE_DEVNODE_NORMAL = 0x00000000
FILE_TIME_EPOCH_OFFSET = 11644473600  # seconds between 1601-01-01 and 1970-01-01

# Bluetooth devnode property namespace (DEVPKEY_Bluetooth_*):
# PID 7 = DeviceVID (UInt16), PID 8 = DevicePID (UInt16).
# Discovered empirically on Windows 11 (build 26200).
BT_PROPKEY_GUID = (0x2BD67D8B, 0x8BEB, 0x48D5,
                   (0x87, 0xE0, 0x6C, 0xDA, 0x34, 0x28, 0x04, 0x0A))
BT_PID_VID = 7
BT_PID_PID = 8

# Output report 0x01 (rumble + subcommand):
# [0x01, packet counter, neutral rumble (8 bytes), subcommand id @ byte 10, args]
NEUTRAL_RUMBLE = bytes([0x00, 0x01, 0x40, 0x40, 0x00, 0x01, 0x40, 0x40])
SUBCMD_SET_INPUT_REPORT_MODE = 0x03
INPUT_MODE_FULL = 0x30
INPUT_MODE_SIMPLE = 0x3F

BATTERY_REPORT_IDS = (0x30, 0x21)  # full reports and subcommand replies


# ---------------------------------------------------------------------------
# CfgMgr32 plumbing
# ---------------------------------------------------------------------------

CFGMGR32 = ctypes.WinDLL("cfgmgr32", use_last_error=True)


class GUID(ctypes.Structure):
    _fields_ = [
        ("Data1", ctypes.c_ulong),
        ("Data2", ctypes.c_ushort),
        ("Data3", ctypes.c_ushort),
        ("Data4", ctypes.c_ubyte * 8),
    ]


class DEVPROPKEY(ctypes.Structure):
    _fields_ = [("fmtid", GUID), ("pid", ctypes.c_ulong)]


def make_guid(text: str) -> GUID:
    u = uuid.UUID(text)
    return GUID(u.time_low, u.time_mid, u.time_hi_version,
                (ctypes.c_ubyte * 8)(*u.bytes[8:]))


def battery_key(prop_id: int) -> DEVPROPKEY:
    return DEVPROPKEY(make_guid(PKEY_BATTERY_GUID), prop_id)


def bt_key(prop_id: int) -> DEVPROPKEY:
    return DEVPROPKEY(GUID(BT_PROPKEY_GUID[0], BT_PROPKEY_GUID[1],
                           BT_PROPKEY_GUID[2],
                           (ctypes.c_ubyte * 8)(*BT_PROPKEY_GUID[3])), prop_id)


CM_Locate_DevNodeW = CFGMGR32.CM_Locate_DevNodeW
CM_Locate_DevNodeW.argtypes = [ctypes.POINTER(ctypes.c_ulong),
                               ctypes.c_wchar_p, ctypes.c_ulong]
CM_Locate_DevNodeW.restype = ctypes.c_ulong

CM_Get_DevNode_PropertyW = CFGMGR32.CM_Get_DevNode_PropertyW
CM_Get_DevNode_PropertyW.argtypes = [
    ctypes.c_ulong,
    ctypes.POINTER(DEVPROPKEY),
    ctypes.POINTER(ctypes.c_ulong),
    ctypes.c_void_p,
    ctypes.POINTER(ctypes.c_ulong),
    ctypes.c_ulong,
]
CM_Get_DevNode_PropertyW.restype = ctypes.c_ulong

CM_Set_DevNode_PropertyW = CFGMGR32.CM_Set_DevNode_PropertyW
CM_Set_DevNode_PropertyW.argtypes = [
    ctypes.c_ulong,
    ctypes.POINTER(DEVPROPKEY),
    ctypes.c_ulong,
    ctypes.c_void_p,
    ctypes.c_ulong,
    ctypes.c_ulong,
]
CM_Set_DevNode_PropertyW.restype = ctypes.c_ulong

CM_Get_Device_ID_List_SizeW = CFGMGR32.CM_Get_Device_ID_List_SizeW
CM_Get_Device_ID_List_SizeW.argtypes = [ctypes.POINTER(ctypes.c_ulong),
                                        ctypes.c_wchar_p, ctypes.c_ulong]
CM_Get_Device_ID_List_SizeW.restype = ctypes.c_ulong

CM_Get_Device_ID_ListW = CFGMGR32.CM_Get_Device_ID_ListW
CM_Get_Device_ID_ListW.argtypes = [ctypes.c_wchar_p, ctypes.c_wchar_p,
                                   ctypes.c_ulong, ctypes.c_ulong]
CM_Get_Device_ID_ListW.restype = ctypes.c_ulong


def locate(instance_id: str):
    """Return the DEVINST for an instance id, or None."""
    if not instance_id:
        return None
    devinst = ctypes.c_ulong(0)
    cr = CM_Locate_DevNodeW(ctypes.byref(devinst), instance_id,
                            CM_LOCATE_DEVNODE_NORMAL)
    return devinst.value if cr == 0 else None


def read_prop(devinst: int, key: DEVPROPKEY):
    """Return (cr, ptype, size, data) for a devnode property."""
    ptype = ctypes.c_ulong(0)
    buf = (ctypes.c_ubyte * 64)()
    size = ctypes.c_ulong(64)
    cr = CM_Get_DevNode_PropertyW(devinst, ctypes.byref(key),
                                  ctypes.byref(ptype), buf,
                                  ctypes.byref(size), 0)
    data = None
    if cr == 0:
        if ptype.value == DEVPROP_TYPE_BYTE and size.value >= 1:
            data = buf[0]
        elif ptype.value == DEVPROP_TYPE_BOOLEAN and size.value >= 1:
            data = bool(buf[0])
        elif ptype.value == DEVPROP_TYPE_UINT16 and size.value >= 2:
            data = int.from_bytes(bytes(buf[:2]), "little")
        elif ptype.value == DEVPROP_TYPE_UINT32 and size.value >= 4:
            data = int.from_bytes(bytes(buf[:4]), "little")
        else:
            data = bytes(buf[:size.value])
    return cr, ptype.value, size.value, data


def set_prop(devinst: int, key: DEVPROPKEY, prop_type: int, value: bytes) -> int:
    buf = (ctypes.c_ubyte * len(value))(*value)
    return CM_Set_DevNode_PropertyW(devinst, ctypes.byref(key), prop_type,
                                    ctypes.cast(buf, ctypes.c_void_p),
                                    len(value), 0)


def delete_prop(devinst: int, key: DEVPROPKEY) -> int:
    return CM_Set_DevNode_PropertyW(devinst, ctypes.byref(key),
                                    DEVPROP_TYPE_EMPTY, None, 0, 0)


def _device_id_list(filter_text: str) -> list:
    size = ctypes.c_ulong(0)
    if CM_Get_Device_ID_List_SizeW(ctypes.byref(size), filter_text, 0) != 0:
        return []
    buf = ctypes.create_unicode_buffer(size.value)
    if CM_Get_Device_ID_ListW(filter_text, buf, size.value, 0) != 0:
        return []
    items = []
    i = 0
    while i < len(buf) and buf[i] != "\0":
        j = i
        while buf[j] != "\0":
            j += 1
        items.append(buf[i:j])
        i = j + 1
    return items


def find_controller_instances() -> list:
    """Return BTHENUM\\Dev_* instance ids whose VID/PID match the controller."""
    matches = []
    for instance in _device_id_list("BTHENUM"):
        if not instance.upper().startswith("BTHENUM\\DEV_"):
            continue
        devinst = locate(instance)
        if devinst is None:
            continue
        _, _, _, vid = read_prop(devinst, bt_key(BT_PID_VID))
        _, _, _, pid = read_prop(devinst, bt_key(BT_PID_PID))
        if vid == CONTROLLER_VID and pid == CONTROLLER_PID:
            matches.append(instance)
    return matches


# ---------------------------------------------------------------------------
# Battery property helpers
# ---------------------------------------------------------------------------

def write_battery(instance_id: str, percent: int, charging: bool):
    """Write the full battery schema. Returns (devinst, crs) or (None, None)."""
    devinst = locate(instance_id)
    if devinst is None:
        return None, None
    now = int((time.time() + FILE_TIME_EPOCH_OFFSET) * 10_000_000)
    crs = (
        set_prop(devinst, battery_key(BATTERY_PID_LEVEL),
                 DEVPROP_TYPE_BYTE, bytes([max(0, min(255, percent))])),
        set_prop(devinst, battery_key(BATTERY_PID_CHARGING),
                 DEVPROP_TYPE_BOOLEAN, bytes([1 if charging else 0])),
        set_prop(devinst, battery_key(BATTERY_PID_TIMESTAMP),
                 DEVPROP_TYPE_FILETIME, now.to_bytes(8, "little")),
    )
    return devinst, crs


def clear_battery(instance_id: str) -> bool:
    """Delete the battery properties. Returns True when the devnode existed."""
    devinst = locate(instance_id)
    if devinst is None:
        return False
    for prop_id in (BATTERY_PID_LEVEL, BATTERY_PID_CHARGING,
                    BATTERY_PID_TIMESTAMP):
        delete_prop(devinst, battery_key(prop_id))
    return True


# ---------------------------------------------------------------------------
# HID side
# ---------------------------------------------------------------------------

def path_text(device) -> str:
    path = device.get("path")
    if isinstance(path, bytes):
        return path.decode("utf-8", errors="replace")
    return str(path)


def is_controller(device) -> bool:
    text = (f"{device.get('manufacturer_string', '')} "
            f"{device.get('product_string', '')}").lower()
    return ((device.get("vendor_id") == CONTROLLER_VID
             and device.get("product_id") == CONTROLLER_PID)
            or "8bitdo" in text
            or "pro controller" in text
            or "wireless gamepad" in text)


def candidate_rank(device) -> int:
    path = path_text(device).lower()
    if "00001124" in path or "bth" in path:
        return 0  # Bluetooth HID service interface
    if "usb#" in path:
        return 1  # USB interface
    return 2


def open_controller(probe_seconds: float = 10.0):
    """Open a controller HID interface that delivers reports.

    The interface is accepted as soon as any report arrives; full (0x30)
    reports only flow after request_full_mode() (or a host negotiates them).
    """
    candidates = [d for d in hid.enumerate() if is_controller(d)]
    candidates.sort(key=candidate_rank)
    for device in candidates:
        print(f"candidate: {device.get('product_string')!r} "
              f"VID={device.get('vendor_id'):#06x} "
              f"PID={device.get('product_id'):#06x} "
              f"path={path_text(device)!r}")
        dev = None
        try:
            dev = hid.device()
            dev.open_path(device["path"])
            dev.set_nonblocking(1)
            deadline = time.monotonic() + probe_seconds
            while time.monotonic() < deadline:
                report = dev.read(128)
                if report and len(report) >= 1:
                    print(f"opened: {device.get('product_string')} "
                          f"(first report id 0x{report[0]:02x})")
                    return dev
                time.sleep(0.02)
            print("  no reports in %.0f s; trying next interface"
                  % probe_seconds)
        except OSError as exc:
            print(f"  open failed: {exc}")
        if dev is not None:
            try:
                dev.close()
            except Exception:
                pass
    return None


def make_subcommand_packet(counter: int, subcmd: int, args: bytes = b"") -> bytes:
    """Output report 0x01: [0x01, counter, rumble(8), subcmd, args...]."""
    packet = bytearray(49)
    packet[0] = 0x01
    packet[1] = counter & 0x0F
    packet[2:10] = NEUTRAL_RUMBLE
    packet[10] = subcmd
    packet[11:11 + len(args)] = args
    return bytes(packet)


def request_full_mode(dev, counter: int) -> bool:
    """Ask the controller to switch to full reports (subcommand 0x03/0x30).

    Returns True when the stack accepted the 49-byte write; the controller
    confirms with a 0x21 ACK and starts streaming 0x30 reports.
    """
    try:
        written = dev.write(make_subcommand_packet(
            counter, SUBCMD_SET_INPUT_REPORT_MODE, bytes([INPUT_MODE_FULL])))
    except OSError:
        return False
    return written == 49


def level_to_percent(level: int) -> int:
    """Linear mapping of the Nintendo level code 0..8 to 0..100 percent."""
    return max(0, min(100, round(level * 100 / 8)))


class Tee:
    """Mirror writes to multiple streams (console + log file)."""

    def __init__(self, *streams):
        self._streams = [s for s in streams if s is not None]

    def write(self, data):
        for stream in self._streams:
            try:
                stream.write(data)
            except Exception:
                pass

    def flush(self):
        for stream in self._streams:
            try:
                stream.flush()
            except Exception:
                pass

    def isatty(self):
        return False
