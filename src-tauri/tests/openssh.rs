//! 在 docker 里的真实 OpenSSH（tests/docker）上做集成测试：三种认证、跳板机、SFTP 传输，
//! 以及 vim、top、htop 的显示。需要 docker compose v2；本机没有 docker 或设置了
//! RAIN_DIAL_SKIP_DOCKER 时跳过。第一次运行要构建镜像，结束时删除容器。
//! 会修改本进程的 SSH_AUTH_SOCK，所以只有一个测试函数，各项按顺序执行。
mod common;

use std::io::Write as _;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rain_dial::core::rdp::cert::CertStore;
use rain_dial::core::ssh::known_hosts::HostKeyStore;
use rain_dial::core::ssh::shell::PtySize;
use rain_dial::events::NoopSink;
use rain_dial::model::{AuthType, ConnParams, Connection, SaveConnReq, SshParams};
use rain_dial::secrets::{MemoryBackend, Secrets};
use rain_dial::service::conn::ConnService;
use rain_dial::service::hostkey::HostKeyService;
use rain_dial::service::pool::ClientPool;
use rain_dial::service::session::{SessionEvent, SessionService};
use rain_dial::service::sftp::{FileKind, OpenOptions, SftpEvent, SftpService};
use rain_dial::store::ConnStore;
use russh::keys::ssh_key::LineEnding;
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

use common::{PASSWORD, USER, assert_same_tree, pattern, random_key, start_agent, temp_dir, write};

const COMPOSE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/docker/compose.yaml");
const WAIT: Duration = Duration::from_secs(20);
const HOME: &str = "/home/tester";

fn compose(args: &[&str]) -> Command {
    let mut c = Command::new("docker");
    c.args(["compose", "-f", COMPOSE]).args(args);
    c
}

fn docker_available() -> bool {
    let ok = |args: &[&str]| {
        Command::new("docker")
            .args(args)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    };
    ok(&["info"]) && ok(&["compose", "version"])
}

/// 跳板机和目标机两个容器；离开作用域时（包括测试失败）删除。
struct Containers {
    bastion: u16,
    target: u16,
}

impl Containers {
    fn up() -> Containers {
        let mut c = Containers {
            bastion: 0,
            target: 0,
        };
        let out = compose(&["up", "-d", "--build", "--wait"])
            .output()
            .expect("无法运行 docker compose");
        assert!(
            out.status.success(),
            "docker compose up 失败：{}",
            String::from_utf8_lossy(&out.stderr)
        );
        c.bastion = Self::port("bastion");
        c.target = Self::port("target");
        c
    }

    fn port(service: &str) -> u16 {
        let out = compose(&["port", service, "22"]).output().unwrap();
        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
        s.rsplit(':')
            .next()
            .and_then(|p| p.parse().ok())
            .unwrap_or_else(|| panic!("查询 {service} 的端口失败：{s:?}"))
    }

    /// 在容器里以 tester 身份执行 shell 脚本，返回标准输出。
    fn sh(&self, service: &str, script: &str) -> String {
        let mut child = compose(&[
            "exec", "-T", "-u", USER, "-w", HOME, service, "sh", "-c", script,
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
        drop(child.stdin.take());
        let out = child.wait_with_output().unwrap();
        assert!(
            out.status.success(),
            "在 {service} 里执行 {script:?} 失败：{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8(out.stdout).unwrap()
    }

    fn authorize(&self, public_key: &str) {
        let mut child = compose(&[
            "exec",
            "-T",
            "-u",
            USER,
            "target",
            "sh",
            "-c",
            "cat >> ~/.ssh/authorized_keys",
        ])
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
        writeln!(child.stdin.take().unwrap(), "{}", public_key.trim()).unwrap();
        assert!(child.wait().unwrap().success());
    }
}

impl Drop for Containers {
    fn drop(&mut self) {
        let _ = compose(&["down", "--timeout", "1"]).output();
    }
}

struct Fixture {
    conns: Arc<ConnService>,
    pool: Arc<ClientPool>,
    sessions: Arc<SessionService>,
    sftp: Arc<SftpService>,
    /// 询问过指纹的连接名，按顺序；全部自动信任。
    asked: Arc<Mutex<Vec<String>>>,
    answerer: tokio::task::JoinHandle<()>,
    dir: PathBuf,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.answerer.abort();
        self.sessions.close_all();
        self.sftp.close_all();
        self.pool.close_all();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn fixture() -> Fixture {
    let dir = temp_dir();
    let host_keys = HostKeyService::new(
        HostKeyStore::new(dir.join("known_hosts"), vec![]),
        CertStore::new(dir.join("rdp_known_hosts")),
        Arc::new(NoopSink),
    );
    let conns = Arc::new(ConnService::new(
        ConnStore::new(dir.join("connections.json")),
        Secrets::new(Box::new(MemoryBackend::default())),
        host_keys.clone(),
        Arc::new(NoopSink),
    ));
    let pool = ClientPool::new(conns.clone());
    let asked = Arc::new(Mutex::new(Vec::new()));
    let answerer = tokio::spawn({
        let asked = asked.clone();
        async move {
            loop {
                for p in host_keys.pending() {
                    asked.lock().unwrap().push(p.conn_name.clone());
                    host_keys.answer(&p.request_id, true).unwrap();
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }
    });
    Fixture {
        sessions: SessionService::new(conns.clone(), pool.clone()),
        sftp: SftpService::new(conns.clone(), pool.clone()),
        conns,
        pool,
        asked,
        answerer,
        dir,
    }
}

fn ssh(host: &str, port: u16) -> SshParams {
    SshParams {
        host: host.into(),
        port: port as i32,
        user: USER.into(),
        password: PASSWORD.into(),
        keepalive_seconds: 30,
        ..Default::default()
    }
}

impl Fixture {
    fn add(&self, name: &str, params: SshParams) -> String {
        let conn = Connection {
            name: name.into(),
            params: ConnParams::Ssh(params),
            ..Default::default()
        };
        self.conns
            .save(SaveConnReq {
                conn,
                ..Default::default()
            })
            .unwrap()
            .conn
            .id
    }

    /// 测试连接，成功返回服务器版本，失败返回错误的国际化 key。
    async fn test(&self, params: SshParams) -> Result<String, &'static str> {
        let conn = Connection {
            name: "测试".into(),
            params: ConnParams::Ssh(params),
            ..Default::default()
        };
        match self
            .conns
            .test(SaveConnReq {
                conn,
                ..Default::default()
            })
            .await
        {
            Ok(r) => Ok(r.server_version.unwrap_or_default()),
            Err(e) => Err(e.msg.i18n),
        }
    }

    async fn term(&self, conn_id: &str) -> Term {
        let (tx, rx) = unbounded_channel();
        let id = self
            .sessions
            .open(
                conn_id,
                PtySize { cols: 80, rows: 24 },
                Box::new(move |e| drop(tx.send(e))),
            )
            .await
            .unwrap();
        Term {
            id,
            rx,
            svc: self.sessions.clone(),
        }
    }

    async fn panel(&self, conn_id: &str) -> Panel {
        let (tx, rx) = unbounded_channel();
        let o = self
            .sftp
            .open(
                conn_id,
                OpenOptions::default(),
                false,
                Box::new(move |e| drop(tx.send(e))),
            )
            .await
            .unwrap();
        Panel {
            id: o.id,
            home: o.path,
            rx,
        }
    }

    fn asked(&self) -> Vec<String> {
        self.asked.lock().unwrap().clone()
    }
}

struct Term {
    id: String,
    rx: UnboundedReceiver<SessionEvent>,
    svc: Arc<SessionService>,
}

impl Term {
    async fn send(&self, data: &str) {
        self.svc.write(&self.id, data.as_bytes()).await.unwrap();
    }

    async fn resize(&self, cols: u32, rows: u32) {
        self.svc
            .resize(&self.id, PtySize { cols, rows })
            .await
            .unwrap();
    }

    /// 一直读到输出里出现 needle，返回读到的全部输出。
    async fn read_until(&mut self, needle: &str) -> String {
        let mut out = Vec::new();
        let r = tokio::time::timeout(WAIT, async {
            loop {
                match self.rx.recv().await {
                    Some(SessionEvent::Output(d)) => {
                        self.svc.ack(&self.id, d.len());
                        out.extend_from_slice(&d);
                        if String::from_utf8_lossy(&out).contains(needle) {
                            return;
                        }
                    }
                    other => panic!("等待 {needle:?} 时收到 {other:?}"),
                }
            }
        })
        .await;
        let text = String::from_utf8_lossy(&out).to_string();
        assert!(r.is_ok(), "等待 {needle:?} 超时，已收到 {text:?}");
        text
    }

    /// 执行命令并等它的结果；用算术展开做标记，命令回显里不会出现计算结果。
    async fn run(&mut self, cmd: &str) -> String {
        self.send(&format!("{cmd}; echo END-$((6*7))\r")).await;
        self.read_until("END-42").await
    }

    /// 按 key 退出全屏程序，等到 shell 重新显示提示符（bash 显示提示符时会打开 bracketed paste）。
    async fn quit(&mut self, key: &str) -> String {
        self.send(key).await;
        self.read_until("\x1b[?2004h").await
    }

    /// 执行命令，输出里应有 want。
    async fn expect(&mut self, cmd: &str, want: &str) {
        let out = self.run(cmd).await;
        assert!(out.contains(want), "{cmd} 的输出里没有 {want:?}：{out:?}");
    }
}

struct Panel {
    id: String,
    home: String,
    rx: UnboundedReceiver<SftpEvent>,
}

impl Panel {
    /// 等任务 id 结束，返回结束事件；其他事件忽略。
    async fn finish(&mut self, id: &str) -> SftpEvent {
        tokio::time::timeout(WAIT * 3, async {
            loop {
                let ev = self.rx.recv().await.expect("事件通道已关闭");
                match &ev {
                    SftpEvent::Done { id: i }
                    | SftpEvent::Failed { id: i, .. }
                    | SftpEvent::Cancelled { id: i }
                        if i == id =>
                    {
                        return ev;
                    }
                    SftpEvent::Closed { error } => {
                        panic!("等待任务结束时连接断开: {}", error.text())
                    }
                    _ => {}
                }
            }
        })
        .await
        .expect("等待传输结束超时")
    }
}

#[tokio::test]
async fn real_openssh() {
    if std::env::var_os("RAIN_DIAL_SKIP_DOCKER").is_some() || !docker_available() {
        eprintln!("跳过 OpenSSH 集成测试：没有可用的 docker（或设置了 RAIN_DIAL_SKIP_DOCKER）");
        return;
    }
    let c = Containers::up();
    password_shell_and_resize(&c).await;
    key_auth(&c).await;
    agent_auth(&c).await;
    jump_host(&c).await;
    sftp_transfer(&c).await;
    tui_apps(&c).await;
}

async fn password_shell_and_resize(c: &Containers) {
    let f = fixture();
    let version = f.test(ssh("127.0.0.1", c.target)).await.unwrap();
    assert!(version.starts_with("SSH-2.0-OpenSSH_"), "{version}");
    assert_eq!(
        f.test(SshParams {
            password: "wrong".into(),
            ..ssh("127.0.0.1", c.target)
        })
        .await,
        Err("err.ssh.authDenied")
    );

    let id = f.add("目标", ssh("127.0.0.1", c.target));
    let mut t = f.term(&id).await;
    t.expect("echo $TERM", "xterm-256color\r\n").await;
    t.expect("stty size", "24 80\r\n").await;
    t.resize(132, 40).await;
    t.expect("stty size", "40 132\r\n").await;
    t.expect("echo 你好🙂", "你好🙂\r\n").await;
    assert_eq!(f.asked(), ["测试"], "同一台服务器只询问一次指纹");
}

async fn key_auth(c: &Containers) {
    // 用 OpenSSH 自己的 ssh-keygen 生成各种格式的密钥。
    c.sh(
        "target",
        "rm -f /tmp/k_* && cd /tmp \
         && ssh-keygen -q -t ed25519 -N '' -f k_ed25519 \
         && ssh-keygen -q -t ecdsa -N '' -f k_ecdsa \
         && ssh-keygen -q -t rsa -b 3072 -N s3cret -f k_rsa \
         && ssh-keygen -q -t rsa -b 2048 -m PEM -N '' -f k_pem \
         && cat k_*.pub >> ~/.ssh/authorized_keys",
    );
    let f = fixture();
    let cases = [
        ("k_ed25519", ""),
        ("k_ecdsa", ""),
        ("k_rsa", "s3cret"),
        ("k_pem", ""),
    ];
    for (name, passphrase) in cases {
        let path = f.dir.join(name);
        std::fs::write(&path, c.sh("target", &format!("cat /tmp/{name}"))).unwrap();
        let params = SshParams {
            auth: AuthType::Key,
            key_path: path.display().to_string(),
            key_passphrase: passphrase.into(),
            password: String::new(),
            ..ssh("127.0.0.1", c.target)
        };
        let r = f.test(params).await;
        assert!(r.is_ok(), "{name} 应能登录：{r:?}");
    }
    let path = f.dir.join("k_rsa").display().to_string();
    let bad = SshParams {
        auth: AuthType::Key,
        key_path: path,
        key_passphrase: "bad".into(),
        ..ssh("127.0.0.1", c.target)
    };
    assert_eq!(f.test(bad).await, Err("err.ssh.keyDecryptFailed"));
    let other = f.dir.join("other");
    std::fs::write(
        &other,
        random_key().to_openssh(LineEnding::LF).unwrap().as_bytes(),
    )
    .unwrap();
    let denied = SshParams {
        auth: AuthType::Key,
        key_path: other.display().to_string(),
        ..ssh("127.0.0.1", c.target)
    };
    assert_eq!(f.test(denied).await, Err("err.ssh.authDenied"));
}

async fn agent_auth(c: &Containers) {
    let key = random_key();
    c.authorize(&key.public_key().to_openssh().unwrap());
    let f = fixture();
    let params = SshParams {
        auth: AuthType::Agent,
        password: String::new(),
        ..ssh("127.0.0.1", c.target)
    };
    unsafe { std::env::set_var("SSH_AUTH_SOCK", start_agent(&[random_key(), key]).await) };
    f.test(params.clone()).await.unwrap();
    unsafe { std::env::set_var("SSH_AUTH_SOCK", start_agent(&[random_key()]).await) };
    assert_eq!(f.test(params).await, Err("err.ssh.authDenied"));
}

async fn jump_host(c: &Containers) {
    let f = fixture();
    let j = f.add("跳板", ssh("127.0.0.1", c.bastion));
    // 目标机只在容器网络里可达，用服务名经跳板机连接。
    let t = f.add(
        "内网目标",
        SshParams {
            jump_id: j.clone(),
            ..ssh("target", 22)
        },
    );
    let mut a = f.term(&t).await;
    let hostname = c.sh("target", "hostname").trim().to_string();
    a.expect("hostname", &format!("{hostname}\r\n")).await;
    let mut b = f.term(&t).await;
    b.run("true").await;
    assert_eq!(f.pool.count(), 1, "两个标签共用一条跳板链");
    assert_eq!(f.asked(), ["跳板", "内网目标"], "每一级都确认指纹");

    let p = f.panel(&t).await;
    assert_eq!(p.home, HOME);
    assert!(
        f.sftp
            .list(&p.id, "/")
            .await
            .unwrap()
            .entries
            .iter()
            .any(|e| e.name == "etc"),
        "SFTP 也能经跳板机使用"
    );

    let bad = f.add(
        "坏跳板",
        SshParams {
            password: "wrong".into(),
            ..ssh("127.0.0.1", c.bastion)
        },
    );
    let t2 = f.add(
        "内网目标2",
        SshParams {
            jump_id: bad,
            ..ssh("target", 22)
        },
    );
    let conn = f.conns.get(&t2).unwrap().conn;
    let e = f
        .conns
        .test(SaveConnReq {
            conn,
            ..Default::default()
        })
        .await
        .unwrap_err();
    assert_eq!(e.msg.i18n, "err.ssh.jumpFailed");
}

async fn sftp_transfer(c: &Containers) {
    c.sh("target", "rm -rf ~/up ~/etc-link ~/host-link && ln -s /etc ~/etc-link && ln -s /etc/hostname ~/host-link");
    let f = fixture();
    let id = f.add("目标", ssh("127.0.0.1", c.target));
    let mut p = f.panel(&id).await;
    assert_eq!(p.home, HOME);

    let l = f.sftp.list(&p.id, HOME).await.unwrap();
    let get = |n: &str| {
        l.entries
            .iter()
            .find(|e| e.name == n)
            .unwrap_or_else(|| panic!("没有 {n}"))
    };
    assert!(
        get("etc-link").kind == FileKind::Symlink && get("etc-link").link_dir,
        "指向目录的符号链接"
    );
    assert!(
        get("host-link").kind == FileKind::Symlink && !get("host-link").link_dir,
        "指向文件的符号链接"
    );
    assert!(
        get(".ssh").perm.starts_with("drwx------"),
        "{}",
        get(".ssh").perm
    );
    assert_eq!(
        f.sftp
            .list(&p.id, &format!("{HOME}/etc-link"))
            .await
            .unwrap()
            .path,
        "/etc",
        "进入目录链接时显示真实路径"
    );

    let src = f.dir.join("up");
    write(&src.join("big.bin"), &pattern(8 * 1024 * 1024 + 123, 1));
    write(&src.join("中文 名称.txt"), "你好".as_bytes());
    write(&src.join("sub/deep/a.txt"), b"aaa");
    write(&src.join("empty"), b"");
    std::fs::create_dir_all(src.join("sub/emptydir")).unwrap();
    let list = f
        .sftp
        .upload(&p.id, &[src.display().to_string()], HOME)
        .unwrap();
    assert_eq!(
        p.finish(&list[0].id).await,
        SftpEvent::Done {
            id: list[0].id.clone()
        }
    );
    let size = c.sh("target", "stat -c %s ~/up/big.bin");
    assert_eq!(size.trim(), (8 * 1024 * 1024 + 123).to_string());
    assert_eq!(
        c.sh("target", "cat ~/up/'中文 名称.txt'"),
        "你好",
        "中文和空格文件名"
    );

    let dl = f.dir.join("down");
    std::fs::create_dir_all(&dl).unwrap();
    let list = f
        .sftp
        .download(&p.id, &[format!("{HOME}/up")], &dl.display().to_string())
        .unwrap();
    assert_eq!(
        p.finish(&list[0].id).await,
        SftpEvent::Done {
            id: list[0].id.clone()
        }
    );
    assert_same_tree(&src, &dl.join("up"));

    f.sftp
        .rename(
            &p.id,
            &format!("{HOME}/up/empty"),
            &format!("{HOME}/up/renamed"),
        )
        .await
        .unwrap();
    f.sftp.remove(&p.id, &[format!("{HOME}/up")]).await.unwrap();
    assert_eq!(
        c.sh("target", "test -e ~/up && echo yes || echo no").trim(),
        "no",
        "目录连同内容一起删除"
    );
}

async fn tui_apps(c: &Containers) {
    let f = fixture();
    let id = f.add("目标", ssh("127.0.0.1", c.target));
    let mut t = f.term(&id).await;
    t.run("true").await;

    // vim：进入备用屏幕，改变窗口大小后 vim 能拿到新尺寸，退出后恢复。
    t.send("vim\r").await;
    let out = t.read_until("VIM - Vi IMproved").await;
    assert!(out.contains("\x1b[?1049h"), "vim 应进入备用屏幕");
    t.resize(120, 40).await;
    t.read_until("\x1b[40;").await;
    t.send(":echo &columns . 'x' . &lines\r").await;
    t.read_until("120x40").await;
    assert!(
        t.quit(":q\r").await.contains("\x1b[?1049l"),
        "退出 vim 后离开备用屏幕"
    );
    t.expect("echo back", "back\r\n").await;

    // top：显示负载和进程列表，q 退出。
    t.send("top\r").await;
    t.read_until("load average").await;
    t.read_until("MiB Mem").await;
    t.resize(100, 30).await;
    t.quit("q").await;
    t.expect("echo back", "back\r\n").await;

    // htop：进入备用屏幕，显示 CPU 和内存条，q 退出后恢复。
    t.send("htop\r").await;
    t.read_until("\x1b[?1049h").await;
    t.read_until("Mem").await;
    t.resize(140, 45).await;
    assert!(
        t.quit("q").await.contains("\x1b[?1049l"),
        "退出 htop 后离开备用屏幕"
    );
    t.expect("echo back", "back\r\n").await;
}
