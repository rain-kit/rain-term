mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use rain_dial::core::rdp::cert::CertStore;
use rain_dial::core::ssh::known_hosts::HostKeyStore;
use rain_dial::core::ssh::shell::PtySize;
use rain_dial::error;
use rain_dial::events::NoopSink;
use rain_dial::model::{ConnParams, Connection, SaveConnReq, SshParams};
use rain_dial::secrets::{MemoryBackend, Secrets};
use rain_dial::service::conn::ConnService;
use rain_dial::service::hostkey::HostKeyService;
use rain_dial::service::pool::ClientPool;
use rain_dial::service::session::{SessionEvent, SessionService};
use rain_dial::service::sftp::{
    Direction, FileKind, OpenOptions, SftpEvent, SftpService, Transfer,
};
use rain_dial::store::ConnStore;
use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

use common::sftp_fs::HOME;
use common::{PASSWORD, TestServer, USER, assert_same_tree, pattern, temp_dir, write};

const WAIT: Duration = Duration::from_secs(20);

struct Fixture {
    sftp: Arc<SftpService>,
    sessions: Arc<SessionService>,
    pool: Arc<ClientPool>,
    conns: Arc<ConnService>,
    dir: PathBuf,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.sessions.close_all();
        self.sftp.close_all();
        self.pool.close_all();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn fixture(srv: &TestServer) -> Fixture {
    let dir = temp_dir();
    let known = dir.join("known_hosts");
    HostKeyStore::new(known.clone(), vec![])
        .add("127.0.0.1", srv.addr.port(), srv.key.public_key())
        .unwrap();
    let host_keys = HostKeyService::new(
        HostKeyStore::new(known, vec![]),
        CertStore::new(dir.join("rdp_known_hosts")),
        Arc::new(NoopSink),
    );
    let conns = Arc::new(ConnService::new(
        ConnStore::new(dir.join("connections.json")),
        Secrets::new(Box::new(MemoryBackend::default())),
        host_keys,
        Arc::new(NoopSink),
    ));
    let pool = ClientPool::new(conns.clone());
    Fixture {
        sftp: SftpService::new(conns.clone(), pool.clone()),
        sessions: SessionService::new(conns.clone(), pool.clone()),
        pool,
        conns,
        dir,
    }
}

impl Fixture {
    fn add_conn(&self, srv: &TestServer, init_dir: &str) -> String {
        let params = SshParams {
            host: "127.0.0.1".into(),
            port: srv.addr.port() as i32,
            user: USER.into(),
            password: PASSWORD.into(),
            keepalive_seconds: 30,
            init_dir: init_dir.into(),
            ..Default::default()
        };
        let conn = Connection {
            name: "srv".into(),
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

    async fn open(&self, conn_id: &str) -> Panel {
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

    /// 本地临时目录下的路径。
    fn local(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }
}

struct Panel {
    id: String,
    home: String,
    rx: UnboundedReceiver<SftpEvent>,
}

/// 一个任务从开始到结束收到的事件。
#[derive(Debug)]
struct Run {
    total: Option<u64>,
    progress: Vec<(u64, u64)>,
    end: SftpEvent,
}

impl Panel {
    async fn next(&mut self) -> SftpEvent {
        tokio::time::timeout(WAIT, self.rx.recv())
            .await
            .expect("等待 SFTP 事件超时")
            .expect("事件通道已关闭")
    }

    /// 收集任务 id 的事件直到它结束；其他任务的事件忽略。
    async fn finish(&mut self, id: &str) -> Run {
        let mut run = Run {
            total: None,
            progress: vec![],
            end: SftpEvent::Done { id: String::new() },
        };
        loop {
            match self.next().await {
                SftpEvent::Started { id: i, total } if i == id => run.total = Some(total),
                SftpEvent::Progress { id: i, done, total } if i == id => {
                    run.progress.push((done, total))
                }
                ev @ (SftpEvent::Done { .. }
                | SftpEvent::Failed { .. }
                | SftpEvent::Cancelled { .. })
                    if event_id(&ev) == id =>
                {
                    run.end = ev;
                    return run;
                }
                SftpEvent::Closed { error } => panic!("等待任务结束时连接断开: {}", error.text()),
                _ => {}
            }
        }
    }

    async fn wait_progress(&mut self, id: &str) -> u64 {
        loop {
            match self.next().await {
                SftpEvent::Progress { id: i, done, .. } if i == id => return done,
                ev @ (SftpEvent::Done { .. }
                | SftpEvent::Failed { .. }
                | SftpEvent::Cancelled { .. })
                    if event_id(&ev) == id =>
                {
                    panic!("任务提前结束: {ev:?}")
                }
                _ => {}
            }
        }
    }
}

fn event_id(ev: &SftpEvent) -> &str {
    match ev {
        SftpEvent::Started { id, .. }
        | SftpEvent::Progress { id, .. }
        | SftpEvent::Done { id }
        | SftpEvent::Failed { id, .. }
        | SftpEvent::Cancelled { id } => id,
        SftpEvent::Closed { .. } => "",
    }
}

fn single(list: Vec<Transfer>) -> Transfer {
    assert_eq!(list.len(), 1);
    list.into_iter().next().unwrap()
}

#[tokio::test]
async fn browse_and_manage_files() {
    let srv = TestServer::start().await;
    let f = fixture(&srv);
    let id = f.add_conn(&srv, "");
    write(&srv.remote_file(&format!("{HOME}/b.txt")), b"hello");
    write(&srv.remote_file(&format!("{HOME}/A.log")), b"x");
    write(
        &srv.remote_file(&format!("{HOME}/zdir/inner/deep.txt")),
        b"deep",
    );
    let p = f.open(&id).await;
    assert_eq!(p.home, HOME);

    let l = f.sftp.list(&p.id, ".").await.unwrap();
    assert_eq!(l.path, HOME);
    let names: Vec<_> = l.entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(
        names,
        ["zdir", "A.log", "b.txt"],
        "目录在前，名称不区分大小写排序"
    );
    let b = &l.entries[2];
    assert_eq!(
        (b.kind, b.size, b.path.as_str()),
        (FileKind::File, 5, "/home/tester/b.txt")
    );
    assert!(b.mtime.is_some());
    assert_eq!(
        f.sftp.list(&p.id, "zdir/inner/../..").await.unwrap().path,
        HOME
    );
    assert_eq!(
        f.sftp.list(&p.id, "/").await.unwrap().entries[0].name,
        "home"
    );

    let err = f.sftp.list(&p.id, "/nope").await.unwrap_err();
    assert_eq!(err.code, error::SFTP_FAILED.0);
    assert!(
        err.msg.text().contains("文件或目录不存在"),
        "{}",
        err.msg.text()
    );

    f.sftp.mkdir(&p.id, "/home/tester/new").await.unwrap();
    assert!(srv.remote_file("/home/tester/new").is_dir());
    let err = f.sftp.mkdir(&p.id, "/home/tester/new").await.unwrap_err();
    assert_eq!(err.code, error::SFTP_FAILED.0);

    f.sftp
        .rename(&p.id, "/home/tester/b.txt", "/home/tester/new/c.txt")
        .await
        .unwrap();
    assert_eq!(
        std::fs::read(srv.remote_file("/home/tester/new/c.txt")).unwrap(),
        b"hello"
    );
    assert!(!srv.remote_file("/home/tester/b.txt").exists());

    f.sftp
        .remove(
            &p.id,
            &["/home/tester/zdir".into(), "/home/tester/A.log".into()],
        )
        .await
        .unwrap();
    let names: Vec<_> = f
        .sftp
        .list(&p.id, HOME)
        .await
        .unwrap()
        .entries
        .into_iter()
        .map(|e| e.name)
        .collect();
    assert_eq!(names, ["new"]);

    f.sftp.close(&p.id);
    let err = f.sftp.list(&p.id, HOME).await.unwrap_err();
    assert_eq!(err.code, error::SFTP_NOT_FOUND.0);
}

#[tokio::test]
async fn start_dir_follows_init_dir() {
    let srv = TestServer::start().await;
    let f = fixture(&srv);
    std::fs::create_dir_all(srv.remote_file("/home/tester/sub")).unwrap();
    std::fs::create_dir_all(srv.remote_file("/srv/app")).unwrap();
    write(&srv.remote_file("/srv/file"), b"");
    for (init, want) in [
        ("~/sub", "/home/tester/sub"),
        ("/srv/app/", "/srv/app"),
        ("/srv/file", HOME),
        ("/missing", HOME),
        ("~", HOME),
    ] {
        let id = f.add_conn(&srv, init);
        let p = f.open(&id).await;
        assert_eq!(p.home, want, "init_dir = {init}");
        f.sftp.close(&p.id);
    }
}

#[tokio::test]
async fn upload_and_download_trees() {
    let srv = TestServer::start().await;
    let f = fixture(&srv);
    let id = f.add_conn(&srv, "");
    let mut p = f.open(&id).await;

    let src = f.local("up/proj");
    write(&src.join("big.bin"), &pattern(3 * 1024 * 1024 + 17, 1));
    write(&src.join("empty"), b"");
    write(&src.join("sub/a.txt"), b"aaa");
    std::fs::create_dir_all(src.join("sub/emptydir")).unwrap();
    let single_file = f.local("up/one.bin");
    write(&single_file, &pattern(100_000, 2));
    write(&srv.remote_file("/home/tester/one.bin"), b"old content");

    let paths = vec![src.display().to_string(), single_file.display().to_string()];
    let list = f.sftp.upload(&p.id, &paths, HOME).unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!(
        (
            list[0].direction,
            list[0].name.as_str(),
            list[0].dst.as_str()
        ),
        (Direction::Upload, "proj", "/home/tester/proj")
    );
    let run = p.finish(&list[0].id).await;
    assert_eq!(
        run.end,
        SftpEvent::Done {
            id: list[0].id.clone()
        }
    );
    let total = 3 * 1024 * 1024 + 17 + 3;
    assert_eq!(run.total, Some(total));
    assert!(
        run.progress.windows(2).all(|w| w[0].0 <= w[1].0),
        "进度单调递增: {:?}",
        run.progress
    );
    assert!(run.progress.iter().all(|&(d, t)| d <= t && t == total));
    let run = p.finish(&list[1].id).await;
    assert_eq!(
        run.end,
        SftpEvent::Done {
            id: list[1].id.clone()
        }
    );
    assert_same_tree(&src, &srv.remote_file("/home/tester/proj"));
    assert_eq!(
        std::fs::read(srv.remote_file("/home/tester/one.bin")).unwrap(),
        pattern(100_000, 2),
        "同名文件被覆盖"
    );

    let dl = f.local("down");
    std::fs::create_dir_all(&dl).unwrap();
    let dl_dir = dl.display().to_string();
    let t = single(
        f.sftp
            .download(&p.id, &["/home/tester/proj".into()], &dl_dir)
            .unwrap(),
    );
    assert_eq!(
        (t.direction, t.dst.clone()),
        (Direction::Download, dl.join("proj").display().to_string())
    );
    let run = p.finish(&t.id).await;
    assert_eq!(
        (run.total, run.end),
        (Some(total), SftpEvent::Done { id: t.id.clone() })
    );
    assert_same_tree(&src, &dl.join("proj"));

    let t = single(
        f.sftp
            .download(&p.id, &["/home/tester/one.bin".into()], &dl_dir)
            .unwrap(),
    );
    assert_eq!(
        p.finish(&t.id).await.end,
        SftpEvent::Done { id: t.id.clone() }
    );
    assert_eq!(
        std::fs::read(dl.join("one.bin")).unwrap(),
        pattern(100_000, 2)
    );

    let t = single(
        f.sftp
            .download(&p.id, &["/home/tester/missing".into()], &dl_dir)
            .unwrap(),
    );
    match p.finish(&t.id).await.end {
        SftpEvent::Failed { error, .. } => assert!(
            error.text().contains("文件或目录不存在"),
            "{}",
            error.text()
        ),
        other => panic!("{other:?}"),
    }
    let t = single(
        f.sftp
            .upload(&p.id, &[f.local("missing").display().to_string()], HOME)
            .unwrap(),
    );
    match p.finish(&t.id).await.end {
        SftpEvent::Failed { error, .. } => assert_eq!(error.i18n, "err.sftp.onPath"),
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn cancel_removes_partial_file() {
    let srv = TestServer::start().await;
    srv.sftp_delay_ms.store(5, Ordering::SeqCst);
    let f = fixture(&srv);
    let id = f.add_conn(&srv, "");
    let mut p = f.open(&id).await;
    let src = f.local("big.bin");
    write(&src, &pattern(32 * 1024 * 1024, 3));

    let ts = f
        .sftp
        .upload(
            &p.id,
            &[src.display().to_string(), src.display().to_string()],
            "/home/tester/",
        )
        .unwrap();
    f.sftp.cancel(&p.id, &ts[1].id).unwrap();
    p.wait_progress(&ts[0].id).await;
    f.sftp.cancel(&p.id, &ts[0].id).unwrap();
    let run = p.finish(&ts[0].id).await;
    assert_eq!(
        run.end,
        SftpEvent::Cancelled {
            id: ts[0].id.clone()
        }
    );
    assert_eq!(
        p.next().await,
        SftpEvent::Cancelled {
            id: ts[1].id.clone()
        },
        "排队中的任务取消后不会开始"
    );
    assert!(
        !srv.remote_file("/home/tester/big.bin").exists(),
        "取消上传后删除远程的半截文件"
    );

    write(
        &srv.remote_file("/home/tester/big.bin"),
        &pattern(32 * 1024 * 1024, 3),
    );
    let dl = f.local("dl");
    std::fs::create_dir_all(&dl).unwrap();
    let t = single(
        f.sftp
            .download(
                &p.id,
                &["/home/tester/big.bin".into()],
                &dl.display().to_string(),
            )
            .unwrap(),
    );
    p.wait_progress(&t.id).await;
    f.sftp.cancel(&p.id, &t.id).unwrap();
    assert_eq!(
        p.finish(&t.id).await.end,
        SftpEvent::Cancelled { id: t.id.clone() }
    );
    assert!(!dl.join("big.bin").exists(), "取消下载后删除本地的半截文件");
    f.sftp.cancel(&p.id, &t.id).unwrap();

    let t = single(
        f.sftp
            .upload(&p.id, &[src.display().to_string()], "/home/tester/up")
            .unwrap(),
    );
    let ev = p.finish(&t.id).await.end;
    assert!(
        matches!(ev, SftpEvent::Failed { .. }),
        "目标目录不存在: {ev:?}"
    );
}

#[tokio::test]
async fn shares_connection_with_terminal() {
    let srv = TestServer::start().await;
    let f = fixture(&srv);
    let id = f.add_conn(&srv, "");
    let (tx, mut term_rx) = unbounded_channel();
    let term = f
        .sessions
        .open(
            &id,
            PtySize { cols: 80, rows: 24 },
            Box::new(move |e| drop(tx.send(e))),
        )
        .await
        .unwrap();
    let p = f.open(&id).await;
    let q = f.open(&id).await;
    assert_eq!(
        (
            srv.accepted.load(Ordering::SeqCst),
            f.pool.count(),
            f.sftp.panel_count()
        ),
        (1, 1, 2)
    );
    f.sftp.list(&q.id, "/").await.unwrap();

    f.sftp.close(&p.id);
    f.sftp.close(&q.id);
    f.sessions.write(&term, b"echo alive\r").await.unwrap();
    let mut out = String::new();
    while !out.contains("alive\r\n") {
        match tokio::time::timeout(WAIT, term_rx.recv()).await.unwrap() {
            Some(SessionEvent::Output(d)) => {
                f.sessions.ack(&term, d.len());
                out.push_str(&String::from_utf8_lossy(&d));
            }
            other => panic!("{other:?}"),
        }
    }
    assert_eq!(f.pool.count(), 1, "关闭 SFTP 面板不影响终端");

    f.sessions.close(&term).await;
    let ok = tokio::time::timeout(WAIT, async {
        while f.pool.count() != 0 {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    assert!(ok.is_ok(), "终端和面板都关闭后断开 SSH 连接");
}

#[tokio::test]
async fn server_disconnect_closes_panel() {
    let srv = TestServer::start().await;
    srv.sftp_delay_ms.store(5, Ordering::SeqCst);
    let f = fixture(&srv);
    let id = f.add_conn(&srv, "");
    let mut p = f.open(&id).await;
    let src = f.local("big.bin");
    write(&src, &pattern(32 * 1024 * 1024, 4));
    let t = single(
        f.sftp
            .upload(&p.id, &[src.display().to_string()], HOME)
            .unwrap(),
    );
    p.wait_progress(&t.id).await;

    srv.kick_all().await;
    let closed = loop {
        match p.next().await {
            SftpEvent::Closed { error } => break error,
            SftpEvent::Progress { .. } | SftpEvent::Started { .. } => {}
            other => panic!("断开后只应收到 Closed: {other:?}"),
        }
    };
    assert_eq!(closed.i18n, "err.ssh.serverClosed");
    assert!(
        tokio::time::timeout(WAIT, p.rx.recv())
            .await
            .unwrap()
            .is_none(),
        "Closed 之后不应再有消息"
    );
    assert_eq!(
        f.sftp.list(&p.id, HOME).await.unwrap_err().code,
        error::SFTP_NOT_FOUND.0
    );
    assert_eq!(f.sftp.panel_count(), 0);

    let q = f.open(&id).await;
    assert_eq!(q.home, HOME, "断开后可以重新打开");
    assert_eq!(srv.accepted.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn subsystem_disabled() {
    let srv = TestServer::start().await;
    srv.sftp_off.store(true, Ordering::SeqCst);
    let f = fixture(&srv);
    let id = f.add_conn(&srv, "");
    let err = f
        .sftp
        .open(&id, OpenOptions::default(), false, Box::new(|_| {}))
        .await
        .unwrap_err();
    assert_eq!(err.code, error::SFTP_OPEN_FAILED.0);
    assert!(
        err.msg.text().contains("服务器未启用 SFTP"),
        "{}",
        err.msg.text()
    );
    assert_eq!(f.pool.count(), 0, "打开失败后释放连接");
}

/// 1GB 文件上传下载。耗时较长，用 cargo test --release --test sftp -- --ignored --nocapture 运行。
#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn transfer_1gb() {
    const SIZE: u64 = 1 << 30;
    let srv = TestServer::start().await;
    let f = fixture(&srv);
    let id = f.add_conn(&srv, "");
    let mut p = f.open(&id).await;
    let src = f.local("1g.bin");
    {
        use std::io::Write;
        let mut w = std::io::BufWriter::new(std::fs::File::create(&src).unwrap());
        let chunk = pattern(1 << 20, 5);
        for i in 0..(SIZE >> 20) {
            let mut c = chunk.clone();
            c[..8].copy_from_slice(&i.to_le_bytes());
            w.write_all(&c).unwrap();
        }
    }
    let src_s = src.display().to_string();

    let t = single(
        f.sftp
            .upload(&p.id, std::slice::from_ref(&src_s), HOME)
            .unwrap(),
    );
    loop {
        if p.wait_progress(&t.id).await > SIZE / 3 {
            break;
        }
    }
    f.sftp.cancel(&p.id, &t.id).unwrap();
    assert_eq!(
        p.finish(&t.id).await.end,
        SftpEvent::Cancelled { id: t.id.clone() }
    );
    assert!(!srv.remote_file("/home/tester/1g.bin").exists());

    for direction in [Direction::Upload, Direction::Download] {
        let start = std::time::Instant::now();
        let dl = f.local("dl");
        std::fs::create_dir_all(&dl).unwrap();
        let t = match direction {
            Direction::Upload => single(
                f.sftp
                    .upload(&p.id, std::slice::from_ref(&src_s), HOME)
                    .unwrap(),
            ),
            Direction::Download => single(
                f.sftp
                    .download(
                        &p.id,
                        &["/home/tester/1g.bin".into()],
                        &dl.display().to_string(),
                    )
                    .unwrap(),
            ),
        };
        let run = p.finish(&t.id).await;
        assert_eq!(run.end, SftpEvent::Done { id: t.id.clone() });
        assert_eq!(run.total, Some(SIZE));
        assert!(run.progress.windows(2).all(|w| w[0].0 < w[1].0));
        assert!(run.progress.iter().all(|&(d, t)| d <= SIZE && t == SIZE));
        let secs = start.elapsed().as_secs_f64();
        println!(
            "{direction:?}: {:.1}s，{:.0} MB/s，进度事件 {} 条",
            secs,
            1024.0 / secs,
            run.progress.len()
        );
    }
    let same = |a: &Path, b: &Path| {
        use std::io::Read;
        let (mut x, mut y) = (
            std::fs::File::open(a).unwrap(),
            std::fs::File::open(b).unwrap(),
        );
        let (mut bx, mut by) = (vec![0u8; 1 << 20], vec![0u8; 1 << 20]);
        loop {
            let n = x.read(&mut bx).unwrap();
            y.read_exact(&mut by[..n]).unwrap();
            if bx[..n] != by[..n] {
                return false;
            }
            if n == 0 {
                return y.read(&mut by).unwrap() == 0;
            }
        }
    };
    assert!(
        same(&src, &srv.remote_file("/home/tester/1g.bin")),
        "上传后内容一致"
    );
    assert!(same(&src, &f.local("dl/1g.bin")), "下载后内容一致");
}
