//! switch-battery: Windows battery badge for Switch-mode controllers.

mod hid;
mod logger;
mod props;
mod publisher;
mod task;
mod util;

use std::path::PathBuf;
use std::time::Duration;

use logger::Logger;

fn is_admin() -> bool {
    unsafe { windows_sys::Win32::UI::Shell::IsUserAnAdmin() != 0 }
}

fn hide_console() {
    unsafe {
        let hwnd = windows_sys::Win32::System::Console::GetConsoleWindow();
        if !hwnd.is_null() {
            windows_sys::Win32::UI::WindowsAndMessaging::ShowWindow(
                hwnd,
                windows_sys::Win32::UI::WindowsAndMessaging::SW_HIDE,
            );
        }
    }
}

struct Opts {
    instance: Option<String>,
    heartbeat: Duration,
    refresh: Option<Duration>,
    log: Option<PathBuf>,
    hidden: bool,
    no_test: bool,
    value: Option<u8>,
}

fn parse_opts(args: &[String]) -> Opts {
    let mut opts = Opts {
        instance: None,
        heartbeat: Duration::from_secs(5),
        refresh: Some(Duration::from_secs(30)),
        log: Some(logger::default_log_path()),
        hidden: false,
        no_test: false,
        value: None,
    };
    let mut i = 0;
    while i < args.len() {
        let arg = args[i].clone();
        let (key, inline) = match arg.split_once('=') {
            Some((k, v)) => (k.to_string(), Some(v.to_string())),
            None => (arg.clone(), None),
        };
        match key.as_str() {
            "--instance" => {
                opts.instance = take_value(args, &mut i, inline.as_deref());
            }
            "--heartbeat" => {
                if let Some(v) = take_value(args, &mut i, inline.as_deref()) {
                    if let Ok(secs) = v.parse::<u64>() {
                        opts.heartbeat = Duration::from_secs(secs.max(1));
                    }
                }
            }
            "--refresh" => {
                if let Some(v) = take_value(args, &mut i, inline.as_deref()) {
                    if let Ok(secs) = v.parse::<u64>() {
                        opts.refresh = if secs == 0 {
                            None
                        } else {
                            Some(Duration::from_secs(secs))
                        };
                    }
                }
            }
            "--log" => {
                if let Some(v) = take_value(args, &mut i, inline.as_deref()) {
                    opts.log = if v.is_empty() {
                        None
                    } else {
                        Some(PathBuf::from(v))
                    };
                }
            }
            "--hidden" => opts.hidden = true,
            "--no-test" => opts.no_test = true,
            other => {
                if let Ok(v) = other.parse::<u8>() {
                    opts.value = Some(v);
                }
            }
        }
        i += 1;
    }
    opts
}

fn take_value(args: &[String], i: &mut usize, inline: Option<&str>) -> Option<String> {
    if let Some(v) = inline {
        return Some(v.to_string());
    }
    *i += 1;
    args.get(*i).cloned()
}

fn print_usage() {
    println!(
        "switch-battery {}\n\n\
Usage:\n\
  switch-battery run [--instance ID] [--heartbeat S] [--refresh S] [--log PATH] [--hidden]\n\
  switch-battery install [--no-test]\n\
  switch-battery uninstall\n\
  switch-battery status\n\
  switch-battery read [--instance ID]\n\
  switch-battery set VALUE [--instance ID]\n\
  switch-battery clear [--instance ID]\n",
        env!("CARGO_PKG_VERSION")
    );
}

fn resolve_instance(explicit: Option<String>) -> Option<String> {
    explicit.or_else(|| props::find_controller_instances().first().cloned())
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (command, rest) = match args.first() {
        Some(first) if !first.starts_with('-') => (first.as_str(), &args[1..]),
        _ => ("run", &args[..]),
    };
    let opts = parse_opts(rest);

    match command {
        "run" => {
            if opts.hidden {
                hide_console();
            }
            publisher::install_ctrl_handler();
            let mut log = Logger::new(opts.log.as_deref());
            if let Some(path) = opts.log.as_deref() {
                log.log(&format!("log file: {}", path.display()));
            }
            if !is_admin() {
                log.log(
                    "WARNING: not elevated; property writes will fail \
                     (run elevated or use 'install').",
                );
            }
            let code = publisher::run(
                &publisher::RunConfig {
                    instance: opts.instance.clone(),
                    heartbeat: opts.heartbeat,
                    refresh: opts.refresh,
                },
                &mut log,
            );
            std::process::exit(code);
        }
        "install" => {
            if !is_admin() {
                eprintln!("install requires an elevated console");
                std::process::exit(1);
            }
            let mut log = Logger::new(None);
            std::process::exit(task::install(opts.no_test, &mut log));
        }
        "uninstall" => {
            if !is_admin() {
                eprintln!("uninstall requires an elevated console");
                std::process::exit(1);
            }
            let mut log = Logger::new(None);
            std::process::exit(task::uninstall(&mut log));
        }
        "status" => {
            let mut log = Logger::new(None);
            log.log(&format!(
                "task '{}' installed: {}",
                task::TASK_NAME,
                if task::task_exists() { "yes" } else { "no" }
            ));
            match resolve_instance(opts.instance.clone()) {
                Some(instance) => {
                    log.log(&format!("controller devnode: {instance}"));
                    let state = props::read_battery(&instance);
                    match (state.level, state.charging) {
                        (Some(level), Some(charging)) => {
                            log.log(&format!(
                                "battery property: {level}% (charging: {charging})"
                            ));
                            if let Some(ts) = state.timestamp {
                                let age = std::time::SystemTime::now()
                                    .duration_since(ts)
                                    .map(|d| d.as_secs())
                                    .unwrap_or(0);
                                log.log(&format!("last update: {age} s ago"));
                            }
                        }
                        _ => log.log(
                            "battery properties not set (publisher has not \
                             published yet)",
                        ),
                    }
                }
                None => log.log("controller devnode not found"),
            }
            std::process::exit(0);
        }
        "read" => {
            let mut log = Logger::new(None);
            let Some(instance) = resolve_instance(opts.instance.clone()) else {
                log.log("no matching controller devnode found; pass --instance");
                std::process::exit(1);
            };
            log.log(&format!("target: {instance}"));
            let state = props::read_battery(&instance);
            if !state.devnode_found {
                log.log("devnode not found");
                std::process::exit(1);
            }
            log.log(&format!("level: {:?}", state.level));
            log.log(&format!("charging: {:?}", state.charging));
            log.log(&format!("timestamp: {:?}", state.timestamp));
            std::process::exit(0);
        }
        "set" => {
            let mut log = Logger::new(None);
            if !is_admin() {
                log.log("note: writing properties needs an elevated console");
            }
            let value = opts.value.unwrap_or(42);
            let Some(instance) = resolve_instance(opts.instance.clone()) else {
                log.log("no matching controller devnode found; pass --instance");
                std::process::exit(1);
            };
            match props::write_battery(&instance, value, false) {
                Some(crs) => {
                    log.log(&format!("wrote {value}% (crs={crs:02x?})"));
                    std::process::exit(if crs.iter().all(|c| *c == 0) { 0 } else { 1 });
                }
                None => {
                    log.log("devnode not found");
                    std::process::exit(1);
                }
            }
        }
        "clear" => {
            let mut log = Logger::new(None);
            let Some(instance) = resolve_instance(opts.instance.clone()) else {
                log.log("no matching controller devnode found; pass --instance");
                std::process::exit(1);
            };
            if props::clear_battery(&instance) {
                log.log("battery properties cleared");
                std::process::exit(0);
            } else {
                log.log("devnode not found");
                std::process::exit(1);
            }
        }
        "help" | "--help" | "-h" => {
            print_usage();
            std::process::exit(0);
        }
        other => {
            eprintln!("unknown command: {other}\n");
            print_usage();
            std::process::exit(2);
        }
    }
}
