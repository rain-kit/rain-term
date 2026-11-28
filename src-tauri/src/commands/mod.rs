//! 前端通过 invoke 调用的接口，前端封装见 src/lib/api.ts。
//! 只返回数据和国际化 key（见 i18n::Msg、error::ApiError），文案由前端翻译。

pub mod accounts;
pub mod conn;
pub mod keys;
pub mod monitor;
pub mod rdp;
pub mod settings;
pub mod sftp;
pub mod ssh;
pub mod update;
pub mod workspace;
