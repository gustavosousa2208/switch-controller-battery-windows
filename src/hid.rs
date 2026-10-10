//! HID plumbing for Switch-mode controllers (via the hidapi crate).

use hidapi::{DeviceInfo, HidApi, HidDevice};

use crate::props::{CONTROLLER_PID, CONTROLLER_VID};

pub const BATTERY_REPORT_IDS: [u8; 2] = [0x30, 0x21];
pub const INPUT_MODE_FULL: u8 = 0x30;
#[allow(dead_code)] // protocol constant, kept for diagnostics
pub const INPUT_MODE_SIMPLE: u8 = 0x3F;

const SUBCMD_SET_INPUT_REPORT_MODE: u8 = 0x03;
const NEUTRAL_RUMBLE: [u8; 8] = [0x00, 0x01, 0x40, 0x40, 0x00, 0x01, 0x40, 0x40];
const REPORT_BUFFER: usize = 512;

/// Output report 0x01: `[0x01, counter, rumble(8), subcmd, args...]`.
pub fn make_subcommand_packet(counter: u8, subcmd: u8, args: &[u8]) -> Vec<u8> {
    let mut packet = vec![0u8; 49];
    packet[0] = 0x01;
    packet[1] = counter & 0x0F;
    packet[2..10].copy_from_slice(&NEUTRAL_RUMBLE);
    packet[10] = subcmd;
    let count = args.len().min(49 - 11);
    packet[11..11 + count].copy_from_slice(&args[..count]);
    packet
}

pub fn is_controller(info: &DeviceInfo) -> bool {
    let product = info.product_string().unwrap_or_default().to_lowercase();
    (info.vendor_id() == CONTROLLER_VID && info.product_id() == CONTROLLER_PID)
        || product.contains("8bitdo")
        || product.contains("pro controller")
        || product.contains("wireless gamepad")
}

fn rank(info: &DeviceInfo) -> u8 {
    let path = info.path().to_string_lossy().to_lowercase();
    if path.contains("00001124") || path.contains("bth") {
        0
    } else if path.contains("usb#") {
        1
    } else {
        2
    }
}

/// Open the first controller interface that delivers reports.
pub fn open_controller(api: &mut HidApi, log: &mut dyn FnMut(&str)) -> Option<HidDevice> {
    // The hidapi crate caches the device list; re-enumerate before each scan.
    let _ = api.refresh_devices();
    let mut candidates: Vec<&DeviceInfo> = api.device_list().filter(|d| is_controller(d)).collect();
    candidates.sort_by_key(|d| rank(d));
    for info in candidates {
        log(&format!(
            "candidate: {:?} VID={:#06x} PID={:#06x} path={:?}",
            info.product_string().unwrap_or_default(),
            info.vendor_id(),
            info.product_id(),
            info.path().to_string_lossy()
        ));
        let device = match api.open_path(info.path()) {
            Ok(device) => device,
            Err(err) => {
                log(&format!("  open failed: {err}"));
                continue;
            }
        };
        let _ = device.set_blocking_mode(false);
        let mut buf = [0u8; REPORT_BUFFER];
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while std::time::Instant::now() < deadline {
            match device.read_timeout(&mut buf, 25) {
                Ok(n) if n >= 1 => {
                    log(&format!(
                        "opened: {} (first report id 0x{:02x})",
                        info.product_string().unwrap_or("controller"),
                        buf[0]
                    ));
                    return Some(device);
                }
                Ok(_) => {}
                Err(err) => {
                    log(&format!("  read failed: {err}"));
                    break;
                }
            }
        }
        log("  no reports in 10 s; trying next interface");
    }
    None
}

/// Ask the controller to switch to full reports (subcommand 0x03/0x30).
pub fn request_full_mode(device: &HidDevice, counter: u8) -> bool {
    match device.write(&make_subcommand_packet(
        counter,
        SUBCMD_SET_INPUT_REPORT_MODE,
        &[INPUT_MODE_FULL],
    )) {
        Ok(n) => n == 49,
        Err(_) => false,
    }
}

/// Linear mapping of the Nintendo level code 0..8 to 0..100 percent.
pub fn level_to_percent(level: u8) -> u8 {
    let value = (u32::from(level) * 100 + 4) / 8;
    value.min(100) as u8
}
