use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use tracing::level_filters::LevelFilter;
use tracing_subscriber::filter::Targets;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

const FILE_MAX_SIZE: u64 = 10 * 1024 * 1024;
const FILE_BACKUPS: usize = 3;
pub const LOG_FILE: &str = "rain-dial.log";

/// 按大小滚动的日志文件：超过 10MB 时改名为 .1，最多保留 3 个旧文件。
pub struct RotatingFile {
    path: PathBuf,
    state: Mutex<Option<(File, u64)>>,
}

impl RotatingFile {
    pub fn new(path: PathBuf) -> Self {
        RotatingFile {
            path,
            state: Mutex::new(None),
        }
    }

    fn open(&self) -> io::Result<(File, u64)> {
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir)?;
        }
        let f = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        let size = f.metadata().map(|m| m.len()).unwrap_or(0);
        Ok((f, size))
    }

    fn rotate(&self) {
        for i in (1..FILE_BACKUPS).rev() {
            let _ = fs::rename(backup(&self.path, i), backup(&self.path, i + 1));
        }
        let _ = fs::rename(&self.path, backup(&self.path, 1));
    }

    fn write_all(&self, buf: &[u8]) -> io::Result<()> {
        let mut st = self.state.lock().unwrap();
        if st
            .as_ref()
            .is_some_and(|(_, size)| *size + buf.len() as u64 > FILE_MAX_SIZE)
        {
            *st = None;
            self.rotate();
        }
        if st.is_none() {
            *st = Some(self.open()?);
        }
        let (f, size) = st.as_mut().unwrap();
        f.write_all(buf)?;
        *size += buf.len() as u64;
        Ok(())
    }
}

fn backup(path: &Path, i: usize) -> PathBuf {
    let mut s = path.as_os_str().to_owned();
    s.push(format!(".{i}"));
    PathBuf::from(s)
}

pub struct FileWriter<'a>(&'a RotatingFile);

impl Write for FileWriter<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.write_all(buf)?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for RotatingFile {
    type Writer = FileWriter<'a>;

    fn make_writer(&'a self) -> Self::Writer {
        FileWriter(self)
    }
}

pub fn parse_level(s: &str) -> LevelFilter {
    match s {
        "debug" => LevelFilter::DEBUG,
        "warn" => LevelFilter::WARN,
        "error" => LevelFilter::ERROR,
        _ => LevelFilter::INFO,
    }
}

/// 本程序按配置级别输出；第三方库最多输出到 warn，避免 russh、sqlx 等刷屏。
fn targets(level: &str) -> Targets {
    let lvl = parse_level(level);
    Targets::new()
        .with_target(env!("CARGO_CRATE_NAME"), lvl)
        .with_default(lvl.min(LevelFilter::WARN))
}

/// 初始化全局日志：写入 logs/rain-dial.log（JSON 行），调试版同时输出到控制台。
pub fn init(log_dir: &Path, level: &str) {
    let file = RotatingFile::new(log_dir.join(LOG_FILE));
    let file_layer = tracing_subscriber::fmt::layer()
        .json()
        .with_timer(tracing_subscriber::fmt::time::ChronoLocal::rfc_3339())
        .with_current_span(false)
        .with_span_list(false)
        .with_writer(file);
    let console = cfg!(debug_assertions).then(|| {
        tracing_subscriber::fmt::layer()
            .with_timer(tracing_subscriber::fmt::time::ChronoLocal::new(
                "%H:%M:%S%.3f".into(),
            ))
            .with_writer(io::stderr)
    });
    let _ = tracing_subscriber::registry()
        .with(targets(level))
        .with(file_layer)
        .with(console)
        .try_init();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotating_file_rotates() {
        let dir = std::env::temp_dir().join(format!("raindial-log-{}", crate::util::new_id()));
        let f = RotatingFile::new(dir.join("a.log"));
        let chunk = vec![b'x'; 1024 * 1024];
        for _ in 0..12 {
            f.write_all(&chunk).unwrap();
        }
        assert!(dir.join("a.log.1").exists());
        assert!(fs::metadata(dir.join("a.log")).unwrap().len() <= FILE_MAX_SIZE);
        let _ = fs::remove_dir_all(dir);
    }
}
