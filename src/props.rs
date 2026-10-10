//! CFGMGR32 plumbing: controller devnode discovery and battery properties.

use std::time::SystemTime;

use windows_sys::core::GUID;
use windows_sys::Win32::Devices::DeviceAndDriverInstallation::{
    CM_Get_DevNode_PropertyW, CM_Get_Device_ID_ListW, CM_Get_Device_ID_List_SizeW,
    CM_Locate_DevNodeW, CM_Set_DevNode_PropertyW,
};

use crate::util::{filetime_now, filetime_to_systemtime};

pub const CONTROLLER_VID: u16 = 0x057E;
pub const CONTROLLER_PID: u16 = 0x2009;

pub const BATTERY_PID_LEVEL: u32 = 2;
pub const BATTERY_PID_CHARGING: u32 = 3;
pub const BATTERY_PID_TIMESTAMP: u32 = 7;

const BATTERY_GUID: GUID = GUID {
    data1: 0x104E_A319,
    data2: 0x6EE2,
    data3: 0x4701,
    data4: [0xBD, 0x47, 0x8D, 0xDB, 0xF4, 0x25, 0xBB, 0xE5],
};
const BT_GUID: GUID = GUID {
    data1: 0x2BD6_7D8B,
    data2: 0x8BEB,
    data3: 0x48D5,
    data4: [0x87, 0xE0, 0x6C, 0xDA, 0x34, 0x28, 0x04, 0x0A],
};

const BT_PID_VID: u32 = 7;
const BT_PID_PID: u32 = 8;

const DEVPROP_TYPE_EMPTY: u32 = 0x00;
const DEVPROP_TYPE_BYTE: u32 = 0x03;
const DEVPROP_TYPE_UINT16: u32 = 0x05;
const DEVPROP_TYPE_FILETIME: u32 = 0x10;
const DEVPROP_TYPE_BOOLEAN: u32 = 0x11;

#[repr(C)]
#[derive(Clone, Copy)]
struct DevPropKey {
    fmtid: GUID,
    pid: u32,
}

fn battery_key(pid: u32) -> DevPropKey {
    DevPropKey {
        fmtid: BATTERY_GUID,
        pid,
    }
}

fn bt_key(pid: u32) -> DevPropKey {
    DevPropKey {
        fmtid: BT_GUID,
        pid,
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn locate(instance_id: &str) -> Option<u32> {
    let mut devinst: u32 = 0;
    let id = wide(instance_id);
    let cr = unsafe { CM_Locate_DevNodeW(&mut devinst, id.as_ptr(), 0) };
    if cr == 0 {
        Some(devinst)
    } else {
        None
    }
}

fn read_raw(devinst: u32, key: &DevPropKey) -> Option<(u32, Vec<u8>)> {
    let mut ptype: u32 = 0;
    let mut buf = [0u8; 64];
    let mut size: u32 = buf.len() as u32;
    let cr = unsafe {
        CM_Get_DevNode_PropertyW(
            devinst,
            key as *const DevPropKey as *const _,
            &mut ptype,
            buf.as_mut_ptr(),
            &mut size,
            0,
        )
    };
    if cr == 0 {
        Some((ptype, buf[..size as usize].to_vec()))
    } else {
        None
    }
}

fn read_bt_u16(devinst: u32, pid: u32) -> Option<u16> {
    let (ptype, data) = read_raw(devinst, &bt_key(pid))?;
    if ptype == DEVPROP_TYPE_UINT16 && data.len() >= 2 {
        Some(u16::from_le_bytes([data[0], data[1]]))
    } else {
        None
    }
}

fn device_id_list(filter: &str) -> Vec<String> {
    let filter_w = wide(filter);
    let mut size: u32 = 0;
    if unsafe { CM_Get_Device_ID_List_SizeW(&mut size, filter_w.as_ptr(), 0) } != 0 || size == 0 {
        return Vec::new();
    }
    let mut buf = vec![0u16; size as usize];
    if unsafe { CM_Get_Device_ID_ListW(filter_w.as_ptr(), buf.as_mut_ptr(), size, 0) } != 0 {
        return Vec::new();
    }
    let mut items = Vec::new();
    let mut start = 0usize;
    while start < buf.len() && buf[start] != 0 {
        let mut end = start;
        while end < buf.len() && buf[end] != 0 {
            end += 1;
        }
        items.push(String::from_utf16_lossy(&buf[start..end]));
        start = end + 1;
    }
    items
}

/// `BTHENUM\Dev_*` instance ids whose Bluetooth VID/PID match the controller.
pub fn find_controller_instances() -> Vec<String> {
    let mut found = Vec::new();
    for instance in device_id_list("BTHENUM") {
        if !instance.to_uppercase().starts_with("BTHENUM\\DEV_") {
            continue;
        }
        let Some(devinst) = locate(&instance) else {
            continue;
        };
        if read_bt_u16(devinst, BT_PID_VID) == Some(CONTROLLER_VID)
            && read_bt_u16(devinst, BT_PID_PID) == Some(CONTROLLER_PID)
        {
            found.push(instance);
        }
    }
    found
}

#[derive(Default)]
pub struct BatteryProps {
    pub devnode_found: bool,
    pub level: Option<u8>,
    pub charging: Option<bool>,
    pub timestamp: Option<SystemTime>,
}

pub fn read_battery(instance_id: &str) -> BatteryProps {
    let mut out = BatteryProps::default();
    let Some(devinst) = locate(instance_id) else {
        return out;
    };
    out.devnode_found = true;
    if let Some((ptype, data)) = read_raw(devinst, &battery_key(BATTERY_PID_LEVEL)) {
        if ptype == DEVPROP_TYPE_BYTE && !data.is_empty() {
            out.level = Some(data[0]);
        }
    }
    if let Some((ptype, data)) = read_raw(devinst, &battery_key(BATTERY_PID_CHARGING)) {
        if ptype == DEVPROP_TYPE_BOOLEAN && !data.is_empty() {
            out.charging = Some(data[0] != 0);
        }
    }
    if let Some((ptype, data)) = read_raw(devinst, &battery_key(BATTERY_PID_TIMESTAMP)) {
        if ptype == DEVPROP_TYPE_FILETIME && data.len() >= 8 {
            let filetime = u64::from_le_bytes(data[..8].try_into().unwrap());
            out.timestamp = Some(filetime_to_systemtime(filetime));
        }
    }
    out
}

/// Write the canonical battery schema; returns the three CONFIGRET values.
pub fn write_battery(instance_id: &str, percent: u8, charging: bool) -> Option<[u32; 3]> {
    let devinst = locate(instance_id)?;
    let filetime = filetime_now().to_le_bytes();
    let level = [percent];
    let charging_byte: u8 = if charging { 1 } else { 0 };
    let crs = [
        unsafe {
            CM_Set_DevNode_PropertyW(
                devinst,
                &battery_key(BATTERY_PID_LEVEL) as *const DevPropKey as *const _,
                DEVPROP_TYPE_BYTE,
                level.as_ptr(),
                1,
                0,
            )
        },
        unsafe {
            CM_Set_DevNode_PropertyW(
                devinst,
                &battery_key(BATTERY_PID_CHARGING) as *const DevPropKey as *const _,
                DEVPROP_TYPE_BOOLEAN,
                &charging_byte as *const u8,
                1,
                0,
            )
        },
        unsafe {
            CM_Set_DevNode_PropertyW(
                devinst,
                &battery_key(BATTERY_PID_TIMESTAMP) as *const DevPropKey as *const _,
                DEVPROP_TYPE_FILETIME,
                filetime.as_ptr(),
                8,
                0,
            )
        },
    ];
    Some(crs)
}

pub fn clear_battery(instance_id: &str) -> bool {
    let Some(devinst) = locate(instance_id) else {
        return false;
    };
    for pid in [BATTERY_PID_LEVEL, BATTERY_PID_CHARGING, BATTERY_PID_TIMESTAMP] {
        unsafe {
            CM_Set_DevNode_PropertyW(
                devinst,
                &battery_key(pid) as *const DevPropKey as *const _,
                DEVPROP_TYPE_EMPTY,
                std::ptr::null(),
                0,
                0,
            );
        }
    }
    true
}
