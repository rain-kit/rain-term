use std::time::Duration;

use russh::client::Msg as ClientMsg;
use russh::{Channel, ChannelMsg};
use russh_sftp::client::error::Error as SftpError;
use russh_sftp::client::{Config, SftpSession};
use russh_sftp::protocol::StatusCode;

use super::{SshClient, SshError};
use crate::i18n::Msg;

/// 单个请求等待回复的最长时间。同时在途的写入约 1MB、读取约 2MB，慢速链路上排在后面的请求也要等前面的传完。
const REQUEST_TIMEOUT_SECS: u64 = 60;
/// 检查 sudo 权限的最长时间。
const PROBE_TIMEOUT: Duration = Duration::from_secs(15);
/// 检查脚本的输出只看第一行，多余的丢弃。
const PROBE_OUTPUT_LIMIT: usize = 4096;
/// 常见发行版里 sftp-server 的位置；sshd_config 写明的路径优先。
const SERVER_PATHS: [&str; 7] = [
    "/usr/lib/openssh/sftp-server",
    "/usr/libexec/openssh/sftp-server",
    "/usr/lib/ssh/sftp-server",
    "/usr/libexec/sftp-server",
    "/usr/lib/sftp-server",
    "/usr/local/libexec/sftp-server",
    "/usr/sbin/sftp-server",
];
/// 检查脚本找不到 sftp-server 时的退出码。
const NO_SERVER: u32 = 100;

/// 用 sudo 打开 SFTP 失败的原因。
#[derive(Debug)]
pub enum SudoError {
    /// sudo 需要密码，但没有提供。
    PasswordRequired,
    /// 提供的 sudo 密码不对。
    WrongPassword,
    Ssh(SshError),
}

impl From<SshError> for SudoError {
    fn from(e: SshError) -> Self {
        SudoError::Ssh(e)
    }
}

impl From<russh::Error> for SudoError {
    fn from(e: russh::Error) -> Self {
        SudoError::Ssh(e.into())
    }
}

/// 一条一次性命令的结果。
#[derive(Default)]
struct Output {
    status: Option<u32>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

impl SshClient {
    /// 在本连接上打开一个 sftp 子系统 channel，权限是登录用户的。
    pub async fn open_sftp(&self) -> Result<SftpSession, SshError> {
        let mut channel = self.handle.channel_open_session().await?;
        channel.request_subsystem(true, "sftp").await?;
        wait_reply(&mut channel, || {
            msg!("err.sftp.subsystemRejected", "服务器未启用 SFTP")
        })
        .await?;
        start(channel).await
    }

    /// 用 sudo 以 root 身份运行 sftp-server。password 为 None 时只用免密 sudo；
    /// 免密 sudo 不可用时不要直接传密码，否则 sudo 不读密码，密码会被当成 SFTP 数据。
    pub async fn open_sftp_sudo(&self, password: Option<&str>) -> Result<SftpSession, SudoError> {
        let input = password.map(|p| format!("{p}\n")).unwrap_or_default();
        let command = format!("sh -c {}", quote(&probe_script(password.is_some())));
        let probe = tokio::time::timeout(PROBE_TIMEOUT, self.run(&command, input.as_bytes()))
            .await
            .map_err(|_| SshError::Other(msg!("err.sftp.sudoTimeout", "检查 sudo 权限超时")))??;
        let server = String::from_utf8_lossy(&probe.stdout)
            .lines()
            .next()
            .unwrap_or_default()
            .trim()
            .to_string();
        match probe.status {
            Some(0) if !server.is_empty() => {}
            Some(NO_SERVER) => {
                return Err(SshError::Other(msg!(
                    "err.sftp.serverNotFound",
                    "服务器上找不到 sftp-server"
                ))
                .into());
            }
            _ => return Err(sudo_failure(&probe.stderr, password.is_some(), &server)),
        }

        let mut channel = self.handle.channel_open_session().await?;
        let command = match password {
            None => format!("sudo -n -- {}", quote(&server)),
            Some(_) => format!("sudo -S -k -p '' -- {}", quote(&server)),
        };
        channel.exec(true, command.as_bytes()).await?;
        wait_reply(&mut channel, || {
            msg!("err.sftp.execRejected", "服务器拒绝执行命令")
        })
        .await?;
        if !input.is_empty() {
            channel.data(input.as_bytes()).await?;
        }
        Ok(start(channel).await?)
    }

    /// 执行一条命令，把 input 写到它的标准输入，等它结束。
    async fn run(&self, command: &str, input: &[u8]) -> Result<Output, SshError> {
        let mut channel = self.handle.channel_open_session().await?;
        channel.exec(true, command.as_bytes()).await?;
        if !input.is_empty() {
            channel.data(input).await?;
        }
        channel.eof().await?;
        let mut out = Output::default();
        let keep = |buf: &mut Vec<u8>, data: &[u8]| {
            let n = data.len().min(PROBE_OUTPUT_LIMIT - buf.len());
            buf.extend_from_slice(&data[..n]);
        };
        while let Some(msg) = channel.wait().await {
            match msg {
                ChannelMsg::Data { data } => keep(&mut out.stdout, &data),
                ChannelMsg::ExtendedData { data, ext: 1 } => keep(&mut out.stderr, &data),
                ChannelMsg::ExitStatus { exit_status } => out.status = Some(exit_status),
                ChannelMsg::Failure => {
                    return Err(SshError::Other(msg!(
                        "err.sftp.execRejected",
                        "服务器拒绝执行命令"
                    )));
                }
                ChannelMsg::Close => break,
                _ => {}
            }
        }
        Ok(out)
    }
}

/// 等服务器答复 channel 请求，拒绝时返回 rejected 给出的消息。
async fn wait_reply(channel: &mut Channel<ClientMsg>, rejected: impl Fn() -> Msg) -> Result<(), SshError> {
    loop {
        match channel.wait().await {
            Some(ChannelMsg::Success) => return Ok(()),
            Some(ChannelMsg::Failure) | None => return Err(SshError::Other(rejected())),
            Some(_) => {}
        }
    }
}

async fn start(channel: Channel<ClientMsg>) -> Result<SftpSession, SshError> {
    let cfg = Config {
        max_concurrent_reads: 8,
        max_concurrent_writes: 16,
        max_write_packet_len: 64 * 1024,
        request_timeout_secs: REQUEST_TIMEOUT_SECS,
        ..Default::default()
    };
    SftpSession::new_with_config(channel.into_stream(), cfg)
        .await
        .map_err(|e| SshError::Other(msg!("err.sftp.initFailed", "SFTP 初始化失败").with_cause(e)))
}

/// 找到 sftp-server 并打印它的路径，再检查能否用 sudo 运行它（sudo -l 不真的运行）。
/// sudo 的提示固定用英文，便于判断失败原因。
fn probe_script(with_password: bool) -> String {
    let sudo = if with_password {
        r#"sudo -S -k -p """#
    } else {
        "sudo -n"
    };
    format!(
        r#"for c in $(awk '{{ if (tolower($1) == "subsystem" && $2 == "sftp") print $3 }}' /etc/ssh/sshd_config 2>/dev/null) {paths}; do if [ -x "$c" ]; then echo "$c"; LC_ALL=C exec {sudo} -l "$c" >/dev/null; fi; done; exit {NO_SERVER}"#,
        paths = SERVER_PATHS.join(" "),
    )
}

/// 用单引号包起来，sh、bash、zsh、fish 都按原样传给命令。
fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

fn sudo_failure(stderr: &[u8], with_password: bool, server: &str) -> SudoError {
    let text = String::from_utf8_lossy(stderr);
    if !with_password && text.contains("password is required") {
        return SudoError::PasswordRequired;
    }
    if with_password && (text.contains("incorrect password") || text.contains("try again")) {
        return SudoError::WrongPassword;
    }
    let m = match text.lines().map(str::trim).rfind(|l| !l.is_empty()) {
        Some(line) => msg!("err.sftp.sudoFailed", "sudo 执行失败").with_cause(line),
        None => msg!(
            "err.sftp.sudoNotAllowed",
            "当前用户不能用 sudo 运行 {server}",
            server
        ),
    };
    SudoError::Ssh(SshError::Other(m))
}

/// SFTP 请求失败的原因。
pub fn describe(e: &SftpError) -> Msg {
    match e {
        SftpError::Status(s) => match s.status_code {
            StatusCode::NoSuchFile => msg!("err.sftp.noSuchFile", "文件或目录不存在"),
            StatusCode::PermissionDenied => msg!("err.sftp.permissionDenied", "没有权限"),
            StatusCode::OpUnsupported => msg!("err.sftp.unsupported", "服务器不支持该操作"),
            StatusCode::NoConnection | StatusCode::ConnectionLost => {
                msg!("err.sftp.connectionLost", "SFTP 连接已断开")
            }
            _ if s.error_message.is_empty() => msg!("err.sftp.failure", "服务器返回失败"),
            _ => msg!("err.sftp.failure", "服务器返回失败").with_cause(&s.error_message),
        },
        SftpError::Timeout => msg!("err.sftp.timeout", "服务器响应超时"),
        other => msg!("err.sftp.other", "SFTP 错误").with_cause(other),
    }
}

/// 是否是“文件不存在”。
pub fn is_not_found(e: &SftpError) -> bool {
    matches!(e, SftpError::Status(s) if s.status_code == StatusCode::NoSuchFile)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quote_escapes_single_quotes() {
        assert_eq!(quote("/usr/lib/sftp-server"), "'/usr/lib/sftp-server'");
        assert_eq!(quote("a'b"), r"'a'\''b'");
    }

    #[test]
    fn probe_script_uses_matching_sudo() {
        let s = probe_script(false);
        assert!(s.contains("exec sudo -n -l \"$c\""));
        assert!(s.contains("/usr/lib/openssh/sftp-server"));
        assert!(s.ends_with("exit 100"));
        assert!(probe_script(true).contains(r#"exec sudo -S -k -p "" -l "$c""#));
    }

    #[test]
    fn classify_sudo_failures() {
        assert!(matches!(
            sudo_failure(b"sudo: a password is required\n", false, "/s"),
            SudoError::PasswordRequired
        ));
        assert!(matches!(
            sudo_failure(b"Sorry, try again.\nsudo: 1 incorrect password attempt\n", true, "/s"),
            SudoError::WrongPassword
        ));
        let SudoError::Ssh(SshError::Other(m)) = sudo_failure(
            b"\nubuntu is not in the sudoers file.\n",
            true,
            "/s",
        ) else {
            panic!("应为其他错误");
        };
        assert_eq!(m.i18n, "err.sftp.sudoFailed");
        let SudoError::Ssh(SshError::Other(m)) = sudo_failure(b"", false, "/s") else {
            panic!("应为其他错误");
        };
        assert_eq!(m.text(), "当前用户不能用 sudo 运行 /s");
    }
}
