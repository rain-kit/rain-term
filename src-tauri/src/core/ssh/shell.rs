use std::collections::VecDeque;

use russh::client::Msg;
use russh::{ChannelMsg, ChannelReadHalf, ChannelWriteHalf, Pty, Sig};

use super::{SshClient, SshError};

pub const TERM: &str = "xterm-256color";

/// 退格键发 DEL，行编辑按 UTF-8 处理（删除中文时一次删一个字），与 xterm.js 的行为一致。
const MODES: &[(Pty, u32)] = &[(Pty::VERASE, 127), (Pty::IUTF8, 1)];

/// 终端尺寸，单位是字符。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PtySize {
    pub cols: u32,
    pub rows: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExitStatus {
    Code(u32),
    /// 被信号结束，值为信号名（如 KILL）。
    Signal(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShellEvent {
    /// 终端输出；有 PTY 时标准错误也合在这里。
    Output(Vec<u8>),
    Exit(ExitStatus),
}

/// 写入端：键盘输入、调整尺寸、关闭。方法只需要 &self，可以放进 Arc 在多处使用。
pub struct ShellWriter(ChannelWriteHalf<Msg>);

impl ShellWriter {
    pub async fn write(&self, data: &[u8]) -> Result<(), SshError> {
        Ok(self.0.data_bytes(data.to_vec()).await?)
    }

    pub async fn resize(&self, size: PtySize) -> Result<(), SshError> {
        Ok(self.0.window_change(size.cols, size.rows, 0, 0).await?)
    }

    pub async fn close(&self) {
        let _ = self.0.eof().await;
        let _ = self.0.close().await;
    }
}

/// 读取端：依次读出输出和退出状态。
pub struct ShellReader {
    half: ChannelReadHalf,
    /// 等待请求回复期间先收到的消息。
    pending: VecDeque<ChannelMsg>,
}

impl ShellReader {
    /// channel 关闭后返回 None；退出状态在关闭前给出，服务器没发时就没有。
    pub async fn next(&mut self) -> Option<ShellEvent> {
        loop {
            let msg = match self.pending.pop_front() {
                Some(m) => m,
                None => self.half.wait().await?,
            };
            match msg {
                ChannelMsg::Data { data } | ChannelMsg::ExtendedData { data, .. } => {
                    return Some(ShellEvent::Output(data.to_vec()));
                }
                ChannelMsg::ExitStatus { exit_status } => {
                    return Some(ShellEvent::Exit(ExitStatus::Code(exit_status)));
                }
                ChannelMsg::ExitSignal { signal_name, .. } => {
                    return Some(ShellEvent::Exit(ExitStatus::Signal(signal(signal_name))));
                }
                ChannelMsg::Close => return None,
                _ => {}
            }
        }
    }
}

fn signal(s: Sig) -> String {
    match s {
        Sig::Custom(name) => name,
        other => format!("{other:?}"),
    }
}

impl SshClient {
    /// 打开交互式 shell：申请 PTY，再启动用户的登录 shell。
    pub async fn open_shell(&self, size: PtySize) -> Result<(ShellWriter, ShellReader), SshError> {
        let channel = self.handle.channel_open_session().await?;
        let (mut read, write) = channel.split();
        let mut pending = VecDeque::new();

        write
            .request_pty(true, TERM, size.cols, size.rows, 0, 0, MODES)
            .await?;
        if !wait_reply(&mut read, &mut pending).await {
            let _ = write.close().await;
            return Err(SshError::Other(msg!(
                "err.ssh.ptyRejected",
                "服务器拒绝分配终端"
            )));
        }
        write.request_shell(true).await?;
        if !wait_reply(&mut read, &mut pending).await {
            let _ = write.close().await;
            return Err(SshError::Other(msg!(
                "err.ssh.shellRejected",
                "服务器拒绝启动 shell"
            )));
        }
        Ok((
            ShellWriter(write),
            ShellReader {
                half: read,
                pending,
            },
        ))
    }
}

/// 等服务器对请求的回复，期间收到的其他消息留给读取端；channel 关闭视为失败。
async fn wait_reply(read: &mut ChannelReadHalf, pending: &mut VecDeque<ChannelMsg>) -> bool {
    while let Some(msg) = read.wait().await {
        match msg {
            ChannelMsg::Success => return true,
            ChannelMsg::Failure => return false,
            other => pending.push_back(other),
        }
    }
    false
}
