use ironrdp_cliprdr::backend::CliprdrBackend;
use ironrdp_cliprdr::pdu::{
    ClipboardFormat, ClipboardFormatId, ClipboardGeneralCapabilityFlags, FileContentsRequest,
    FileContentsResponse, FormatDataRequest, FormatDataResponse, LockDataId,
};

/// 剪贴板通道上需要会话循环处理的事情。
#[derive(Debug, PartialEq)]
pub enum ClipEvent {
    /// 通道就绪或服务器要求重新通告本地剪贴板的格式。
    Announce,
    /// 远程复制了文本，需要取回内容。
    RemoteCopied,
    /// 远程要粘贴本地剪贴板的文本。
    DataRequested,
    /// 取回的远程文本。
    RemoteText(String),
}

/// 只同步文本的剪贴板。回调里不能直接回复服务器，先记下事件，由会话循环在处理完每个数据包后取走。
#[derive(Debug, Default)]
pub struct TextClipboard {
    events: Vec<ClipEvent>,
}

ironrdp_core::impl_as_any!(TextClipboard);

impl TextClipboard {
    pub fn take_events(&mut self) -> Vec<ClipEvent> {
        std::mem::take(&mut self.events)
    }
}

pub fn text_format() -> ClipboardFormat {
    ClipboardFormat::new(ClipboardFormatId::CF_UNICODETEXT)
}

impl CliprdrBackend for TextClipboard {
    fn temporary_directory(&self) -> &str {
        ""
    }

    fn client_capabilities(&self) -> ClipboardGeneralCapabilityFlags {
        ClipboardGeneralCapabilityFlags::USE_LONG_FORMAT_NAMES
    }

    fn on_ready(&mut self) {}

    fn on_request_format_list(&mut self) {
        self.events.push(ClipEvent::Announce);
    }

    fn on_process_negotiated_capabilities(&mut self, _: ClipboardGeneralCapabilityFlags) {}

    fn on_remote_copy(&mut self, formats: &[ClipboardFormat]) {
        if formats
            .iter()
            .any(|f| f.id() == ClipboardFormatId::CF_UNICODETEXT)
        {
            self.events.push(ClipEvent::RemoteCopied);
        }
    }

    fn on_format_data_request(&mut self, request: FormatDataRequest) {
        if request.format == ClipboardFormatId::CF_UNICODETEXT {
            self.events.push(ClipEvent::DataRequested);
        }
    }

    fn on_format_data_response(&mut self, response: FormatDataResponse<'_>) {
        if response.is_error() {
            return;
        }
        if let Ok(text) = response.to_unicode_string() {
            self.events.push(ClipEvent::RemoteText(text));
        }
    }

    fn on_file_contents_request(&mut self, _: FileContentsRequest) {}

    fn on_file_contents_response(&mut self, _: FileContentsResponse<'_>) {}

    fn on_lock(&mut self, _: LockDataId) {}

    fn on_unlock(&mut self, _: LockDataId) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_text_events_only() {
        let mut c = TextClipboard::default();
        c.on_remote_copy(&[ClipboardFormat::new(ClipboardFormatId::CF_DIB)]);
        c.on_remote_copy(&[text_format()]);
        c.on_format_data_response(FormatDataResponse::new_unicode_string("你好"));
        c.on_format_data_response(FormatDataResponse::new_error());
        assert_eq!(
            c.take_events(),
            vec![ClipEvent::RemoteCopied, ClipEvent::RemoteText("你好".into())]
        );
        assert!(c.take_events().is_empty());
    }
}
