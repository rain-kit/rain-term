use std::sync::{Arc, Mutex};
use std::time::Duration;

use ironrdp_cliprdr::CliprdrClient;
use ironrdp_cliprdr::pdu::{ClipboardFormatId, FormatDataResponse};
use ironrdp_connector::connection_activation::{
    ConnectionActivationFactory, ConnectionActivationSequence, ConnectionActivationState,
};
use ironrdp_core::{IntoOwned, WriteBuf};
use ironrdp_displaycontrol::pdu::MonitorLayoutEntry;
use ironrdp_graphics::image_processing::PixelFormat;
use ironrdp_graphics::pointer::DecodedPointer;
use ironrdp_input::{Database, Operation};
use ironrdp_pdu::PduResult;
use ironrdp_pdu::geometry::InclusiveRectangle;
use ironrdp_session::image::DecodedImage;
use ironrdp_session::{ActiveStage, ActiveStageBuilder, ActiveStageOutput, GracefulDisconnectReason};
use ironrdp_svc::{SvcProcessor, SvcProcessorMessages};
use ironrdp_tokio::FramedWrite;
use tokio::sync::{Notify, mpsc};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;
use tracing::{debug, warn};

use super::clipboard::{ClipEvent, TextClipboard, text_format};
use super::input::{InputEvent, ctrl_alt_del, operations};
use super::{Connected, Framed, boxed};
use crate::i18n::Msg;

/// 合并画面更新的间隔，约 60 帧每秒。
const FLUSH_INTERVAL: Duration = Duration::from_millis(16);
/// 待发送区域超过这么多块时合并成一块。
const MAX_RECTS: usize = 32;

pub const MSG_POINTER: u8 = 2;
pub const MSG_POINTER_DEFAULT: u8 = 3;
pub const MSG_POINTER_HIDDEN: u8 = 4;

/// 前端发给会话的操作。
#[derive(Debug)]
pub enum Command {
    Input(Vec<InputEvent>),
    CtrlAltDel,
    /// 窗口大小变化，服务器支持时调整远程分辨率。
    Resize {
        width: u32,
        height: u32,
        scale: u32,
    },
    /// 窗口失去焦点时松开所有按键，避免远程卡在按下状态。
    ReleaseAll,
    /// 本地剪贴板的文本变了。
    Clipboard(String),
}

/// 会话推给前端的数据；画面不走这里，由前端从 [`Frames`] 取。
#[derive(Debug)]
pub enum Event {
    /// 鼠标指针形状，格式见 [`encode_pointer`]。
    Pointer(Vec<u8>),
    /// 远程复制的文本。
    Clipboard(String),
}

/// 画面信箱：最多放一条，前端取走后才编码下一条，期间的更新合并进去。
/// 前端画上一条的同时，下一条已经在准备，两边都不用等。
#[derive(Default)]
pub struct Frames {
    slot: Mutex<Option<Vec<u8>>>,
    ready: Notify,
    taken: Notify,
}

impl Frames {
    /// 等下一条画面，格式见 [`encode_frame`]。
    pub async fn next(&self) -> Vec<u8> {
        loop {
            if let Some(f) = self.slot.lock().unwrap().take() {
                self.taken.notify_one();
                return f;
            }
            self.ready.notified().await;
        }
    }

    fn full(&self) -> bool {
        self.slot.lock().unwrap().is_some()
    }

    fn put(&self, frame: Vec<u8>) {
        *self.slot.lock().unwrap() = Some(frame);
        self.ready.notify_one();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Rect {
    x: u16,
    y: u16,
    w: u16,
    h: u16,
}

impl Rect {
    fn clip(r: &InclusiveRectangle, width: u16, height: u16) -> Option<Rect> {
        if r.left >= width || r.top >= height || r.right < r.left || r.bottom < r.top {
            return None;
        }
        let right = r.right.min(width - 1);
        let bottom = r.bottom.min(height - 1);
        Some(Rect {
            x: r.left,
            y: r.top,
            w: right - r.left + 1,
            h: bottom - r.top + 1,
        })
    }

    fn area(&self) -> usize {
        self.w as usize * self.h as usize
    }

    fn contains(&self, o: &Rect) -> bool {
        o.x >= self.x
            && o.y >= self.y
            && o.x + o.w <= self.x + self.w
            && o.y + o.h <= self.y + self.h
    }

    fn union(&self, o: &Rect) -> Rect {
        let (x, y) = (self.x.min(o.x), self.y.min(o.y));
        let right = (self.x + self.w).max(o.x + o.w);
        let bottom = (self.y + self.h).max(o.y + o.h);
        Rect {
            x,
            y,
            w: right - x,
            h: bottom - y,
        }
    }
}

/// 还没发给前端的画面区域。
#[derive(Default)]
struct Dirty {
    rects: Vec<Rect>,
    since: Option<Instant>,
}

impl Dirty {
    fn add(&mut self, r: Rect) {
        if self.rects.iter().any(|x| x.contains(&r)) {
            return;
        }
        self.rects.retain(|x| !r.contains(x));
        self.rects.push(r);
        if self.rects.len() > MAX_RECTS {
            let all = self.bounds();
            self.rects = vec![all];
        }
        self.since.get_or_insert_with(Instant::now);
    }

    fn bounds(&self) -> Rect {
        let first = self.rects[0];
        self.rects[1..].iter().fold(first, |a, r| a.union(r))
    }

    /// 取出待发送的区域；零散的小块面积接近外接矩形时直接发外接矩形。
    fn take(&mut self) -> Vec<Rect> {
        self.since = None;
        if self.rects.len() > 1 {
            let all = self.bounds();
            let sum: usize = self.rects.iter().map(Rect::area).sum();
            if sum * 10 >= all.area() * 7 {
                self.rects.clear();
                return vec![all];
            }
        }
        std::mem::take(&mut self.rects)
    }
}

/// `[u16 画面宽][u16 画面高][u16 块数]`，每块 `[u16 x][u16 y][u16 宽][u16 高][RGBA 像素]`，均为小端。
/// 带上画面大小，前端据此调整 canvas，不用依赖另一条通道上的通知先到。
fn encode_frame(image: &DecodedImage, rects: &[Rect]) -> Vec<u8> {
    let size: usize = rects.iter().map(|r| 8 + r.area() * 4).sum();
    let mut out = Vec::with_capacity(6 + size);
    for v in [image.width(), image.height(), rects.len() as u16] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    let (data, stride) = (image.data(), image.stride());
    for r in rects {
        for v in [r.x, r.y, r.w, r.h] {
            out.extend_from_slice(&v.to_le_bytes());
        }
        for row in r.y as usize..(r.y + r.h) as usize {
            let start = row * stride + r.x as usize * 4;
            let at = out.len();
            out.extend_from_slice(&data[start..start + r.w as usize * 4]);
            for px in out[at..].chunks_exact_mut(4) {
                px[3] = 0xFF;
            }
        }
    }
    out
}

/// `[2][u16 宽][u16 高][u16 热点 x][u16 热点 y][RGBA 像素]`，均为小端。
fn encode_pointer(p: &DecodedPointer) -> Vec<u8> {
    let mut out = Vec::with_capacity(9 + p.bitmap_data.len());
    out.push(MSG_POINTER);
    for v in [p.width, p.height, p.hotspot_x, p.hotspot_y] {
        out.extend_from_slice(&v.to_le_bytes());
    }
    out.extend_from_slice(&p.bitmap_data);
    out
}

fn lost(e: impl std::fmt::Display) -> Msg {
    msg!("err.rdp.connectionLost", "远程桌面连接断开").with_cause(e)
}

fn protocol(e: impl std::fmt::Display) -> Msg {
    msg!("err.rdp.protocol", "远程桌面协议错误").with_cause(e)
}

struct Session<E> {
    framed: Framed,
    stage: ActiveStage,
    activation: ConnectionActivationFactory,
    image: DecodedImage,
    input: Database,
    dirty: Dirty,
    /// 上次发出的画面大小，变了就整屏重发。
    size: (u16, u16),
    frames: Arc<Frames>,
    emit: E,
    clipboard: bool,
    local_text: Option<String>,
    /// 等显示控制通道就绪后再发的分辨率。
    pending_resize: Option<(u32, u32, u32)>,
}

/// 会话结束的原因，None 表示是本地关闭的。
type End = Option<Msg>;

impl<E: FnMut(Event) + Send> Session<E> {
    /// 参数用 Vec 而不是切片：带生命周期参数时编译器证明不了 spawn 出去的 future 是 Send。
    async fn send(&mut self, frame: Vec<u8>) -> Result<(), End> {
        self.framed
            .write_all(&frame)
            .await
            .map_err(|e| Some(lost(e)))
    }

    /// 处理服务器数据或本地输入产生的输出；返回 Err 表示会话结束。
    async fn outputs(&mut self, outputs: Vec<ActiveStageOutput>) -> Result<(), End> {
        for out in outputs {
            match out {
                ActiveStageOutput::ResponseFrame(f) => self.send(f).await?,
                ActiveStageOutput::GraphicsUpdate(r) => {
                    if let Some(r) = Rect::clip(&r, self.image.width(), self.image.height()) {
                        self.dirty.add(r);
                    }
                }
                ActiveStageOutput::PointerDefault => {
                    (self.emit)(Event::Pointer(vec![MSG_POINTER_DEFAULT]))
                }
                ActiveStageOutput::PointerHidden => {
                    (self.emit)(Event::Pointer(vec![MSG_POINTER_HIDDEN]))
                }
                ActiveStageOutput::PointerBitmap(p) => (self.emit)(Event::Pointer(encode_pointer(&p))),
                ActiveStageOutput::Terminate(reason) => return Err(terminated(reason)),
                ActiveStageOutput::DeactivateAll => {
                    let mut seq = self.activation.create();
                    self.reactivate(&mut seq).await?;
                }
                ActiveStageOutput::ServerRedirection(_) => {
                    return Err(Some(msg!(
                        "err.rdp.redirect",
                        "服务器要求重定向到其他主机，暂不支持"
                    )));
                }
                other => debug!(?other, "忽略远程桌面输出"),
            }
        }
        Ok(())
    }

    async fn apply(&mut self, ops: Vec<Operation>) -> Result<(), End> {
        let events = self.input.apply(ops);
        let outputs = self
            .stage
            .process_fastpath_input(&mut self.image, &events)
            .map_err(|e| Some(protocol(e.report())))?;
        self.outputs(outputs).await
    }

    async fn release_all(&mut self) -> Result<(), End> {
        let events = self.input.release_all();
        let outputs = self
            .stage
            .process_fastpath_input(&mut self.image, &events)
            .map_err(|e| Some(protocol(e.report())))?;
        self.outputs(outputs).await
    }

    fn flush(&mut self) {
        let rects = self.dirty.take();
        if rects.is_empty() {
            return;
        }
        self.frames.put(encode_frame(&self.image, &rects));
    }

    /// 分辨率调整后（重新激活或图形管道重置）画面被清空，整屏重发。
    fn check_size(&mut self) {
        let size = (self.image.width(), self.image.height());
        if size == self.size {
            return;
        }
        self.size = size;
        self.dirty = Dirty::default();
        if size.0 > 0 && size.1 > 0 {
            self.dirty.add(Rect {
                x: 0,
                y: 0,
                w: size.0,
                h: size.1,
            });
        }
    }

    fn cliprdr(&mut self) -> Option<&mut CliprdrClient> {
        if !self.clipboard {
            return None;
        }
        self.stage.get_svc_processor_mut::<CliprdrClient>()
    }

    /// 处理剪贴板回调记下的事件。
    async fn clipboard_events(&mut self) -> Result<(), End> {
        let events = match self.cliprdr() {
            Some(c) => match c.downcast_backend_mut::<TextClipboard>() {
                Some(b) => b.take_events(),
                None => return Ok(()),
            },
            None => return Ok(()),
        };
        for ev in events {
            match ev {
                ClipEvent::Announce => self.announce().await?,
                ClipEvent::RemoteCopied => {
                    let msgs = self
                        .cliprdr()
                        .map(|c| c.initiate_paste(ClipboardFormatId::CF_UNICODETEXT));
                    self.svc(msgs).await?;
                }
                ClipEvent::DataRequested => {
                    let resp = match &self.local_text {
                        Some(t) => FormatDataResponse::new_unicode_string(t).into_owned(),
                        None => FormatDataResponse::new_error().into_owned(),
                    };
                    let msgs = self.cliprdr().map(|c| c.submit_format_data(resp));
                    self.svc(msgs).await?;
                }
                ClipEvent::RemoteText(text) => {
                    self.local_text = Some(text.clone());
                    (self.emit)(Event::Clipboard(text));
                }
            }
        }
        Ok(())
    }

    /// 告诉服务器本地剪贴板里有文本。
    async fn announce(&mut self) -> Result<(), End> {
        if self.local_text.is_none() {
            return Ok(());
        }
        let msgs = self.cliprdr().and_then(|c| {
            c.is_ready()
                .then(|| c.initiate_copy(&[text_format()]))
        });
        self.svc(msgs).await
    }

    async fn svc<P: SvcProcessor + 'static>(
        &mut self,
        msgs: Option<PduResult<SvcProcessorMessages<P>>>,
    ) -> Result<(), End> {
        let Some(msgs) = msgs else { return Ok(()) };
        let msgs = match msgs {
            Ok(m) => m,
            Err(e) => {
                warn!(err = %e, "剪贴板消息编码失败");
                return Ok(());
            }
        };
        let frame = self
            .stage
            .process_svc_processor_messages(msgs)
            .map_err(|e| Some(protocol(e.report())))?;
        self.send(frame).await
    }

    async fn try_resize(&mut self) -> Result<(), End> {
        let Some((w, h, scale)) = self.pending_resize else {
            return Ok(());
        };
        if self.stage.display_control_ready() != Some(true) {
            return Ok(());
        }
        self.pending_resize = None;
        let (w, h) = MonitorLayoutEntry::adjust_display_size(w, h);
        if (w, h) == (self.image.width() as u32, self.image.height() as u32) {
            return Ok(());
        }
        if let Some(r) = self.stage.encode_resize(w, h, Some(scale), None) {
            let frame = r.map_err(|e| Some(protocol(e.report())))?;
            self.send(frame).await?;
        }
        Ok(())
    }

    /// 服务器调整分辨率等情况下会重新走一遍激活流程。
    async fn reactivate(&mut self, seq: &mut ConnectionActivationSequence) -> Result<(), End> {
        let mut buf = WriteBuf::new();
        loop {
            let written = boxed(ironrdp_tokio::single_sequence_step_read(
                &mut self.framed,
                seq,
                &mut buf,
            ))
            .await
            .map_err(|e| Some(lost(e.report())))?;
            if let Some(n) = written.size() {
                let frame = buf.filled()[..n].to_vec();
                self.send(frame).await?;
            }
            if let ConnectionActivationState::Finalized {
                desktop_size,
                share_id,
                static_channel_chunk_size,
                enable_server_pointer,
                pointer_software_rendering,
                ..
            } = seq.connection_activation_state()
            {
                if !self.stage.reactivate(
                    seq.io_channel_id(),
                    seq.user_channel_id(),
                    share_id,
                    enable_server_pointer,
                    pointer_software_rendering,
                    static_channel_chunk_size,
                ) {
                    return Err(Some(protocol("invalid static channel chunk size")));
                }
                self.image =
                    DecodedImage::new(PixelFormat::RgbA32, desktop_size.width, desktop_size.height);
                self.size = (0, 0);
                return Ok(());
            }
        }
    }

    async fn command(&mut self, cmd: Command) -> Result<(), End> {
        match cmd {
            Command::Input(events) => self.apply(operations(&events)).await,
            Command::CtrlAltDel => self.apply(ctrl_alt_del()).await,
            Command::ReleaseAll => self.release_all().await,
            Command::Resize {
                width,
                height,
                scale,
            } => {
                self.pending_resize = Some((width, height, scale));
                self.try_resize().await
            }
            Command::Clipboard(text) => {
                if self.local_text.as_deref() == Some(text.as_str()) {
                    return Ok(());
                }
                self.local_text = Some(text);
                self.announce().await
            }
        }
    }

    async fn shutdown(&mut self) {
        if let Ok(outputs) = self.stage.graceful_shutdown() {
            for out in outputs {
                if let ActiveStageOutput::ResponseFrame(f) = out {
                    let _ = self.framed.write_all(&f).await;
                }
            }
        }
    }
}

fn terminated(reason: GracefulDisconnectReason) -> End {
    match reason {
        GracefulDisconnectReason::UserInitiated => None,
        GracefulDisconnectReason::ServerInitiated => {
            Some(msg!("err.rdp.serverClosed", "服务器关闭了远程桌面连接"))
        }
        GracefulDisconnectReason::Other(desc) => {
            Some(msg!("err.rdp.sessionEnded", "远程会话已结束").with_cause(desc))
        }
    }
}

/// 运行会话直到结束：转发服务器画面、本地输入和剪贴板。返回结束原因，本地关闭时为 None。
pub async fn run(
    conn: Connected,
    mut commands: mpsc::UnboundedReceiver<Command>,
    emit: impl FnMut(Event) + Send,
    frames: Arc<Frames>,
    cancel: CancellationToken,
) -> Option<Msg> {
    let Connected {
        framed,
        result,
        clipboard,
        ..
    } = conn;
    let size = result.desktop_size;
    let stage = ActiveStageBuilder {
        static_channels: result.static_channels,
        user_channel_id: result.user_channel_id,
        io_channel_id: result.io_channel_id,
        message_channel_id: result.message_channel_id,
        share_id: result.share_id,
        compression_type: result.compression_type,
        enable_server_pointer: result.enable_server_pointer,
        pointer_software_rendering: result.pointer_software_rendering,
    }
    .build();
    let mut s = Session {
        framed,
        stage,
        activation: result.activation_factory,
        image: DecodedImage::new(PixelFormat::RgbA32, size.width, size.height),
        input: Database::new(),
        dirty: Dirty::default(),
        size: (0, 0),
        frames: frames.clone(),
        emit,
        clipboard,
        local_text: None,
        pending_resize: None,
    };

    loop {
        s.check_size();
        let flush_at = s.dirty.since.map(|t| t + FLUSH_INTERVAL);
        let busy = frames.full();
        let r: Result<(), End> = tokio::select! {
            _ = cancel.cancelled() => {
                s.shutdown().await;
                return None;
            }
            r = boxed(s.framed.read_pdu()) => match r {
                Err(e) => Err(Some(lost(e))),
                Ok((action, payload)) => {
                    let at = s.framed.last_read_at();
                    match s.stage.process(&mut s.image, action, &payload, at) {
                        Err(e) => Err(Some(protocol(e.report()))),
                        Ok(outputs) => {
                            let mut r = s.outputs(outputs).await;
                            if r.is_ok() {
                                r = s.clipboard_events().await;
                            }
                            if r.is_ok() {
                                r = s.try_resize().await;
                            }
                            r
                        }
                    }
                }
            },
            cmd = commands.recv() => match cmd {
                Some(cmd) => s.command(cmd).await,
                None => {
                    s.shutdown().await;
                    return None;
                }
            },
            _ = tokio::time::sleep_until(flush_at.unwrap_or_else(Instant::now)), if flush_at.is_some() && !busy => {
                s.flush();
                Ok(())
            }
            _ = frames.taken.notified(), if busy => Ok(()),
        };
        if let Err(end) = r {
            if end.is_none() {
                s.shutdown().await;
            }
            return end;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: u16, y: u16, w: u16, h: u16) -> Rect {
        Rect { x, y, w, h }
    }

    #[test]
    fn clip_rect() {
        let r = InclusiveRectangle {
            left: 10,
            top: 10,
            right: 2000,
            bottom: 19,
        };
        assert_eq!(Rect::clip(&r, 100, 100), Some(rect(10, 10, 90, 10)));
        let out = InclusiveRectangle {
            left: 100,
            top: 0,
            right: 120,
            bottom: 5,
        };
        assert_eq!(Rect::clip(&out, 100, 100), None);
    }

    #[test]
    fn dirty_merges() {
        let mut d = Dirty::default();
        d.add(rect(0, 0, 10, 10));
        d.add(rect(2, 2, 3, 3));
        assert_eq!(d.rects.len(), 1, "被包含的区域不重复记录");
        d.add(rect(90, 90, 10, 10));
        assert_eq!(d.take().len(), 2, "相距很远的两块分开发送");
        assert!(d.since.is_none());

        d.add(rect(0, 0, 10, 10));
        d.add(rect(10, 0, 10, 10));
        assert_eq!(d.take(), vec![rect(0, 0, 20, 10)], "相邻的小块合并");

        for i in 0..=MAX_RECTS as u16 {
            d.add(rect(i * 3, 0, 1, 1));
        }
        assert_eq!(d.rects.len(), 1);
    }

    #[test]
    fn frame_layout() {
        let image = DecodedImage::new(PixelFormat::RgbA32, 4, 2);
        let msg = encode_frame(&image, &[rect(1, 0, 2, 2)]);
        assert_eq!(&msg[0..6], &[4, 0, 2, 0, 1, 0]);
        assert_eq!(&msg[6..14], &[1, 0, 0, 0, 2, 0, 2, 0]);
        assert_eq!(msg.len(), 6 + 8 + 2 * 2 * 4);
        assert!(msg[14..].chunks(4).all(|px| px[3] == 0xFF));
    }

    #[tokio::test]
    async fn frames_mailbox() {
        let f = Arc::new(Frames::default());
        assert!(!f.full());
        let waiter = tokio::spawn({
            let f = f.clone();
            async move { f.next().await }
        });
        tokio::task::yield_now().await;
        f.put(vec![1]);
        assert_eq!(waiter.await.unwrap(), vec![1], "先等后放也能取到");
        assert!(!f.full());

        f.put(vec![2]);
        assert!(f.full(), "取走之前会话不再编码新画面");
        assert_eq!(f.next().await, vec![2]);
        assert!(!f.full());
    }
}
