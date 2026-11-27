//! 测试服务器的 SFTP 子系统：远程的 / 映射到本地临时目录，主目录是 /home/tester。不支持符号链接。

use std::collections::HashMap;
use std::fs::{self, File as LocalFile, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use russh_sftp::protocol::{
    Attrs, Data, File, FileAttributes, Handle, Name, OpenFlags, Status, StatusCode,
};

pub const HOME: &str = "/home/tester";

enum Opened {
    File(LocalFile),
    Dir(Vec<File>),
}

pub struct FakeFs {
    pub root: PathBuf,
    /// 每个读写请求额外等待的毫秒数，用来把传输拖慢到能中途取消。
    pub delay_ms: Arc<AtomicU64>,
    handles: HashMap<String, Opened>,
    next: u64,
}

impl FakeFs {
    pub fn new(root: PathBuf, delay_ms: Arc<AtomicU64>) -> FakeFs {
        FakeFs {
            root,
            delay_ms,
            handles: HashMap::new(),
            next: 0,
        }
    }

    /// 规范化远程路径：相对路径基于主目录，处理 . 和 ..。
    fn normalize(path: &str) -> Vec<String> {
        let full = if path.starts_with('/') {
            path.to_string()
        } else {
            format!("{HOME}/{path}")
        };
        let mut parts: Vec<String> = Vec::new();
        for c in full.split('/') {
            match c {
                "" | "." => {}
                ".." => drop(parts.pop()),
                c => parts.push(c.to_string()),
            }
        }
        parts
    }

    fn local(&self, path: &str) -> PathBuf {
        let mut p = self.root.clone();
        p.extend(Self::normalize(path));
        p
    }

    fn add(&mut self, h: Opened) -> String {
        self.next += 1;
        let id = self.next.to_string();
        self.handles.insert(id.clone(), h);
        id
    }

    async fn slow(&self) {
        let ms = self.delay_ms.load(Ordering::SeqCst);
        if ms > 0 {
            tokio::time::sleep(Duration::from_millis(ms)).await;
        }
    }
}

fn ok(id: u32) -> Status {
    Status {
        id,
        status_code: StatusCode::Ok,
        error_message: "Ok".into(),
        language_tag: "en-US".into(),
    }
}

fn code(e: io::Error) -> StatusCode {
    match e.kind() {
        io::ErrorKind::NotFound => StatusCode::NoSuchFile,
        io::ErrorKind::PermissionDenied => StatusCode::PermissionDenied,
        _ => StatusCode::Failure,
    }
}

fn attrs(m: &fs::Metadata) -> FileAttributes {
    FileAttributes::from(m)
}

impl russh_sftp::server::Handler for FakeFs {
    type Error = StatusCode;

    fn unimplemented(&self) -> StatusCode {
        StatusCode::OpUnsupported
    }

    async fn open(
        &mut self,
        id: u32,
        filename: String,
        pflags: OpenFlags,
        _attrs: FileAttributes,
    ) -> Result<Handle, StatusCode> {
        let f = OpenOptions::from(pflags)
            .open(self.local(&filename))
            .map_err(code)?;
        Ok(Handle {
            id,
            handle: self.add(Opened::File(f)),
        })
    }

    async fn close(&mut self, id: u32, handle: String) -> Result<Status, StatusCode> {
        self.handles.remove(&handle).ok_or(StatusCode::Failure)?;
        Ok(ok(id))
    }

    async fn read(
        &mut self,
        id: u32,
        handle: String,
        offset: u64,
        len: u32,
    ) -> Result<Data, StatusCode> {
        self.slow().await;
        let Some(Opened::File(f)) = self.handles.get_mut(&handle) else {
            return Err(StatusCode::Failure);
        };
        f.seek(SeekFrom::Start(offset)).map_err(code)?;
        let mut data = Vec::with_capacity(len as usize);
        f.take(len as u64).read_to_end(&mut data).map_err(code)?;
        if data.is_empty() {
            return Err(StatusCode::Eof);
        }
        Ok(Data { id, data })
    }

    async fn write(
        &mut self,
        id: u32,
        handle: String,
        offset: u64,
        data: Vec<u8>,
    ) -> Result<Status, StatusCode> {
        self.slow().await;
        let Some(Opened::File(f)) = self.handles.get_mut(&handle) else {
            return Err(StatusCode::Failure);
        };
        f.seek(SeekFrom::Start(offset)).map_err(code)?;
        f.write_all(&data).map_err(code)?;
        Ok(ok(id))
    }

    async fn stat(&mut self, id: u32, path: String) -> Result<Attrs, StatusCode> {
        let m = fs::metadata(self.local(&path)).map_err(code)?;
        Ok(Attrs {
            id,
            attrs: attrs(&m),
        })
    }

    async fn lstat(&mut self, id: u32, path: String) -> Result<Attrs, StatusCode> {
        let m = fs::symlink_metadata(self.local(&path)).map_err(code)?;
        Ok(Attrs {
            id,
            attrs: attrs(&m),
        })
    }

    async fn fstat(&mut self, id: u32, handle: String) -> Result<Attrs, StatusCode> {
        let Some(Opened::File(f)) = self.handles.get(&handle) else {
            return Err(StatusCode::Failure);
        };
        Ok(Attrs {
            id,
            attrs: attrs(&f.metadata().map_err(code)?),
        })
    }

    async fn setstat(
        &mut self,
        id: u32,
        _path: String,
        _attrs: FileAttributes,
    ) -> Result<Status, StatusCode> {
        Ok(ok(id))
    }

    async fn fsetstat(
        &mut self,
        id: u32,
        _handle: String,
        _attrs: FileAttributes,
    ) -> Result<Status, StatusCode> {
        Ok(ok(id))
    }

    async fn opendir(&mut self, id: u32, path: String) -> Result<Handle, StatusCode> {
        let dir = self.local(&path);
        if !fs::metadata(&dir).map_err(code)?.is_dir() {
            return Err(StatusCode::Failure);
        }
        let mut files = vec![
            File::new(".", FileAttributes::dummy()),
            File::new("..", FileAttributes::dummy()),
        ];
        for e in fs::read_dir(&dir).map_err(code)? {
            let e = e.map_err(code)?;
            let m = e.metadata().map_err(code)?;
            files.push(File::new(e.file_name().to_string_lossy(), attrs(&m)));
        }
        Ok(Handle {
            id,
            handle: self.add(Opened::Dir(files)),
        })
    }

    /// 和 OpenSSH 一样每次最多返回 100 项，大目录要读多次。
    async fn readdir(&mut self, id: u32, handle: String) -> Result<Name, StatusCode> {
        let Some(Opened::Dir(files)) = self.handles.get_mut(&handle) else {
            return Err(StatusCode::Failure);
        };
        if files.is_empty() {
            return Err(StatusCode::Eof);
        }
        let n = files.len().min(100);
        Ok(Name {
            id,
            files: files.drain(..n).collect(),
        })
    }

    async fn remove(&mut self, id: u32, filename: String) -> Result<Status, StatusCode> {
        fs::remove_file(self.local(&filename)).map_err(code)?;
        Ok(ok(id))
    }

    async fn mkdir(
        &mut self,
        id: u32,
        path: String,
        _attrs: FileAttributes,
    ) -> Result<Status, StatusCode> {
        fs::create_dir(self.local(&path)).map_err(code)?;
        Ok(ok(id))
    }

    async fn rmdir(&mut self, id: u32, path: String) -> Result<Status, StatusCode> {
        fs::remove_dir(self.local(&path)).map_err(code)?;
        Ok(ok(id))
    }

    async fn realpath(&mut self, id: u32, path: String) -> Result<Name, StatusCode> {
        let parts = Self::normalize(&path);
        fs::metadata(self.local(&path)).map_err(code)?;
        Ok(Name {
            id,
            files: vec![File::dummy(format!("/{}", parts.join("/")))],
        })
    }

    async fn rename(
        &mut self,
        id: u32,
        oldpath: String,
        newpath: String,
    ) -> Result<Status, StatusCode> {
        let to = self.local(&newpath);
        if to.exists() {
            return Err(StatusCode::Failure);
        }
        fs::rename(self.local(&oldpath), to).map_err(code)?;
        Ok(ok(id))
    }
}
