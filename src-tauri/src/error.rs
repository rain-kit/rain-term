use std::fmt;

use serde::Serialize;

use crate::i18n::Msg;

/// 返回给前端的错误，序列化为 `{ code, i18n, params }`：code 区分错误类型，i18n 和 params 由前端翻译成文案。
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ApiError {
    pub code: u32,
    #[serde(flatten)]
    pub msg: Msg,
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "错误码:{},消息:{},i18n:{}",
            self.code,
            self.msg.text(),
            self.msg.i18n
        )
    }
}

impl std::error::Error for ApiError {}

pub type ApiResult<T> = Result<T, ApiError>;

/// 错误码定义：码值、国际化 key、中文说明（只用于日志）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Code(pub u32, pub &'static str, pub &'static str);

impl Code {
    pub fn err(self) -> ApiError {
        ApiError {
            code: self.0,
            msg: Msg::new(self.1, self.2.to_string(), Vec::new()),
        }
    }

    /// 附带底层错误原文，前端显示为“错误码文案: 原文”。
    pub fn cause(self, cause: impl fmt::Display) -> ApiError {
        let mut e = self.err();
        e.msg = e.msg.with_cause(cause);
        e
    }

    /// 用更具体的消息代替错误码本身的文案。
    pub fn msg(self, msg: Msg) -> ApiError {
        ApiError { code: self.0, msg }
    }
}

impl From<Code> for ApiError {
    fn from(c: Code) -> Self {
        c.err()
    }
}

impl From<crate::core::ssh::SshError> for ApiError {
    fn from(e: crate::core::ssh::SshError) -> Self {
        SSH_CONNECT_FAILED.msg(e.msg())
    }
}

impl From<crate::core::rdp::RdpError> for ApiError {
    fn from(e: crate::core::rdp::RdpError) -> Self {
        RDP_CONNECT_FAILED.msg(e.msg())
    }
}

// -------------------- 系统 -------------------- //
pub const SERVICE_ERROR: Code = Code(1, "err.service", "系统异常");
pub const PARAM_INVALID: Code = Code(2, "err.paramInvalid", "请求参数错误");
pub const OPEN_DIR_FAILED: Code = Code(3, "err.openDirFailed", "打开目录失败");
pub const OPEN_URL_FAILED: Code = Code(4, "err.openUrlFailed", "打开链接失败");
pub const SECRET_STORE_FAILED: Code = Code(5, "err.secretStoreFailed", "保存密码到系统凭据库失败");
pub const SECRET_READ_FAILED: Code = Code(6, "err.secretReadFailed", "读取系统凭据库失败");
pub const LOG_READ_FAILED: Code = Code(7, "err.logReadFailed", "读取日志失败");
pub const OS_AUTH_FAILED: Code = Code(8, "err.osAuthFailed", "验证电脑密码失败");

// -------------------- 连接 100000 -------------------- //
pub const CONN_NOT_FOUND: Code = Code(100001, "err.connNotFound", "连接不存在");
pub const CONN_INVALID: Code = Code(100002, "err.connInvalid", "连接配置无效");
pub const CONN_FILE_INVALID: Code =
    Code(100003, "err.connFileInvalid", "connections.json 格式错误");
pub const CONN_FILE_IO: Code = Code(100004, "err.connFileIo", "读写 connections.json 失败");
pub const CONN_IN_USE: Code = Code(100005, "err.connInUse", "连接正被其他连接使用");
pub const IMPORT_FAILED: Code = Code(100008, "err.importFailed", "导入失败");
pub const EXPORT_FAILED: Code = Code(100009, "err.exportFailed", "导出失败");

// -------------------- 设置 200000 -------------------- //
pub const SETTING_SAVE_FAILED: Code = Code(200001, "err.settingSaveFailed", "保存设置失败");
pub const UPDATE_CHECK_FAILED: Code = Code(200003, "err.updateCheckFailed", "检查更新失败");
pub const UPDATE_INSTALL_FAILED: Code = Code(200004, "err.updateInstallFailed", "更新失败");
pub const UPDATE_NOT_AVAILABLE: Code = Code(
    200005,
    "err.updateNotAvailable",
    "没有可直接安装的新版本，请前往下载页更新",
);
pub const UPDATE_JUMP_FAILED: Code =
    Code(200006, "err.updateJumpFailed", "经跳板机访问 GitHub 失败");

// -------------------- SSH 300000 -------------------- //
pub const SSH_CONNECT_FAILED: Code = Code(300001, "err.sshConnectFailed", "SSH 连接失败");
pub const HOST_KEY_EXPIRED: Code = Code(300002, "err.hostKeyExpired", "指纹确认请求已过期");
pub const SESSION_NOT_FOUND: Code = Code(300004, "err.sessionNotFound", "终端会话不存在或已关闭");
pub const SESSION_WRITE_FAILED: Code = Code(300005, "err.sessionWriteFailed", "发送到终端失败");

// -------------------- SFTP 400000 -------------------- //
pub const SFTP_OPEN_FAILED: Code = Code(400001, "err.sftpOpenFailed", "打开 SFTP 失败");
pub const SFTP_NOT_FOUND: Code = Code(400002, "err.sftpNotFound", "SFTP 会话不存在或已关闭");
pub const SFTP_FAILED: Code = Code(400003, "err.sftpFailed", "SFTP 操作失败");

// -------------------- 工作区 500000 -------------------- //
pub const WORKSPACE_SAVE_FAILED: Code =
    Code(500001, "err.workspaceSaveFailed", "保存打开的标签失败");

// -------------------- 远程桌面 600000 -------------------- //
pub const RDP_CONNECT_FAILED: Code = Code(600001, "err.rdpConnectFailed", "远程桌面连接失败");
pub const RDP_NOT_FOUND: Code = Code(600002, "err.rdpNotFound", "远程桌面会话不存在或已关闭");
