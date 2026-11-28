//! ssh-agent 认证。会修改本进程的 SSH_AUTH_SOCK，所以单独放一个测试文件。
mod common;

use std::sync::Arc;
use std::time::Duration;

use rain_term::core::ssh::known_hosts::HostKeyStore;
use rain_term::core::ssh::{self, DialOptions, SshTarget};
use rain_term::model::AuthType;
use tokio_util::sync::CancellationToken;

use common::{TestServer, USER, random_key, start_agent, temp_dir};

#[tokio::test]
async fn agent_auth() {
    let srv = TestServer::start().await;
    let dir = temp_dir();
    let opts = DialOptions {
        host_keys: Arc::new(HostKeyStore::new(dir.join("known_hosts"), vec![])),
        trust: Some(Arc::new(|_q| Box::pin(async { true }))),
        timeout: Duration::from_secs(5),
        keepalive: None,
    };
    let t = SshTarget {
        host: "127.0.0.1".into(),
        port: srv.addr.port(),
        user: USER.into(),
        auth: AuthType::Agent,
        ..Default::default()
    };
    let cancel = CancellationToken::new();
    let dial = || ssh::connect(&t, &opts, &cancel);

    let key = random_key();
    srv.authorize(key.public_key());
    // 其他测试文件不读这个变量，这里也只有一个测试。
    unsafe { std::env::set_var("SSH_AUTH_SOCK", start_agent(&[random_key(), key]).await) };
    dial().await.unwrap();

    unsafe { std::env::set_var("SSH_AUTH_SOCK", start_agent(&[random_key()]).await) };
    let Err(e) = dial().await else {
        panic!("agent 里没有授权的密钥时应失败")
    };
    assert_eq!(e.msg().i18n, "err.ssh.authDenied");

    unsafe { std::env::set_var("SSH_AUTH_SOCK", start_agent(&[]).await) };
    let Err(e) = dial().await else {
        panic!("agent 为空时应失败")
    };
    assert_eq!(e.msg().i18n, "err.ssh.agentNoKeys");
    let _ = std::fs::remove_dir_all(dir);
}
