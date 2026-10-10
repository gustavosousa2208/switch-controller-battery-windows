//! Log file with 1 MB rotation; console output goes through println!.
//! The default log lives in `<exe dir>/logs/publisher.log`.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

pub struct Logger {
    file: Option<File>,
}

impl Logger {
    pub fn new(path: Option<&Path>) -> Logger {
        let file = path.and_then(|p| {
            if let Some(parent) = p.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Ok(meta) = std::fs::metadata(p) {
                if meta.len() > 1_000_000 {
                    let mut rotated = p.as_os_str().to_owned();
                    rotated.push(".1");
                    let _ = std::fs::rename(p, PathBuf::from(rotated));
                }
            }
            OpenOptions::new().create(true).append(true).open(p).ok()
        });
        Logger { file }
    }

    pub fn log(&mut self, message: &str) {
        println!("{message}");
        if let Some(file) = self.file.as_mut() {
            let _ = writeln!(file, "{message}");
        }
    }
}

pub fn default_log_path() -> PathBuf {
    let base = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("logs").join("publisher.log")
}
