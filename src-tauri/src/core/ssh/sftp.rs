use russh::ChannelMsg;
use russh_sftp::client::error::Error as SftpError;
use russh_sftp::client::{Config, SftpSession};
use russh_sftp::protocol::StatusCode;

use super::{SshClient, SshError};
use crate::i18n::Msg;

/// 单个请求等待回复的最长时间。同时在途的写入约 1MB、读取约 2MB，慢速链路上排在后面的请求也要等前面的传完。
const REQUEST_TIMEOUT_SECS: u64 = 60;

impl SshClient {
    /// 在本连接上打开一个 sftp 子系统 channel。
    pub async fn open_sftp(&self) -> Result<SftpSession, SshError> {
        let mut channel = self.handle.channel_open_session().await?;
        channel.request_subsystem(true, "sftp").await?;
        loop {
            match channel.wait().await {
                Some(ChannelMsg::Success) => break,
                Some(ChannelMsg::Failure) | None => {
                    return Err(SshError::Other(msg!(
                        "err.sftp.subsystemRejected",
                        "服务器未启用 SFTP"
                    )));
                }
                Some(_) => {}
            }
        }
        let cfg = Config {
            max_concurrent_reads: 8,
            max_concurrent_writes: 16,
            max_write_packet_len: 64 * 1024,
            request_timeout_secs: REQUEST_TIMEOUT_SECS,
            ..Default::default()
        };
        SftpSession::new_with_config(channel.into_stream(), cfg)
            .await
            .map_err(|e| {
                SshError::Other(msg!("err.sftp.initFailed", "SFTP 初始化失败").with_cause(e))
            })
    }
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
