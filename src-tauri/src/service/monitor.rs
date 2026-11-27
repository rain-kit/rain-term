//! 服务器监控：在终端已建立的 SSH 连接上定时执行一段脚本读取 /proc，
//! 算出 CPU、内存、负载、网络速度和占用最高的进程。只支持 Linux 服务器。

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use tokio_util::sync::CancellationToken;
use tracing::debug;

use super::conn::ConnService;
use super::pool::{Client, ClientPool};
use crate::error::{self, ApiResult};
use crate::i18n::Msg;
use crate::util::new_id;

/// 两次采集的间隔。
pub const INTERVAL: Duration = Duration::from_secs(3);
/// 第一次采集只有累计值，算不出进程的 CPU 占用，隔这么久就再采一次。
const FIRST_INTERVAL: Duration = Duration::from_secs(1);
const SAMPLE_TIMEOUT: Duration = Duration::from_secs(10);
/// 采集脚本输出的上限，进程很多时也够用。
const MAX_OUTPUT: usize = 4 * 1024 * 1024;
/// 按 CPU、按内存各取占用最高的几个进程；也是前端可以设置的最大显示数。
pub const TOP: usize = 30;

/// 采集脚本，从标准输入交给 sh 执行，不受用户登录 shell（如 fish）语法的影响。
/// 进程一行一个：pid ppid utime stime rss 名称；名称可能含空格和括号，放在最后。
/// 先用 cat 读：进程在读取前退出时 cat 会跳过，部分 awk 遇到打不开的文件会直接退出。
const SCRIPT: &str = r##"LC_ALL=C; export LC_ALL
if [ ! -r /proc/stat ]; then echo '#unsupported'; exit 0; fi
echo "#self $$"
echo "#page $(getconf PAGESIZE 2>/dev/null)"
echo '#cpu'; grep '^cpu' /proc/stat
echo '#load'; cat /proc/loadavg
echo '#mem'; cat /proc/meminfo
echo '#uptime'; cat /proc/uptime
echo '#net'; cat /proc/net/dev 2>/dev/null
echo '#phys'; ls -d /sys/class/net/*/device 2>/dev/null
echo '#disk'; df -kP / 2>/dev/null
echo '#ps'
cat /proc/[0-9]*/stat 2>/dev/null | awk '{ s = $0; sub(/^[0-9]+ \(/, "", s); n = s; sub(/\) [^)]*$/, "", n); sub(/.*\) /, "", s); split(s, f, " "); print $1, f[2], f[12], f[13], f[22], n }'
"##;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Proc {
    pub pid: u32,
    pub name: String,
    /// 占用的 CPU，100 表示占满一个核；刚打开、还没有上一次采集时为空。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu: Option<f32>,
    /// 常驻内存，字节。
    pub mem: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Disk {
    pub total: u64,
    pub used: u64,
    pub avail: u64,
}

/// 每秒收发的字节数，不含回环和容器的虚拟网卡。
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Net {
    pub rx: u64,
    pub tx: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    /// 总的 CPU 使用率 0 ~ 100；第一次采集时为空。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cpu: Option<f32>,
    pub cores: u32,
    /// 1、5、15 分钟的平均负载。
    pub load: [f64; 3],
    pub mem_total: u64,
    pub mem_used: u64,
    pub swap_total: u64,
    pub swap_used: u64,
    /// 根分区。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub disk: Option<Disk>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub net: Option<Net>,
    /// 开机时长，秒。
    pub uptime: u64,
    /// 按 CPU 和按内存各自最高的 TOP 个进程合在一起，按内存从高到低排列。
    pub procs: Vec<Proc>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum MonitorEvent {
    Stats(Stats),
    /// 服务器不是 Linux，读不到 /proc；一定是最后一条。
    Unsupported,
    /// 这次采集失败，下个周期会再试。
    Failed { error: Msg },
}

/// 监控数据的去处，commands 层把它接到 tauri::ipc::Channel。
pub type MonitorSink = Box<dyn Fn(MonitorEvent) + Send + Sync>;

pub struct MonitorService {
    conns: Arc<ConnService>,
    pool: Arc<ClientPool>,
    monitors: Mutex<HashMap<String, CancellationToken>>,
}

impl MonitorService {
    pub fn new(conns: Arc<ConnService>, pool: Arc<ClientPool>) -> Arc<Self> {
        Arc::new(MonitorService {
            conns,
            pool,
            monitors: Mutex::default(),
        })
    }

    /// 在连接已建立的 SSH 连接上开始采集，返回监控 ID；连接没有建立时不会新建，直接报错。
    pub fn open(self: &Arc<Self>, conn_id: &str, sink: MonitorSink) -> ApiResult<String> {
        let conn = self.conns.resolve(conn_id)?;
        conn.ssh_params().map_err(|m| error::CONN_INVALID.msg(m))?;
        let client = self
            .pool
            .existing(&conn.id)
            .ok_or_else(|| error::MONITOR_NOT_CONNECTED.err())?;
        let id = new_id();
        let cancel = CancellationToken::new();
        self.monitors
            .lock()
            .unwrap()
            .insert(id.clone(), cancel.clone());
        debug!(conn = %conn.name, monitor = %id, "开始采集服务器状态");

        let (svc, mid, name) = (self.clone(), id.clone(), conn.name);
        tokio::spawn(async move {
            run(&client, &sink, &cancel).await;
            svc.monitors.lock().unwrap().remove(&mid);
            svc.pool.release(&client);
            debug!(conn = %name, monitor = %mid, "停止采集服务器状态");
        });
        Ok(id)
    }

    /// 停止采集；不再推送任何消息。
    pub fn close(&self, id: &str) {
        if let Some(c) = self.monitors.lock().unwrap().remove(id) {
            c.cancel();
        }
    }

    /// 停止全部采集：程序退出、前端页面重新加载时调用，之后应再关闭连接池。
    pub fn close_all(&self) {
        for c in std::mem::take(&mut *self.monitors.lock().unwrap()).values() {
            c.cancel();
        }
    }
}

/// 定时采集并推送，直到被关闭、SSH 连接断开或确认服务器不支持。
async fn run(client: &Client, sink: &MonitorSink, cancel: &CancellationToken) {
    let mut tracker = Tracker::default();
    let mut wait = FIRST_INTERVAL;
    loop {
        let sample = client.ssh.exec_output("sh", SCRIPT.as_bytes(), MAX_OUTPUT);
        let r = tokio::select! {
            _ = cancel.cancelled() => return,
            _ = client.dead.cancelled() => return,
            r = tokio::time::timeout(SAMPLE_TIMEOUT, sample) => r,
        };
        let ev = match r {
            Err(_) => MonitorEvent::Failed {
                error: msg!("err.monitor.timeout", "服务器响应超时"),
            },
            Ok(Err(e)) => MonitorEvent::Failed { error: e.msg() },
            Ok(Ok(out)) => match parse(&String::from_utf8_lossy(&out)) {
                Ok(s) => MonitorEvent::Stats(tracker.update(s)),
                Err(Bad::Unsupported) => {
                    if !cancel.is_cancelled() {
                        sink(MonitorEvent::Unsupported);
                    }
                    return;
                }
                Err(Bad::Invalid) => MonitorEvent::Failed {
                    error: msg!("err.monitor.badOutput", "无法读取服务器状态"),
                },
            },
        };
        if let MonitorEvent::Failed { error } = &ev {
            debug!(err = %error.text(), "采集服务器状态失败");
        }
        if cancel.is_cancelled() {
            return;
        }
        sink(ev);
        tokio::select! {
            _ = cancel.cancelled() => return,
            _ = client.dead.cancelled() => return,
            _ = tokio::time::sleep(wait) => {}
        }
        wait = INTERVAL;
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct CpuTimes {
    /// 全部核累计的时钟滴答。
    total: u64,
    /// 其中空闲和等待 IO 的部分。
    idle: u64,
}

#[derive(Debug, Clone, PartialEq)]
struct RawProc {
    pid: u32,
    ppid: u32,
    /// 用户态加内核态累计的时钟滴答。
    ticks: u64,
    /// 常驻内存页数。
    rss: u64,
    name: String,
}

/// 一次采集的原始数据。
#[derive(Debug, Clone, Default)]
struct Sample {
    /// 执行采集脚本的 sh，它和它的子进程不算在进程列表里。
    self_pid: u32,
    page_size: u64,
    cpu: CpuTimes,
    cores: u32,
    load: [f64; 3],
    mem_total: u64,
    mem_available: u64,
    swap_total: u64,
    swap_free: u64,
    uptime: f64,
    /// 累计收发的字节数。
    net: Option<(u64, u64)>,
    disk: Option<Disk>,
    procs: Vec<RawProc>,
}

#[derive(Debug, PartialEq, Eq)]
enum Bad {
    Unsupported,
    Invalid,
}

fn parse(text: &str) -> Result<Sample, Bad> {
    let mut s = Sample {
        page_size: 4096,
        ..Default::default()
    };
    let mut section = "";
    let mut cpu_seen = false;
    let mut mem: HashMap<&str, u64> = HashMap::new();
    let mut ifaces: Vec<(&str, u64, u64)> = Vec::new();
    let mut phys: HashSet<&str> = HashSet::new();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix('#') {
            let (name, value) = rest.split_once(' ').unwrap_or((rest, ""));
            match name {
                "unsupported" => return Err(Bad::Unsupported),
                "self" => s.self_pid = value.trim().parse().unwrap_or(0),
                "page" => {
                    if let Ok(n) = value.trim().parse::<u64>()
                        && n > 0
                    {
                        s.page_size = n;
                    }
                }
                _ => section = name,
            }
            continue;
        }
        match section {
            "cpu" => {
                let mut f = line.split_whitespace();
                match f.next() {
                    Some("cpu") => {
                        let v: Vec<u64> = f.take(8).filter_map(|x| x.parse().ok()).collect();
                        if v.len() >= 4 {
                            cpu_seen = true;
                            s.cpu = CpuTimes {
                                total: v.iter().sum(),
                                idle: v[3] + v.get(4).copied().unwrap_or(0),
                            };
                        }
                    }
                    Some(_) => s.cores += 1,
                    None => {}
                }
            }
            "load" => {
                for (i, v) in line.split_whitespace().take(3).enumerate() {
                    s.load[i] = v.parse().unwrap_or(0.0);
                }
            }
            "mem" => {
                if let Some((k, v)) = line.split_once(':')
                    && let Some(n) = v.split_whitespace().next().and_then(|n| n.parse().ok())
                {
                    mem.insert(k.trim(), n);
                }
            }
            "uptime" => {
                s.uptime = line
                    .split_whitespace()
                    .next()
                    .and_then(|v| v.parse().ok())
                    .unwrap_or(0.0);
            }
            "net" => {
                if let Some((name, rest)) = line.split_once(':') {
                    let f: Vec<u64> = rest
                        .split_whitespace()
                        .filter_map(|x| x.parse().ok())
                        .collect();
                    if f.len() >= 9 {
                        ifaces.push((name.trim(), f[0], f[8]));
                    }
                }
            }
            "phys" => {
                if let Some(name) = line
                    .strip_prefix("/sys/class/net/")
                    .and_then(|x| x.strip_suffix("/device"))
                {
                    phys.insert(name);
                }
            }
            "disk" => {
                let f: Vec<&str> = line.split_whitespace().collect();
                let kb = |i: usize| f.get(i).and_then(|x| x.parse::<u64>().ok()).map(|n| n * 1024);
                if f.len() >= 6
                    && let (Some(total), Some(used), Some(avail)) =
                        (kb(f.len() - 5), kb(f.len() - 4), kb(f.len() - 3))
                {
                    s.disk = Some(Disk { total, used, avail });
                }
            }
            "ps" => {
                if let Some(p) = parse_proc(line) {
                    s.procs.push(p);
                }
            }
            _ => {}
        }
    }

    let kb = |k: &str| mem.get(k).copied().unwrap_or(0) * 1024;
    s.mem_total = kb("MemTotal");
    if !cpu_seen || s.mem_total == 0 {
        return Err(Bad::Invalid);
    }
    s.mem_available = if mem.contains_key("MemAvailable") {
        kb("MemAvailable")
    } else {
        kb("MemFree") + kb("Buffers") + kb("Cached")
    };
    s.swap_total = kb("SwapTotal");
    s.swap_free = kb("SwapFree");
    s.cores = s.cores.max(1);
    // 有物理网卡时只算物理网卡，否则（如容器里）算除回环外的全部。
    let counted = |name: &str| {
        if phys.is_empty() {
            name != "lo"
        } else {
            phys.contains(name)
        }
    };
    if !ifaces.is_empty() {
        s.net = Some(
            ifaces
                .iter()
                .filter(|(name, ..)| counted(name))
                .fold((0, 0), |(rx, tx), (_, r, t)| (rx + r, tx + t)),
        );
    }
    Ok(s)
}

fn parse_proc(line: &str) -> Option<RawProc> {
    let mut f = line.splitn(6, ' ');
    let mut num = || f.next()?.parse::<u64>().ok();
    let (pid, ppid, utime, stime, rss) = (num()?, num()?, num()?, num()?, num()?);
    let name = f.next().unwrap_or_default().to_string();
    Some(RawProc {
        pid: pid as u32,
        ppid: ppid as u32,
        ticks: utime + stime,
        rss,
        name,
    })
}

/// 上一次采集的累计值，用来算这一段时间里的占用。
struct Prev {
    cpu: CpuTimes,
    uptime: f64,
    net: Option<(u64, u64)>,
    ticks: HashMap<u32, u64>,
}

#[derive(Default)]
struct Tracker {
    prev: Option<Prev>,
}

impl Tracker {
    fn update(&mut self, s: Sample) -> Stats {
        let prev = self.prev.take();
        let delta = prev
            .as_ref()
            .map(|p| s.cpu.total.saturating_sub(p.cpu.total))
            .filter(|d| *d > 0);
        let cpu = match (&prev, delta) {
            (Some(p), Some(d)) => {
                let idle = s.cpu.idle.saturating_sub(p.cpu.idle).min(d);
                Some(((d - idle) as f64 / d as f64 * 100.0) as f32)
            }
            _ => None,
        };
        // 一个核在这段时间里的时钟滴答数，进程占满一个核时为 100%。
        let per_core = delta.map(|d| d as f64 / s.cores as f64);
        let procs = s
            .procs
            .iter()
            .filter(|p| s.self_pid == 0 || (p.pid != s.self_pid && p.ppid != s.self_pid))
            .map(|p| Proc {
                pid: p.pid,
                name: p.name.clone(),
                // 上一次没有的进程是这段时间里新启动的，累计值就是这段时间的占用。
                cpu: match (&prev, per_core) {
                    (Some(prev), Some(c)) => {
                        let before = prev.ticks.get(&p.pid).copied().unwrap_or(0);
                        Some((p.ticks.saturating_sub(before) as f64 / c * 100.0) as f32)
                    }
                    _ => None,
                },
                mem: p.rss * s.page_size,
            })
            .collect();
        let net = match (prev.as_ref().and_then(|p| p.net.map(|n| (n, p.uptime))), s.net) {
            (Some(((rx0, tx0), t0)), Some((rx, tx))) if s.uptime > t0 => {
                let dt = s.uptime - t0;
                Some(Net {
                    rx: (rx.saturating_sub(rx0) as f64 / dt) as u64,
                    tx: (tx.saturating_sub(tx0) as f64 / dt) as u64,
                })
            }
            _ => None,
        };
        self.prev = Some(Prev {
            cpu: s.cpu,
            uptime: s.uptime,
            net: s.net,
            ticks: s.procs.iter().map(|p| (p.pid, p.ticks)).collect(),
        });
        Stats {
            cpu,
            cores: s.cores,
            load: s.load,
            mem_total: s.mem_total,
            mem_used: s.mem_total.saturating_sub(s.mem_available),
            swap_total: s.swap_total,
            swap_used: s.swap_total.saturating_sub(s.swap_free),
            disk: s.disk,
            net,
            uptime: s.uptime as u64,
            procs: top(procs),
        }
    }
}

/// 按 CPU 和按内存各取最高的 TOP 个，合在一起按内存从高到低排列。
fn top(mut all: Vec<Proc>) -> Vec<Proc> {
    let cpu = |p: &Proc| p.cpu.unwrap_or(-1.0);
    all.sort_by(|a, b| cpu(b).total_cmp(&cpu(a)));
    let mut keep: HashSet<u32> = all
        .iter()
        .take(TOP)
        .filter(|p| p.cpu.is_some())
        .map(|p| p.pid)
        .collect();
    all.sort_by(|a, b| b.mem.cmp(&a.mem));
    keep.extend(all.iter().take(TOP).map(|p| p.pid));
    all.retain(|p| keep.contains(&p.pid));
    all
}

#[cfg(test)]
mod tests {
    use super::*;

    fn output(cpu: &str, uptime: &str, rx: u64, procs: &str) -> String {
        format!(
            "#self 900\n#page 4096\n#cpu\n{cpu}\ncpu0 1 0 1 1 0 0 0 0 0 0\ncpu1 1 0 1 1 0 0 0 0 0 0\n\
             #load\n0.52 0.58 0.59 2/331 1234\n\
             #mem\nMemTotal:        8000000 kB\nMemFree:         1000000 kB\nMemAvailable:    6000000 kB\n\
             SwapTotal:       2000000 kB\nSwapFree:        1500000 kB\n\
             #uptime\n{uptime} 7000.00\n\
             #net\nInter-|   Receive\n face |bytes\n    lo: 999 1 0 0 0 0 0 0 999 1 0 0 0 0 0 0\n\
             \x20 eth0: {rx} 10 0 0 0 0 0 0 500 5 0 0 0 0 0 0\n docker0:7777 1 0 0 0 0 0 0 7777 1 0 0 0 0 0 0\n\
             #phys\n/sys/class/net/eth0/device\n\
             #disk\nFilesystem     1024-blocks    Used Available Capacity Mounted on\n\
             /dev/sda1         100000   40000     55000      43% /\n\
             #ps\n{procs}"
        )
    }

    #[test]
    fn parses_proc_output() {
        let text = output(
            "cpu  100 0 50 800 50 0 0 0 0 0",
            "3600.50",
            1000,
            "1 0 10 5 3000 systemd\n42 1 300 100 50000 my app (worker)\n900 1 0 0 100 sh\n901 900 1 0 200 awk\n",
        );
        let s = parse(&text).unwrap();
        assert_eq!(s.self_pid, 900);
        assert_eq!(s.cores, 2);
        assert_eq!(s.cpu, CpuTimes { total: 1000, idle: 850 });
        assert_eq!(s.load, [0.52, 0.58, 0.59]);
        assert_eq!(s.mem_total, 8_000_000 * 1024);
        assert_eq!(s.mem_available, 6_000_000 * 1024);
        assert_eq!(s.swap_free, 1_500_000 * 1024);
        assert_eq!(s.net, Some((1000, 500)), "只算物理网卡");
        assert_eq!(
            s.disk,
            Some(Disk {
                total: 100_000 * 1024,
                used: 40_000 * 1024,
                avail: 55_000 * 1024
            })
        );
        assert_eq!(s.procs.len(), 4);
        assert_eq!(s.procs[1].name, "my app (worker)");
        assert_eq!(s.procs[1].ticks, 400);
    }

    #[test]
    fn unsupported_and_invalid() {
        assert_eq!(parse("#unsupported\n").unwrap_err(), Bad::Unsupported);
        assert_eq!(parse("").unwrap_err(), Bad::Invalid);
        assert_eq!(parse("#cpu\ncpu  1 2 3 4\n").unwrap_err(), Bad::Invalid);
    }

    #[test]
    fn rates_between_samples() {
        let mut t = Tracker::default();
        let first = t.update(
            parse(&output(
                "cpu  100 0 50 800 50 0 0 0 0 0",
                "100.00",
                1000,
                "1 0 10 5 3000 systemd\n42 1 300 100 50000 app\n900 1 0 0 100 sh\n",
            ))
            .unwrap(),
        );
        assert_eq!(first.cpu, None);
        assert_eq!(first.net, None);
        assert!(first.procs.iter().all(|p| p.cpu.is_none()));
        assert_eq!(first.mem_used, 2_000_000 * 1024);
        assert_eq!(first.swap_used, 500_000 * 1024);

        // 两个核共 200 个滴答，其中空闲 50：CPU 75%；app 用了 100 个滴答，即一个核的 100%。
        let second = t.update(
            parse(&output(
                "cpu  220 0 80 840 60 0 0 0 0 0",
                "102.00",
                3000,
                "1 0 10 5 3000 systemd\n42 1 350 150 50000 app\n77 1 20 0 10 new\n900 1 0 0 100 sh\n",
            ))
            .unwrap(),
        );
        assert_eq!(second.cpu, Some(75.0));
        assert_eq!(second.net, Some(Net { rx: 1000, tx: 0 }));
        assert_eq!(second.uptime, 102);
        let cpu = |pid| second.procs.iter().find(|p| p.pid == pid).and_then(|p| p.cpu);
        assert_eq!(cpu(42), Some(100.0));
        assert_eq!(cpu(77), Some(20.0), "新启动的进程按累计值算");
        assert_eq!(cpu(1), Some(0.0));
        assert!(second.procs.iter().all(|p| p.pid != 900), "不含采集脚本自己");
        assert_eq!(second.procs[0].pid, 42, "按内存从高到低");
        assert_eq!(second.procs[0].mem, 50000 * 4096);
    }

    #[test]
    fn top_keeps_both_rankings() {
        let p = |pid, cpu, mem| Proc {
            pid,
            name: String::new(),
            cpu: Some(cpu),
            mem,
        };
        let n = TOP as u32 * 3;
        let all: Vec<Proc> = (0..n).map(|i| p(i, i as f32, (n - i) as u64)).collect();
        let pids: HashSet<u32> = top(all).iter().map(|p| p.pid).collect();
        let want: HashSet<u32> = (n - TOP as u32..n).chain(0..TOP as u32).collect();
        assert_eq!(pids, want, "CPU 最高的和内存最高的各 TOP 个");
    }

    #[test]
    fn event_json() {
        let ev = MonitorEvent::Failed {
            error: msg!("err.monitor.timeout", "服务器响应超时"),
        };
        assert_eq!(
            serde_json::to_value(&ev).unwrap(),
            serde_json::json!({ "type": "failed", "error": { "i18n": "err.monitor.timeout" } })
        );
        assert_eq!(
            serde_json::to_value(MonitorEvent::Unsupported).unwrap(),
            serde_json::json!({ "type": "unsupported" })
        );
    }
}
