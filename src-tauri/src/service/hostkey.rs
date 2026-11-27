use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use tokio::sync::oneshot;
use tracing::{info, warn};

use crate::core::ssh::known_hosts::HostKeyStore;
use crate::core::ssh::{DEFAULT_TIMEOUT, DialOptions, HostKeyQuery, TrustFn};
use crate::error::{self, ApiResult};
use crate::events::{self, EventSink};
use crate::model::HostKeyPrompt;
use crate::util::new_id;

/// 用户一直不回答时，按拒绝处理。
pub const PROMPT_TIMEOUT: Duration = Duration::from_secs(120);

struct Prompt {
    info: HostKeyPrompt,
    answer: Option<oneshot::Sender<bool>>,
}

/// 管理已信任的主机指纹，以及等待用户确认的请求。终端会话、SFTP、测试连接共用。
pub struct HostKeyService {
    store: Arc<HostKeyStore>,
    events: Arc<dyn EventSink>,
    prompts: Mutex<HashMap<String, Prompt>>,
    prompt_timeout: Duration,
    me: Weak<HostKeyService>,
}

impl HostKeyService {
    pub fn new(store: HostKeyStore, events: Arc<dyn EventSink>) -> Arc<Self> {
        Self::with_timeout(store, events, PROMPT_TIMEOUT)
    }

    pub fn with_timeout(
        store: HostKeyStore,
        events: Arc<dyn EventSink>,
        prompt_timeout: Duration,
    ) -> Arc<Self> {
        Arc::new_cyclic(|me| HostKeyService {
            store: Arc::new(store),
            events,
            prompts: Mutex::new(HashMap::new()),
            prompt_timeout,
            me: me.clone(),
        })
    }

    /// 连接 conn_id 时使用的参数；遇到未知主机会弹窗询问，提示里显示 conn_name。
    pub fn dial_options(
        &self,
        conn_id: &str,
        conn_name: &str,
        keepalive: Option<Duration>,
    ) -> DialOptions {
        DialOptions {
            host_keys: self.store.clone(),
            trust: Some(self.trust_fn(conn_id, conn_name)),
            timeout: DEFAULT_TIMEOUT,
            keepalive,
        }
    }

    fn trust_fn(&self, conn_id: &str, conn_name: &str) -> TrustFn {
        let me = self.me.clone();
        let (id, name) = (conn_id.to_string(), conn_name.to_string());
        Arc::new(move |q: HostKeyQuery| {
            let me = me.clone();
            let (id, name) = (id.clone(), name.clone());
            Box::pin(async move {
                match me.upgrade() {
                    Some(svc) => svc.ask(id, name, q).await,
                    None => false,
                }
            })
        })
    }

    /// 推送指纹给前端确认，用户回答前一直挂起握手。
    async fn ask(&self, conn_id: String, conn_name: String, q: HostKeyQuery) -> bool {
        let info = HostKeyPrompt {
            request_id: new_id(),
            conn_id,
            conn_name,
            host: q.host,
            key_type: q.key_type,
            fingerprint: q.fingerprint,
        };
        let (tx, rx) = oneshot::channel();
        let rid = info.request_id.clone();
        self.prompts.lock().unwrap().insert(
            rid.clone(),
            Prompt {
                info: info.clone(),
                answer: Some(tx),
            },
        );

        info!(conn = %info.conn_name, host = %info.host, fingerprint = %info.fingerprint, "等待确认主机指纹");
        self.events.show_main_window();
        self.events.emit(
            events::HOST_KEY,
            serde_json::to_value(&info).unwrap_or_default(),
        );

        let ok = tokio::select! {
            r = rx => r.unwrap_or(false),
            _ = q.cancel.cancelled() => false,
            _ = tokio::time::sleep(self.prompt_timeout) => {
                warn!(conn = %info.conn_name, "主机指纹确认超时");
                false
            }
        };
        self.prompts.lock().unwrap().remove(&rid);
        self.events
            .emit(events::HOST_KEY_DONE, serde_json::Value::String(rid));
        ok
    }

    /// 尚未回答的请求；窗口新建后前端会先拉一次，避免错过事件。
    pub fn pending(&self) -> Vec<HostKeyPrompt> {
        self.prompts
            .lock()
            .unwrap()
            .values()
            .map(|p| p.info.clone())
            .collect()
    }

    pub fn answer(&self, request_id: &str, accept: bool) -> ApiResult<()> {
        let tx = self
            .prompts
            .lock()
            .unwrap()
            .get_mut(request_id)
            .and_then(|p| p.answer.take());
        match tx {
            Some(tx) => {
                let _ = tx.send(accept);
                Ok(())
            }
            None => Err(error::HOST_KEY_EXPIRED.err()),
        }
    }
}
