use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::Serialize;
use serde_json::Value;
use tracing::level_filters::LevelFilter;
use tracing_subscriber::filter::Targets;
use tracing_subscriber::fmt::MakeWriter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{Registry, reload};

const FILE_MAX_SIZE: u64 = 10 * 1024 * 1024;
const FILE_BACKUPS: usize = 3;
pub const LOG_FILE: &str = "rain-dial.log";
/// 查看日志时每个文件最多从末尾读这么多字节。
const TAIL_BYTES: u64 = 2 * 1024 * 1024;

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

#[derive(Clone)]
pub struct LevelHandle(reload::Handle<Targets, Registry>);

impl LevelHandle {
    pub fn set(&self, level: &str) {
        let _ = self.0.reload(targets(level));
    }
}

/// 初始化全局日志：写入 logs/rain-dial.log（JSON 行），调试版同时输出到控制台。
pub fn init(log_dir: &Path, level: &str) -> LevelHandle {
    let (filter, handle) = reload::Layer::new(targets(level));
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
        .with(filter)
        .with(file_layer)
        .with(console)
        .try_init();
    LevelHandle(handle)
}

/// 日志文件里的一条记录。不是 JSON 的行整行放在 message 里。
#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LogEntry {
    pub time: String,
    pub level: String,
    pub target: String,
    pub message: String,
    /// message 以外的字段，值转成文本。
    pub fields: Vec<(String, String)>,
}

/// 最近的 limit 条日志，按时间先后排列。当前文件不够时接着读上一个滚动出去的文件。
pub fn tail(log_dir: &Path, limit: usize) -> io::Result<Vec<LogEntry>> {
    let path = log_dir.join(LOG_FILE);
    let mut lines = tail_lines(&path, limit)?;
    if lines.len() < limit {
        let mut older = tail_lines(&backup(&path, 1), limit - lines.len())?;
        older.append(&mut lines);
        lines = older;
    }
    Ok(lines.iter().map(|l| parse_entry(l)).collect())
}

fn tail_lines(path: &Path, limit: usize) -> io::Result<Vec<String>> {
    let mut f = match File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let start = f.metadata()?.len().saturating_sub(TAIL_BYTES);
    f.seek(SeekFrom::Start(start))?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf)?;
    let text = String::from_utf8_lossy(&buf);
    // 从文件中间开始读时，第一行多半不完整。
    let text = if start > 0 {
        text.split_once('\n').map_or("", |(_, rest)| rest)
    } else {
        &text
    };
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    let skip = lines.len().saturating_sub(limit);
    Ok(lines[skip..].iter().map(|l| l.to_string()).collect())
}

fn parse_entry(line: &str) -> LogEntry {
    let Ok(Value::Object(mut obj)) = serde_json::from_str::<Value>(line) else {
        return LogEntry {
            time: String::new(),
            level: String::new(),
            target: String::new(),
            message: line.to_string(),
            fields: Vec::new(),
        };
    };
    let mut take = |k: &str| match obj.remove(k) {
        Some(Value::String(s)) => s,
        _ => String::new(),
    };
    let (time, level, target) = (take("timestamp"), take("level"), take("target"));
    let mut fields = match obj.remove("fields") {
        Some(Value::Object(m)) => m,
        _ => Default::default(),
    };
    let message = match fields.remove("message") {
        Some(Value::String(s)) => s,
        Some(v) => v.to_string(),
        None => String::new(),
    };
    let fields = fields
        .into_iter()
        .map(|(k, v)| {
            let v = match v {
                Value::String(s) => s,
                v => v.to_string(),
            };
            (k, v)
        })
        .collect();
    LogEntry {
        time,
        level,
        target,
        message,
        fields,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tail_reads_newest_entries_across_rotation() {
        let dir = std::env::temp_dir().join(format!("raindial-log-{}", crate::util::new_id()));
        fs::create_dir_all(&dir).unwrap();
        let line = |n: u32| {
            format!(
                r#"{{"timestamp":"2026-11-27T10:00:0{n}+08:00","level":"INFO","fields":{{"message":"m{n}","count":{n}}},"target":"rain_dial::x"}}"#
            )
        };
        fs::write(
            dir.join(format!("{LOG_FILE}.1")),
            format!("{}\n{}\n", line(1), line(2)),
        )
        .unwrap();
        fs::write(dir.join(LOG_FILE), format!("{}\nnot json\n", line(3))).unwrap();

        let got = tail(&dir, 3).unwrap();
        let messages: Vec<_> = got.iter().map(|e| e.message.as_str()).collect();
        assert_eq!(messages, ["m2", "m3", "not json"]);
        assert_eq!(got[0].level, "INFO");
        assert_eq!(got[0].target, "rain_dial::x");
        assert_eq!(got[0].fields, [("count".to_string(), "2".to_string())]);
        assert!(got[2].level.is_empty());

        assert!(tail(&dir.join("missing"), 10).unwrap().is_empty());
        let _ = fs::remove_dir_all(dir);
    }

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
