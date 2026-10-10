//! Task Scheduler integration via schtasks.exe (locale-aware output avoided).

use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;

use crate::logger::Logger;

pub const TASK_NAME: &str = "SwitchControllerBattery";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

fn schtasks(args: &[&str]) -> std::process::Output {
    Command::new("schtasks")
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .expect("failed to run schtasks")
}

pub fn task_exists() -> bool {
    schtasks(&["/Query", "/TN", TASK_NAME]).status.success()
}

fn stop_task() {
    let _ = schtasks(&["/End", "/TN", TASK_NAME]);
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn build_task_xml() -> String {
    let exe = std::env::current_exe().expect("current exe path");
    let exe = exe.canonicalize().unwrap_or(exe);
    let exe_str = exe.to_string_lossy().to_string();
    let workdir = exe
        .parent()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    let user = format!(
        "{}\\{}",
        std::env::var("USERDOMAIN").unwrap_or_default(),
        std::env::var("USERNAME").unwrap_or_default()
    );
    let start = crate::util::local_iso();
    format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo>
    <Description>Publishes the controller battery to Windows Settings.</Description>
  </RegistrationInfo>
  <Triggers>
    <LogonTrigger>
      <Enabled>true</Enabled>
      <UserId>{user}</UserId>
      <Delay>PT30S</Delay>
    </LogonTrigger>
    <TimeTrigger>
      <StartBoundary>{start}</StartBoundary>
      <Enabled>true</Enabled>
      <Repetition>
        <Interval>PT1M</Interval>
        <Duration>P3650D</Duration>
        <StopAtDurationEnd>false</StopAtDurationEnd>
      </Repetition>
    </TimeTrigger>
  </Triggers>
  <Principals>
    <Principal id="Author">
      <UserId>{user}</UserId>
      <LogonType>InteractiveToken</LogonType>
      <RunLevel>HighestAvailable</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <AllowHardTerminate>true</AllowHardTerminate>
    <StartWhenAvailable>true</StartWhenAvailable>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <Enabled>true</Enabled>
    <Hidden>true</Hidden>
    <RestartOnFailure>
      <Interval>PT1M</Interval>
      <Count>999</Count>
    </RestartOnFailure>
  </Settings>
  <Actions Context="Author">
    <Exec>
      <Command>{exe}</Command>
      <Arguments>run --hidden</Arguments>
      <WorkingDirectory>{workdir}</WorkingDirectory>
    </Exec>
  </Actions>
</Task>
"#,
        user = xml_escape(&user),
        start = start,
        exe = xml_escape(&exe_str),
        workdir = xml_escape(&workdir),
    )
}

fn write_task_xml(xml: &str) -> PathBuf {
    let path = std::env::temp_dir().join("switch-battery-task.xml");
    let mut bytes: Vec<u8> = vec![0xFF, 0xFE]; // UTF-16 LE BOM
    for unit in xml.encode_utf16() {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    std::fs::write(&path, bytes).expect("write task xml");
    path
}

/// PIDs of running `switch-battery.exe` processes (excluding this one).
pub fn list_publisher_pids() -> Vec<u32> {
    let output = Command::new("tasklist")
        .args(["/FI", "IMAGENAME eq switch-battery.exe", "/FO", "CSV", "/NH"])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    let Ok(output) = output else {
        return Vec::new();
    };
    let text = String::from_utf8_lossy(&output.stdout);
    let self_pid = std::process::id();
    let mut pids = Vec::new();
    for line in text.lines() {
        let fields: Vec<&str> = line.split("\",\"").collect();
        if fields.len() >= 2 {
            if let Ok(pid) = fields[1].trim_matches('"').parse::<u32>() {
                if pid != self_pid {
                    pids.push(pid);
                }
            }
        }
    }
    pids
}

fn kill_pid(pid: u32) {
    let _ = Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/F"])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
}

pub fn install(skip_watchdog_test: bool, log: &mut Logger) -> i32 {
    log.log("stopping any running publisher instances...");
    stop_task();
    for pid in list_publisher_pids() {
        kill_pid(pid);
    }
    std::thread::sleep(std::time::Duration::from_millis(800));

    let xml = build_task_xml();
    let xml_path = write_task_xml(&xml);
    let xml_path_str = xml_path.to_string_lossy().to_string();

    let out = schtasks(&["/Create", "/TN", TASK_NAME, "/XML", &xml_path_str, "/F"]);
    if !out.status.success() {
        log.log(&format!(
            "task registration failed:\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&out.stdout).trim(),
            String::from_utf8_lossy(&out.stderr).trim()
        ));
        return 1;
    }
    log.log(&format!("task '{TASK_NAME}' registered"));

    if !schtasks(&["/Run", "/TN", TASK_NAME]).status.success() {
        log.log("failed to start the task");
        return 1;
    }

    log.log("waiting for the hidden publisher process...");
    let mut first_pid = None;
    for _ in 0..30 {
        std::thread::sleep(std::time::Duration::from_secs(1));
        if let Some(pid) = list_publisher_pids().first() {
            first_pid = Some(*pid);
            break;
        }
    }
    let Some(first_pid) = first_pid else {
        log.log("publisher process did not appear");
        return 1;
    };
    log.log(&format!("hidden publisher running as pid {first_pid}"));

    if !skip_watchdog_test {
        log.log(&format!(
            "watchdog test: killing pid {first_pid} (expect a new instance within ~1-2 min)..."
        ));
        kill_pid(first_pid);
        let mut new_pid = None;
        for i in 0..150 {
            std::thread::sleep(std::time::Duration::from_secs(1));
            if let Some(pid) = list_publisher_pids().first() {
                if *pid != first_pid {
                    new_pid = Some(*pid);
                    break;
                }
            }
            if i % 15 == 14 {
                log.log(&format!("  waiting for restart... {}s", i + 1));
            }
        }
        match new_pid {
            Some(pid) => log.log(&format!("watchdog test PASSED: new pid {pid}")),
            None => {
                log.log("watchdog test FAILED (no restart within 150 s); starting manually");
                let _ = schtasks(&["/Run", "/TN", TASK_NAME]);
            }
        }
    }
    log.log("done.");
    0
}

pub fn uninstall(log: &mut Logger) -> i32 {
    stop_task();
    let out = schtasks(&["/Delete", "/TN", TASK_NAME, "/F"]);
    if out.status.success() {
        log.log(&format!("task '{TASK_NAME}' removed"));
        0
    } else {
        log.log("task not found or could not be removed");
        1
    }
}
