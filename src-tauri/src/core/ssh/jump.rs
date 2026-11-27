//! 经跳板机连接：依次登录各级跳板机，每一级都通过上一级的 direct-tcpip channel 建立，最后连到目标主机。

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

use super::{Connected, DialOptions, SshClient, SshError, SshTarget};
use crate::i18n::Msg;

/// 跳板机断开时目标主机的连接随之断开，两个通知谁先到不确定；目标主机先断开时等这么久看是不是跳板机的原因。
const JUMP_REASON_GRACE: Duration = Duration::from_millis(200);

/// 跳板链上的一台主机。
pub struct Hop {
    /// 连接名称，出错时提示是哪一台。
    pub name: String,
    pub target: SshTarget,
    pub opts: DialOptions,
}

/// 依次连接 hops：前面的是跳板机（第一台直接连接），最后一台是目标主机。
/// 返回目标主机的连接，它持有全部跳板机连接；任何一级断开都从 disconnected 通知。
pub async fn connect_chain(
    hops: &[Hop],
    cancel: &CancellationToken,
) -> Result<Connected, SshError> {
    let Some((last, jumps)) = hops.split_last() else {
        return Err(SshError::Other(msg!("err.ssh.other", "SSH 错误")));
    };
    let mut via: Option<(Arc<SshClient>, &str)> = None;
    let mut elapsed = Duration::ZERO;
    let mut watch = Vec::with_capacity(jumps.len());
    for hop in jumps {
        let r = match &via {
            None => super::connect(&hop.target, &hop.opts, cancel).await,
            Some((client, name)) => {
                client
                    .connect_via(name, &hop.target, &hop.opts, cancel)
                    .await
            }
        };
        let c = r.map_err(|e| jump_failed(hop, e))?;
        elapsed += c.elapsed;
        watch.push((hop.name.clone(), c.disconnected));
        via = Some((c.client, &hop.name));
    }
    let Some((client, name)) = via else {
        return super::connect(&last.target, &last.opts, cancel).await;
    };
    let c = client
        .connect_via(name, &last.target, &last.opts, cancel)
        .await?;
    Ok(Connected {
        client: c.client,
        disconnected: merge(c.disconnected, watch),
        elapsed: elapsed + c.elapsed,
    })
}

/// 跳板机连接失败：消息里带上是哪一台，保留能否重试的性质。
fn jump_failed(hop: &Hop, e: SshError) -> SshError {
    if matches!(e, SshError::Cancelled) {
        return e;
    }
    let (name, addr, reason) = (&hop.name, hop.target.addr(), e.msg());
    let m = msg!(
        "err.ssh.jumpFailed",
        "连接跳板机 {name}（{addr}）失败: {reason}",
        name,
        addr,
        reason
    );
    if e.is_permanent() {
        SshError::Permanent(m)
    } else {
        SshError::Other(m)
    }
}

/// 合并目标主机和各级跳板机的断开通知，取最先断开的那一个；跳板机断开时说明是哪一台。
fn merge(
    target: oneshot::Receiver<Msg>,
    jumps: Vec<(String, oneshot::Receiver<Msg>)>,
) -> oneshot::Receiver<Msg> {
    let (tx, rx) = oneshot::channel();
    let (jtx, mut jrx) = mpsc::unbounded_channel();
    for (name, r) in jumps {
        let jtx = jtx.clone();
        tokio::spawn(async move {
            if let Ok(reason) = r.await {
                let _ = jtx.send(msg!(
                    "err.ssh.jumpDisconnected",
                    "跳板机 {name} 已断开: {reason}",
                    name,
                    reason
                ));
            }
        });
    }
    drop(jtx);
    tokio::spawn(async move {
        let reason = tokio::select! {
            biased;
            Some(m) = jrx.recv() => Some(m),
            r = target => match tokio::time::timeout(JUMP_REASON_GRACE, jrx.recv()).await {
                Ok(Some(m)) => Some(m),
                _ => r.ok(),
            },
        };
        if let Some(m) = reason {
            let _ = tx.send(m);
        }
    });
    rx
}
