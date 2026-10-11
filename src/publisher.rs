//! The publisher loop: HID reports in, Windows battery properties out.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use hidapi::HidApi;

use crate::hid;
use crate::logger::Logger;
use crate::props;
use crate::util::local_hms;

static CANCEL: AtomicBool = AtomicBool::new(false);

extern "system" fn ctrl_handler(_ctrl_type: u32) -> i32 {
    CANCEL.store(true, Ordering::SeqCst);
    1
}

pub fn install_ctrl_handler() {
    unsafe {
        windows_sys::Win32::System::Console::SetConsoleCtrlHandler(Some(ctrl_handler), 1);
    }
}

pub struct RunConfig {
    pub instance: Option<String>,
    pub heartbeat: Duration,
    pub refresh: Option<Duration>,
}

pub fn run(cfg: &RunConfig, log: &mut Logger) -> i32 {
    let mut api = match HidApi::new() {
        Ok(api) => api,
        Err(err) => {
            log.log(&format!("hidapi init failed: {err}"));
            return 1;
        }
    };

    let mut instance_id = cfg.instance.clone();
    if instance_id.is_none() {
        if let Some(first) = props::find_controller_instances().first() {
            instance_id = Some(first.clone());
        }
    }
    log.log(&format!(
        "controller devnode: {}",
        instance_id.as_deref().unwrap_or("(not found yet)")
    ));
    log.log("publisher running; Ctrl+C to stop and clear\n");

    let mut dev: Option<hidapi::HidDevice> = None;
    let mut last_written: Option<(u8, bool)> = None;
    let mut last_write_time = Instant::now() - Duration::from_secs(3600);
    let mut seen_battery = false;
    let mut last_hint = Instant::now();
    let mut last_not_found_log: Option<Instant> = None;
    let mut packet_counter: u8 = 0;
    let mut last_0x30 = Instant::now();
    let mut last_any_report = Instant::now();

    while !CANCEL.load(Ordering::SeqCst) {
        if dev.is_none() {
            dev = hid::open_controller(&mut api, &mut |msg| log.log(msg));
            if dev.is_none() {
                let now = Instant::now();
                let due = last_not_found_log
                    .map(|t| now.duration_since(t) >= Duration::from_secs(60))
                    .unwrap_or(true);
                if due {
                    last_not_found_log = Some(now);
                    log.log("controller not found; retrying every 3 s");
                }
                std::thread::sleep(Duration::from_secs(3));
                continue;
            }
            seen_battery = false;
            last_hint = Instant::now();
            last_0x30 = Instant::now();
            last_any_report = Instant::now();
            if cfg.refresh.is_some() {
                let ok = hid::request_full_mode(dev.as_ref().unwrap(), packet_counter);
                packet_counter = (packet_counter + 1) & 0x0F;
                log.log(if ok {
                    "requested full mode (0x03/0x30)"
                } else {
                    "full-mode request rejected; continuing passively (will retry)"
                });
            }
        }

        let mut buf = [0u8; 512];
        match dev.as_ref().unwrap().read_timeout(&mut buf, 5) {
            Ok(n) if n >= 3 && hid::BATTERY_REPORT_IDS.contains(&buf[0]) => {
                last_any_report = Instant::now();
                seen_battery = true;
                if buf[0] == 0x30 {
                    last_0x30 = Instant::now();
                }
                let battery_byte = buf[2];
                let level = (battery_byte & 0xE0) >> 4;
                let percent = hid::level_to_percent(level);
                let charging = battery_byte & 0x01 != 0;
                if last_written != Some((percent, charging)) {
                    write_and_log(&mut instance_id, log, buf[0], level, percent, charging);
                    last_written = Some((percent, charging));
                    last_write_time = Instant::now();
                }
            }
            Ok(n) if n >= 1 => {
                last_any_report = Instant::now();
                if !seen_battery && last_hint.elapsed() >= Duration::from_secs(30) {
                    last_hint = Instant::now();
                    log.log("waiting for battery reports; the full-mode request has been sent");
                }
            }
            Ok(_) => {}
            Err(err) => {
                log.log(&format!("controller disconnected or read failed: {err}"));
                dev = None;
                last_written = None;
                if let Some(instance) = ensure_instance(&mut instance_id, log) {
                    if props::clear_battery(&instance) {
                        log.log("cleared battery properties (no stale value)");
                    } else {
                        log.log("devnode not found while clearing (skipped)");
                    }
                }
                log.log("waiting for the controller to come back...");
                std::thread::sleep(Duration::from_secs(1));
                continue;
            }
        }

        // Silence guard: a live controller streams reports continuously
        // (50-100/s). A handle that sees nothing at all is stale (observed
        // after overnight sleep/reconnect cycles) - close and reopen it.
        if dev.is_some() && last_any_report.elapsed() >= Duration::from_secs(30) {
            log.log("no reports for 30 s; reopening the controller handle");
            dev = None;
            last_not_found_log = None;
            continue;
        }

        // Heartbeat: re-assert the last published value so soft devnode
        // properties (cleared on device re-arrival) stay alive.
        if let Some(state) = last_written {
            if last_write_time.elapsed() >= cfg.heartbeat {
                if let Some(instance) = ensure_instance(&mut instance_id, log) {
                    if let Some(crs) = props::write_battery(&instance, state.0, state.1) {
                        if crs.iter().any(|c| *c != 0) {
                            log.log(&format!(
                                "{} re-assert FAILED (crs={:02x?})",
                                local_hms(),
                                crs
                            ));
                        }
                    }
                    last_write_time = Instant::now();
                }
            }
        }

        // Refresh watchdog: if the full (0x30) stream stopped, ask again.
        if let Some(refresh) = cfg.refresh {
            if last_0x30.elapsed() >= refresh {
                if let Some(device) = dev.as_ref() {
                    let ok = hid::request_full_mode(device, packet_counter);
                    packet_counter = (packet_counter + 1) & 0x0F;
                    log.log(&format!(
                        "{} no 0x30 reports for {}s; re-requested full mode ({})",
                        local_hms(),
                        refresh.as_secs(),
                        if ok { "ok" } else { "write rejected" }
                    ));
                    last_0x30 = Instant::now();
                }
            }
        }

        std::thread::sleep(Duration::from_millis(2));
    }

    log.log("\nstopping (Ctrl+C)");
    if let Some(instance) = ensure_instance(&mut instance_id, log) {
        if props::clear_battery(&instance) {
            log.log("cleared battery properties on exit");
        } else {
            log.log("devnode not found on exit (nothing to clear)");
        }
    }
    0
}

fn ensure_instance(instance_id: &mut Option<String>, log: &mut Logger) -> Option<String> {
    if instance_id.is_none() {
        if let Some(first) = props::find_controller_instances().first() {
            *instance_id = Some(first.clone());
            log.log(&format!("controller devnode: {}", first));
        }
    }
    instance_id.clone()
}

fn write_and_log(
    instance_id: &mut Option<String>,
    log: &mut Logger,
    report_id: u8,
    level: u8,
    percent: u8,
    charging: bool,
) {
    let Some(instance) = ensure_instance(instance_id, log) else {
        log.log(&format!("{} devnode not found; write skipped", local_hms()));
        return;
    };
    match props::write_battery(&instance, percent, charging) {
        Some(crs) if crs.iter().all(|c| *c == 0) => log.log(&format!(
            "{} src=0x{:02x} level={}/8 -> {}%{} written=ok",
            local_hms(),
            report_id,
            level,
            percent,
            if charging { " charging" } else { "" }
        )),
        Some(crs) => log.log(&format!(
            "{} src=0x{:02x} level={}/8 -> {}% written=FAILED (crs={:02x?})",
            local_hms(),
            report_id,
            level,
            percent,
            crs
        )),
        None => log.log(&format!("{} devnode not found; write skipped", local_hms())),
    }
}
