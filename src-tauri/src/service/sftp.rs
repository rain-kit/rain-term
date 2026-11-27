//! SFTP 文件管理：每个面板在连接池的 SSH 连接上开一个 sftp channel；传输任务按面板排队，一次执行一个。

use std::collections::HashMap;
use std::future::Future;
use std::io;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use russh_sftp::client::SftpSession;
use russh_sftp::client::error::Error as SftpError;
use russh_sftp::protocol::{FileAttributes, FileType};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use super::conn::ConnService;
use super::pool::{Client, ClientPool};
use crate::core::ssh::SshClient;
use crate::core::ssh::sftp::{SudoError, describe, is_not_found};
use crate::error::{self, ApiError, ApiResult};
use crate::i18n::Msg;
use crate::model::{AuthType, SshParams};
use crate::util::new_id;

const BUF_SIZE: usize = 256 * 1024;
/// 传输进度最多每隔这么久推送一次。
pub const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);
/// 关闭面板时等服务器确认关闭 sftp channel 的最长时间。
const CLOSE_TIMEOUT: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum FileKind {
    Dir,
    File,
    Symlink,
    Other,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FileEntry {
    pub name: String,
    pub path: String,
    pub kind: FileKind,
    /// 符号链接指向的是目录。
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub link_dir: bool,
    pub size: u64,
    /// 修改时间，Unix 秒。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mtime: Option<u32>,
    /// 如 drwxr-xr-x；服务器没给权限时为空。
    pub perm: String,
}

impl FileEntry {
    fn is_dir_like(&self) -> bool {
        self.kind == FileKind::Dir || self.link_dir
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Listing {
    /// 规范化后的绝对路径。
    pub path: String,
    /// 目录在前，再按名称排序。
    pub entries: Vec<FileEntry>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Opened {
    pub id: String,
    /// 初始目录：连接设置了登录后初始目录时用它，否则是用户主目录。
    pub path: String,
    /// 是否用 sudo 以 root 身份打开。
    pub sudo: bool,
}

/// 打开面板时的选项。
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct OpenOptions {
    /// 是否用 sudo 以 root 身份打开；为 None 时按设置。
    pub sudo: Option<bool>,
    /// 用户输入的 sudo 密码，只用于这一次打开。
    pub sudo_password: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    Upload,
    Download,
}

/// 已加入队列的传输任务。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Transfer {
    pub id: String,
    pub direction: Direction,
    pub name: String,
    pub src: String,
    pub dst: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum SftpEvent {
    /// 任务开始执行，total 为总字节数（目录为其中全部文件之和）。
    Started {
        id: String,
        total: u64,
    },
    Progress {
        id: String,
        done: u64,
        total: u64,
    },
    Done {
        id: String,
    },
    Failed {
        id: String,
        error: Msg,
    },
    Cancelled {
        id: String,
    },
    /// SSH 连接断开，面板已不可用；一定是最后一条。用户主动关闭面板时不发。
    Closed {
        error: Msg,
    },
}

/// 面板事件的去处，commands 层把它接到 tauri::ipc::Channel。
pub type SftpSink = Box<dyn Fn(SftpEvent) + Send + Sync>;

/// 面板关闭后不再推送任何事件。
struct Sink {
    f: SftpSink,
    finished: AtomicBool,
}

impl Sink {
    fn send(&self, ev: SftpEvent) {
        if !self.finished.load(Ordering::SeqCst) {
            (self.f)(ev);
        }
    }

    fn finish(&self, last: Option<SftpEvent>) {
        if !self.finished.swap(true, Ordering::SeqCst)
            && let Some(ev) = last
        {
            (self.f)(ev);
        }
    }
}

struct Job {
    id: String,
    direction: Direction,
    src: String,
    dst: String,
    cancel: CancellationToken,
}

struct Panel {
    conn_name: String,
    client: Arc<Client>,
    sftp: Arc<SftpSession>,
    sink: Arc<Sink>,
    jobs: mpsc::UnboundedSender<Job>,
    /// 排队中和执行中的任务。
    tasks: Mutex<HashMap<String, CancellationToken>>,
    closed: CancellationToken,
}

pub struct SftpService {
    conns: Arc<ConnService>,
    pool: Arc<ClientPool>,
    panels: Mutex<HashMap<String, Arc<Panel>>>,
}

impl SftpService {
    pub fn new(conns: Arc<ConnService>, pool: Arc<ClientPool>) -> Arc<Self> {
        Arc::new(SftpService {
            conns,
            pool,
            panels: Mutex::default(),
        })
    }

    /// 打开 SFTP 面板；传输进度和断开通知从 sink 推出。
    /// sudo 为 None 时按连接的设置，连接没有单独设置时按 global_sudo（顶栏“工具”里的设置）。
    pub async fn open(
        self: &Arc<Self>,
        conn_id: &str,
        opts: OpenOptions,
        global_sudo: bool,
        sink: SftpSink,
    ) -> ApiResult<Opened> {
        let conn = self.conns.resolve(conn_id)?;
        let p = conn.ssh_params().map_err(|m| error::CONN_INVALID.msg(m))?;
        let sudo = opts.sudo.unwrap_or_else(|| p.sftp_sudo.enabled(global_sudo));

        // 复用的连接可能刚断开、还没被发现，这时重新连接再试一次。
        let mut retry = true;
        let (client, sftp) = loop {
            let (client, reused) = self.pool.acquire(&conn, p).await?;
            let r = if sudo {
                open_sudo(&client.ssh, p, opts.sudo_password.as_deref()).await
            } else {
                client
                    .ssh
                    .open_sftp()
                    .await
                    .map_err(|e| error::SFTP_OPEN_FAILED.msg(e.msg()))
            };
            match r {
                Ok(s) => break (client, s),
                Err(e) => {
                    self.pool.release(&client);
                    if reused && retry && !client.alive() {
                        retry = false;
                        continue;
                    }
                    if e.code == error::SFTP_OPEN_FAILED.0 {
                        warn!(conn = %conn.name, sudo, err = %e.msg.text(), "打开 SFTP 失败");
                    }
                    return Err(e);
                }
            }
        };
        let sftp = Arc::new(sftp);
        let path = start_dir(&sftp, &p.init_dir).await;

        let id = new_id();
        let (jobs, rx) = mpsc::unbounded_channel();
        let panel = Arc::new(Panel {
            conn_name: conn.name.clone(),
            client,
            sftp,
            sink: Arc::new(Sink {
                f: sink,
                finished: AtomicBool::new(false),
            }),
            jobs,
            tasks: Mutex::default(),
            closed: CancellationToken::new(),
        });
        self.panels
            .lock()
            .unwrap()
            .insert(id.clone(), panel.clone());
        info!(conn = %conn.name, panel = %id, path = %path, sudo, "SFTP 已打开");

        tokio::spawn(worker(panel.clone(), rx, self.pool.clone()));
        let (svc, pid) = (self.clone(), id.clone());
        tokio::spawn(async move {
            tokio::select! {
                _ = panel.closed.cancelled() => {}
                _ = panel.client.dead.cancelled() => {
                    if svc.take(&pid).is_some() {
                        let error = panel.client.reason().unwrap_or_else(|| msg!("err.ssh.connectionLost", "连接断开"));
                        warn!(conn = %panel.conn_name, panel = %pid, err = %error.text(), "SFTP 连接断开");
                        panel.sink.finish(Some(SftpEvent::Closed { error }));
                    }
                }
            }
        });
        Ok(Opened { id, path, sudo })
    }

    fn panel(&self, id: &str) -> ApiResult<Arc<Panel>> {
        self.panels
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .ok_or_else(|| error::SFTP_NOT_FOUND.err())
    }

    /// 从列表移除并停止全部任务；SSH 连接在传输任务收尾后由 worker 释放。
    fn take(&self, id: &str) -> Option<Arc<Panel>> {
        let p = self.panels.lock().unwrap().remove(id)?;
        p.closed.cancel();
        for t in p.tasks.lock().unwrap().values() {
            t.cancel();
        }
        Some(p)
    }

    /// 列出目录；path 可以是相对路径或带 ..，返回规范化后的路径。
    pub async fn list(&self, id: &str, path: &str) -> ApiResult<Listing> {
        let p = self.panel(id)?;
        let path = p
            .sftp
            .canonicalize(path)
            .await
            .map_err(|e| failed(path, &e))?;
        let mut entries = Vec::new();
        for e in p
            .sftp
            .read_dir(path.as_str())
            .await
            .map_err(|e| failed(&path, &e))?
        {
            let meta = e.metadata();
            let full = e.path();
            let kind = kind_of(&meta);
            let link_dir = kind == FileKind::Symlink
                && p.sftp
                    .metadata(full.as_str())
                    .await
                    .is_ok_and(|m| m.file_type().is_dir());
            entries.push(FileEntry {
                name: e.file_name(),
                path: full,
                kind,
                link_dir,
                size: meta.len(),
                mtime: meta.mtime,
                perm: perm_string(&meta),
            });
        }
        entries.sort_by(|a, b| {
            b.is_dir_like()
                .cmp(&a.is_dir_like())
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });
        Ok(Listing { path, entries })
    }

    pub async fn mkdir(&self, id: &str, path: &str) -> ApiResult<()> {
        let p = self.panel(id)?;
        p.sftp
            .create_dir(path)
            .await
            .map_err(|e| failed(path, &e))?;
        info!(conn = %p.conn_name, path, "已新建目录");
        Ok(())
    }

    pub async fn rename(&self, id: &str, from: &str, to: &str) -> ApiResult<()> {
        let p = self.panel(id)?;
        p.sftp
            .rename(from, to)
            .await
            .map_err(|e| failed(from, &e))?;
        info!(conn = %p.conn_name, from, to, "已重命名");
        Ok(())
    }

    /// 删除文件或目录，目录连同其中的内容一起删除。
    pub async fn remove(&self, id: &str, paths: &[String]) -> ApiResult<()> {
        let p = self.panel(id)?;
        for path in paths {
            remove_tree(&p.sftp, path)
                .await
                .map_err(|(at, e)| failed(&at, &e))?;
            info!(conn = %p.conn_name, path = %path, "已删除");
        }
        Ok(())
    }

    /// 把本地文件或目录上传到远程目录 remote_dir，同名文件会被覆盖。
    pub fn upload(
        &self,
        id: &str,
        local_paths: &[String],
        remote_dir: &str,
    ) -> ApiResult<Vec<Transfer>> {
        let p = self.panel(id)?;
        let list = local_paths
            .iter()
            .map(|src| {
                let name = Path::new(src)
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| src.clone());
                let dst = join_remote(remote_dir, &name);
                enqueue(&p, Direction::Upload, name, src.clone(), dst)
            })
            .collect();
        Ok(list)
    }

    /// 把远程文件或目录下载到本地目录 local_dir，同名文件会被覆盖。
    pub fn download(
        &self,
        id: &str,
        remote_paths: &[String],
        local_dir: &str,
    ) -> ApiResult<Vec<Transfer>> {
        let p = self.panel(id)?;
        let list = remote_paths
            .iter()
            .map(|src| {
                let name = remote_name(src).to_string();
                let dst = Path::new(local_dir)
                    .join(local_name(&name))
                    .display()
                    .to_string();
                enqueue(&p, Direction::Download, name, src.clone(), dst)
            })
            .collect();
        Ok(list)
    }

    /// 取消排队中或执行中的任务；任务已结束时什么也不做。
    pub fn cancel(&self, id: &str, task_id: &str) -> ApiResult<()> {
        let p = self.panel(id)?;
        if let Some(t) = p.tasks.lock().unwrap().get(task_id) {
            t.cancel();
        }
        Ok(())
    }

    /// 关闭面板，取消它的全部传输任务；不再推送任何事件。
    pub fn close(&self, id: &str) {
        if let Some(p) = self.take(id) {
            p.sink.finish(None);
            info!(conn = %p.conn_name, panel = %id, "SFTP 已关闭");
        }
    }

    /// 关闭全部面板：程序退出、前端页面重新加载时调用，之后应再关闭连接池。
    pub fn close_all(&self) {
        let ids: Vec<String> = self.panels.lock().unwrap().keys().cloned().collect();
        for id in ids {
            self.close(&id);
        }
    }

    pub fn panel_count(&self) -> usize {
        self.panels.lock().unwrap().len()
    }
}

fn enqueue(p: &Panel, direction: Direction, name: String, src: String, dst: String) -> Transfer {
    let id = new_id();
    let cancel = CancellationToken::new();
    p.tasks.lock().unwrap().insert(id.clone(), cancel.clone());
    let _ = p.jobs.send(Job {
        id: id.clone(),
        direction,
        src: src.clone(),
        dst: dst.clone(),
        cancel,
    });
    Transfer {
        id,
        direction,
        name,
        src,
        dst,
    }
}

/// 用 sudo 打开 SFTP：先试免密 sudo，需要密码时用 password；没有提供时再试登录密码（Ubuntu 等系统上 sudo 密码通常就是登录密码）。
async fn open_sudo(ssh: &SshClient, p: &SshParams, password: Option<&str>) -> ApiResult<SftpSession> {
    let fail = |e: SudoError| match e {
        SudoError::PasswordRequired => error::SFTP_SUDO_PASSWORD_REQUIRED.err(),
        SudoError::WrongPassword => error::SFTP_SUDO_WRONG_PASSWORD.err(),
        SudoError::Ssh(e) => error::SFTP_OPEN_FAILED.msg(e.msg()),
    };
    match ssh.open_sftp_sudo(None).await {
        Err(SudoError::PasswordRequired) => {}
        r => return r.map_err(fail),
    }
    if let Some(pw) = password {
        return ssh.open_sftp_sudo(Some(pw)).await.map_err(fail);
    }
    if p.auth == AuthType::Password && !p.password.is_empty() {
        match ssh.open_sftp_sudo(Some(&p.password)).await {
            Err(SudoError::WrongPassword) => {}
            r => return r.map_err(fail),
        }
    }
    Err(error::SFTP_SUDO_PASSWORD_REQUIRED.err())
}

fn failed(path: &str, e: &SftpError) -> ApiError {
    error::SFTP_FAILED.msg(on_path(path, describe(e)))
}

fn on_path(path: &str, reason: Msg) -> Msg {
    msg!("err.sftp.onPath", "{path}：{reason}", path, reason)
}

/// 面板打开时进入的目录。
async fn start_dir(sftp: &SftpSession, init_dir: &str) -> String {
    let home = sftp
        .canonicalize(".")
        .await
        .unwrap_or_else(|_| "/".to_string());
    let want = match init_dir {
        "" | "~" => return home,
        d => d
            .strip_prefix("~/")
            .map(|rest| join_remote(&home, rest))
            .unwrap_or_else(|| d.to_string()),
    };
    match sftp.canonicalize(want).await {
        Ok(p)
            if sftp
                .metadata(p.as_str())
                .await
                .is_ok_and(|m| m.file_type().is_dir()) =>
        {
            p
        }
        _ => home,
    }
}

pub fn join_remote(dir: &str, name: &str) -> String {
    if dir.ends_with('/') {
        format!("{dir}{name}")
    } else {
        format!("{dir}/{name}")
    }
}

fn remote_name(path: &str) -> &str {
    path.trim_end_matches('/')
        .rsplit('/')
        .next()
        .filter(|n| !n.is_empty())
        .unwrap_or("_")
}

/// 远程文件名里本地文件系统不允许的字符换成下划线。
fn local_name(name: &str) -> String {
    let bad = |c: char| {
        c.is_control()
            || c == '/'
            || c == '\\'
            || (cfg!(windows) && matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*'))
    };
    let s: String = name.chars().map(|c| if bad(c) { '_' } else { c }).collect();
    let s = if cfg!(windows) {
        s.trim_end_matches([' ', '.']).to_string()
    } else {
        s
    };
    if s.is_empty() || s == "." || s == ".." {
        "_".to_string()
    } else {
        s
    }
}

fn kind_of(m: &FileAttributes) -> FileKind {
    match m.file_type() {
        FileType::Dir => FileKind::Dir,
        FileType::File => FileKind::File,
        FileType::Symlink => FileKind::Symlink,
        FileType::Other => FileKind::Other,
    }
}

fn perm_string(m: &FileAttributes) -> String {
    let Some(mode) = m.permissions else {
        return String::new();
    };
    let t = match mode & 0o170000 {
        0o040000 => 'd',
        0o120000 => 'l',
        0o100000 => '-',
        0o020000 => 'c',
        0o060000 => 'b',
        0o010000 => 'p',
        0o140000 => 's',
        _ => '?',
    };
    let mut s = String::with_capacity(10);
    s.push(t);
    for shift in [6, 3, 0] {
        let b = (mode >> shift) & 7;
        s.push(if b & 4 != 0 { 'r' } else { '-' });
        s.push(if b & 2 != 0 { 'w' } else { '-' });
        s.push(if b & 1 != 0 { 'x' } else { '-' });
    }
    s
}

type Fut<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// 删除文件或目录（递归），失败时返回出错的路径。符号链接只删链接本身。
fn remove_tree<'a>(
    sftp: &'a SftpSession,
    path: &'a str,
) -> Fut<'a, Result<(), (String, SftpError)>> {
    Box::pin(async move {
        let at = |e| (path.to_string(), e);
        let meta = sftp.symlink_metadata(path).await.map_err(at)?;
        if meta.file_type().is_dir() {
            for e in sftp.read_dir(path).await.map_err(at)? {
                remove_tree(sftp, &e.path()).await?;
            }
            sftp.remove_dir(path).await.map_err(at)
        } else {
            sftp.remove_file(path).await.map_err(at)
        }
    })
}

/// 传输中断时要删掉的未写完的文件。
enum Partial {
    Local(PathBuf),
    Remote(String),
}

/// 一个传输任务的执行状态。
struct Ctx {
    sftp: Arc<SftpSession>,
    sink: Arc<Sink>,
    id: String,
    done: u64,
    total: u64,
    last: Instant,
    partial: Option<Partial>,
}

impl Ctx {
    fn started(&mut self) {
        self.last = Instant::now();
        self.sink.send(SftpEvent::Started {
            id: self.id.clone(),
            total: self.total,
        });
    }

    fn advance(&mut self, n: u64) {
        self.done += n;
        if self.last.elapsed() >= PROGRESS_INTERVAL {
            self.last = Instant::now();
            self.sink.send(SftpEvent::Progress {
                id: self.id.clone(),
                done: self.done,
                total: self.total.max(self.done),
            });
        }
    }
}

fn local_err(path: &Path, e: io::Error) -> Msg {
    on_path(
        &path.display().to_string(),
        msg!("err.sftp.local", "本地文件读写失败").with_cause(e),
    )
}

fn remote_err(path: &str, e: &SftpError) -> Msg {
    on_path(path, describe(e))
}

fn remote_io_err(path: &str, e: io::Error) -> Msg {
    on_path(path, msg!("err.sftp.transfer", "传输失败").with_cause(e))
}

/// 逐个执行面板的传输任务；面板关闭后收尾、关闭 sftp channel 并释放 SSH 连接。
async fn worker(panel: Arc<Panel>, mut rx: mpsc::UnboundedReceiver<Job>, pool: Arc<ClientPool>) {
    loop {
        let job = tokio::select! {
            biased;
            _ = panel.closed.cancelled() => break,
            j = rx.recv() => match j {
                Some(j) => j,
                None => break,
            },
        };
        let id = job.id.clone();
        let ev = if job.cancel.is_cancelled() {
            SftpEvent::Cancelled { id: id.clone() }
        } else {
            let mut ctx = Ctx {
                sftp: panel.sftp.clone(),
                sink: panel.sink.clone(),
                id: id.clone(),
                done: 0,
                total: 0,
                last: Instant::now(),
                partial: None,
            };
            let start = Instant::now();
            let r = tokio::select! {
                biased;
                _ = job.cancel.cancelled() => Err(None),
                r = run(&mut ctx, &job) => r.map_err(Some),
            };
            if r.is_err()
                && let Some(partial) = ctx.partial.take()
            {
                match partial {
                    Partial::Local(p) => drop(tokio::fs::remove_file(p).await),
                    Partial::Remote(p) => drop(panel.sftp.remove_file(p).await),
                }
            }
            let what = match job.direction {
                Direction::Upload => "上传",
                Direction::Download => "下载",
            };
            match r {
                Ok(()) => {
                    let ms = start.elapsed().as_millis() as u64;
                    info!(conn = %panel.conn_name, src = %job.src, dst = %job.dst, bytes = ctx.done, ms, "{what}完成");
                    SftpEvent::Done { id: id.clone() }
                }
                Err(None) => {
                    info!(conn = %panel.conn_name, src = %job.src, "{what}已取消");
                    SftpEvent::Cancelled { id: id.clone() }
                }
                Err(Some(error)) => {
                    warn!(conn = %panel.conn_name, src = %job.src, err = %error.text(), "{what}失败");
                    SftpEvent::Failed {
                        id: id.clone(),
                        error,
                    }
                }
            }
        };
        panel.tasks.lock().unwrap().remove(&id);
        panel.sink.send(ev);
    }
    let _ = tokio::time::timeout(CLOSE_TIMEOUT, panel.sftp.close()).await;
    pool.release(&panel.client);
}

async fn run(ctx: &mut Ctx, job: &Job) -> Result<(), Msg> {
    match job.direction {
        Direction::Upload => {
            let src = Path::new(&job.src);
            ctx.total = local_size(src, true).await.map_err(|e| local_err(src, e))?;
            ctx.started();
            upload_tree(ctx, src, &job.dst, true).await
        }
        Direction::Download => {
            let meta = ctx
                .sftp
                .metadata(job.src.as_str())
                .await
                .map_err(|e| remote_err(&job.src, &e))?;
            let is_dir = meta.file_type().is_dir();
            ctx.total = if is_dir {
                remote_size(&ctx.sftp, &job.src).await?
            } else {
                meta.len()
            };
            ctx.started();
            download_tree(ctx, &job.src, is_dir, Path::new(&job.dst)).await
        }
    }
}

/// 本地文件信息：符号链接取它指向的文件；目录里指向目录的符号链接跳过（返回 None），避免循环。
async fn local_meta(path: &Path, top: bool) -> io::Result<Option<std::fs::Metadata>> {
    let m = tokio::fs::symlink_metadata(path).await?;
    if !m.file_type().is_symlink() {
        return Ok(Some(m));
    }
    let target = tokio::fs::metadata(path).await?;
    Ok((top || !target.is_dir()).then_some(target))
}

fn local_size<'a>(path: &'a Path, top: bool) -> Fut<'a, io::Result<u64>> {
    Box::pin(async move {
        let Some(m) = local_meta(path, top).await? else {
            return Ok(0);
        };
        if !m.is_dir() {
            return Ok(if m.is_file() { m.len() } else { 0 });
        }
        let mut total = 0;
        let mut rd = tokio::fs::read_dir(path).await?;
        while let Some(e) = rd.next_entry().await? {
            total += local_size(&e.path(), false).await?;
        }
        Ok(total)
    })
}

fn remote_size<'a>(sftp: &'a SftpSession, path: &'a str) -> Fut<'a, Result<u64, Msg>> {
    Box::pin(async move {
        let mut total = 0;
        for e in sftp
            .read_dir(path)
            .await
            .map_err(|e| remote_err(path, &e))?
        {
            total += match e.file_type() {
                FileType::Dir => remote_size(sftp, &e.path()).await?,
                FileType::File => e.metadata().len(),
                FileType::Symlink => match sftp.metadata(e.path()).await {
                    Ok(m) if m.file_type().is_file() => m.len(),
                    _ => 0,
                },
                FileType::Other => 0,
            };
        }
        Ok(total)
    })
}

/// 远程目录不存在时新建；已存在同名文件时报错。
async fn ensure_remote_dir(sftp: &SftpSession, path: &str) -> Result<(), Msg> {
    match sftp.metadata(path).await {
        Ok(m) if m.file_type().is_dir() => Ok(()),
        Ok(_) => Err(on_path(path, msg!("err.sftp.notDir", "已存在同名文件"))),
        Err(e) if is_not_found(&e) => sftp
            .create_dir(path)
            .await
            .map_err(|e| remote_err(path, &e)),
        Err(e) => Err(remote_err(path, &e)),
    }
}

fn upload_tree<'a>(
    ctx: &'a mut Ctx,
    local: &'a Path,
    remote: &'a str,
    top: bool,
) -> Fut<'a, Result<(), Msg>> {
    Box::pin(async move {
        let Some(m) = local_meta(local, top)
            .await
            .map_err(|e| local_err(local, e))?
        else {
            return Ok(());
        };
        if m.is_dir() {
            ensure_remote_dir(&ctx.sftp, remote).await?;
            let mut rd = tokio::fs::read_dir(local)
                .await
                .map_err(|e| local_err(local, e))?;
            while let Some(e) = rd.next_entry().await.map_err(|e| local_err(local, e))? {
                let name = e.file_name().to_string_lossy().to_string();
                upload_tree(ctx, &e.path(), &join_remote(remote, &name), false).await?;
            }
            Ok(())
        } else if m.is_file() {
            upload_file(ctx, local, remote).await
        } else {
            Ok(())
        }
    })
}

async fn upload_file(ctx: &mut Ctx, local: &Path, remote: &str) -> Result<(), Msg> {
    let mut src = tokio::fs::File::open(local)
        .await
        .map_err(|e| local_err(local, e))?;
    let mut dst = ctx
        .sftp
        .create(remote)
        .await
        .map_err(|e| remote_err(remote, &e))?;
    ctx.partial = Some(Partial::Remote(remote.to_string()));
    let mut buf = vec![0u8; BUF_SIZE];
    loop {
        let n = src.read(&mut buf).await.map_err(|e| local_err(local, e))?;
        if n == 0 {
            break;
        }
        dst.write_all(&buf[..n])
            .await
            .map_err(|e| remote_io_err(remote, e))?;
        ctx.advance(n as u64);
    }
    dst.shutdown().await.map_err(|e| remote_io_err(remote, e))?;
    ctx.partial = None;
    Ok(())
}

fn download_tree<'a>(
    ctx: &'a mut Ctx,
    remote: &'a str,
    is_dir: bool,
    local: &'a Path,
) -> Fut<'a, Result<(), Msg>> {
    Box::pin(async move {
        if !is_dir {
            return download_file(ctx, remote, local).await;
        }
        tokio::fs::create_dir_all(local)
            .await
            .map_err(|e| local_err(local, e))?;
        for e in ctx
            .sftp
            .read_dir(remote)
            .await
            .map_err(|e| remote_err(remote, &e))?
        {
            let (path, child) = (e.path(), local.join(local_name(&e.file_name())));
            match e.file_type() {
                FileType::Dir => download_tree(ctx, &path, true, &child).await?,
                FileType::File => download_file(ctx, &path, &child).await?,
                FileType::Symlink
                    if ctx
                        .sftp
                        .metadata(path.as_str())
                        .await
                        .is_ok_and(|m| m.file_type().is_file()) =>
                {
                    download_file(ctx, &path, &child).await?
                }
                _ => {}
            }
        }
        Ok(())
    })
}

async fn download_file(ctx: &mut Ctx, remote: &str, local: &Path) -> Result<(), Msg> {
    let mut src = ctx
        .sftp
        .open(remote)
        .await
        .map_err(|e| remote_err(remote, &e))?;
    let mut dst = tokio::fs::File::create(local)
        .await
        .map_err(|e| local_err(local, e))?;
    ctx.partial = Some(Partial::Local(local.to_path_buf()));
    let mut buf = vec![0u8; BUF_SIZE];
    loop {
        let n = src
            .read(&mut buf)
            .await
            .map_err(|e| remote_io_err(remote, e))?;
        if n == 0 {
            break;
        }
        dst.write_all(&buf[..n])
            .await
            .map_err(|e| local_err(local, e))?;
        ctx.advance(n as u64);
    }
    dst.flush().await.map_err(|e| local_err(local, e))?;
    drop(dst);
    let _ = src.close().await;
    ctx.partial = None;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_paths() {
        assert_eq!(join_remote("/", "a"), "/a");
        assert_eq!(join_remote("/home/u", "a b"), "/home/u/a b");
        assert_eq!(remote_name("/var/log/"), "log");
        assert_eq!(remote_name("/"), "_");
        assert_eq!(local_name("a/b\u{1}c"), "a_b_c");
        assert_eq!(local_name(".."), "_");
        if cfg!(windows) {
            assert_eq!(local_name("x:y*?. "), "x_y__");
        }
    }

    #[test]
    fn perm() {
        let m = |p| FileAttributes {
            permissions: Some(p),
            ..Default::default()
        };
        assert_eq!(perm_string(&m(0o040755)), "drwxr-xr-x");
        assert_eq!(perm_string(&m(0o100640)), "-rw-r-----");
        assert_eq!(perm_string(&m(0o120777)), "lrwxrwxrwx");
        assert_eq!(perm_string(&FileAttributes::default()), "");
    }

    #[test]
    fn event_json() {
        let ev = SftpEvent::Progress {
            id: "t".into(),
            done: 1,
            total: 2,
        };
        assert_eq!(
            serde_json::to_value(&ev).unwrap(),
            serde_json::json!({ "type": "progress", "id": "t", "done": 1, "total": 2 })
        );
    }
}
