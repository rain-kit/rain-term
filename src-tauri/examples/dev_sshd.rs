//! 手动测试终端和 SFTP 界面用的本地 SSH 服务器（模拟 shell，命令见 tests/common/mod.rs）。
//! 运行：cargo run --example dev_sshd [端口]，默认 2222；用户 tester，密码 pa55。
//! 主机密钥保存在系统临时目录，重启后指纹不变。

#[path = "../tests/common/mod.rs"]
mod common;

use russh::keys::PrivateKey;
use russh::keys::ssh_key::LineEnding;

fn load_key() -> PrivateKey {
    let path = std::env::temp_dir().join("rain-dial-dev-sshd.key");
    if let Ok(text) = std::fs::read_to_string(&path)
        && let Ok(key) = PrivateKey::from_openssh(&text)
    {
        return key;
    }
    let key = common::random_key();
    std::fs::write(&path, key.to_openssh(LineEnding::LF).unwrap().as_bytes()).unwrap();
    key
}

#[tokio::main]
async fn main() {
    let port: u16 = std::env::args()
        .nth(1)
        .and_then(|p| p.parse().ok())
        .unwrap_or(2222);
    let srv = common::TestServer::start_on(&format!("127.0.0.1:{port}"), load_key()).await;
    println!(
        "dev sshd 监听 {}，用户 {}，密码 {}",
        srv.addr,
        common::USER,
        common::PASSWORD
    );
    println!("SFTP 的 / 对应本地目录 {}", srv.sftp_root.display());
    std::future::pending::<()>().await;
}
