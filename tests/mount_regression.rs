//! Explicit real-kernel check; the ordinary suite must not pretend mount access exists.
#![cfg(all(
    target_os = "linux",
    feature = "tokio-runtime",
    not(feature = "async-io-runtime"),
    feature = "unprivileged"
))]
use bytes::Bytes;
use fuse3::{FileType, MountOptions, Result, notify::Notify, raw::prelude::*};
use futures_util::{Stream, stream};
use std::{
    ffi::{CString, OsStr, OsString},
    io::{Read, Seek, SeekFrom},
    num::NonZeroU32,
    sync::{Arc, Mutex},
    time::{Duration, UNIX_EPOCH},
};
struct State {
    content: Mutex<Vec<u8>>,
    notifier: Mutex<Option<Notify>>,
    ready: tokio::sync::Notify,
}
struct Probe(Arc<State>);
fn attr(ino: u64) -> FileAttr {
    let directory = ino == 1 || ino == 3;
    FileAttr {
        ino,
        size: if ino == 2 { 6 } else { 0 },
        blocks: 1,
        atime: UNIX_EPOCH.into(),
        mtime: UNIX_EPOCH.into(),
        ctime: UNIX_EPOCH.into(),
        kind: if directory {
            FileType::Directory
        } else {
            FileType::RegularFile
        },
        perm: if directory { 0o755 } else { 0o644 },
        nlink: if directory { 2 } else { 1 },
        uid: unsafe { libc::geteuid() },
        gid: unsafe { libc::getegid() },
        rdev: 0,
        blksize: 4096,
    }
}
fn listing(parent: u64) -> Vec<(u64, OsString)> {
    let mut entries = vec![(parent, ".".into()), (1, "..".into())];
    if parent == 1 {
        entries.extend([(2, "file".into()), (3, "entries".into())]);
    } else {
        entries.extend((0..400).map(|i| (100 + i, format!("{i:03}").into())));
    }
    entries
}
impl Filesystem for Probe {
    async fn init(&self, _: Request) -> Result<ReplyInit> {
        Ok(ReplyInit::new(NonZeroU32::new(65536).unwrap()))
    }
    async fn ready(&self, notify: Notify) {
        *self.0.notifier.lock().unwrap() = Some(notify);
        self.0.ready.notify_one();
    }
    async fn destroy(&self, _: Request) {}
    async fn lookup(&self, _: Request, parent: u64, name: &OsStr) -> Result<ReplyEntry> {
        let ino = listing(parent)
            .into_iter()
            .find(|(_, n)| n == name)
            .ok_or(libc::ENOENT)?
            .0;
        Ok(ReplyEntry {
            ttl: Duration::from_secs(60),
            attr: attr(ino),
            generation: 0,
        })
    }
    async fn getattr(&self, _: Request, ino: u64, _: Option<u64>, _: u32) -> Result<ReplyAttr> {
        Ok(ReplyAttr {
            ttl: Duration::from_secs(60),
            attr: attr(ino),
        })
    }
    async fn open(&self, _: Request, _: u64, _: u32) -> Result<ReplyOpen> {
        Ok(ReplyOpen { fh: 1, flags: 0 })
    }
    async fn read(
        &self,
        _: Request,
        ino: u64,
        _: u64,
        offset: u64,
        size: u32,
    ) -> Result<ReplyData> {
        if ino != 2 {
            return Err(libc::EINVAL.into());
        }
        let bytes = self.0.content.lock().unwrap();
        let start = (offset as usize).min(bytes.len());
        let end = start.saturating_add(size as usize).min(bytes.len());
        Ok(ReplyData {
            data: Bytes::copy_from_slice(&bytes[start..end]),
        })
    }
    async fn release(&self, _: Request, _: u64, _: u64, _: u32, _: u64, _: bool) -> Result<()> {
        Ok(())
    }
    async fn flush(&self, _: Request, _: u64, _: u64, _: u64) -> Result<()> {
        Ok(())
    }
    async fn opendir(&self, _: Request, inode: u64, _: u32) -> Result<ReplyOpen> {
        Ok(ReplyOpen {
            fh: inode,
            flags: 0,
        })
    }
    async fn readdir<'a>(
        &'a self,
        _: Request,
        parent: u64,
        _: u64,
        offset: i64,
    ) -> Result<ReplyDirectory<impl Stream<Item = Result<DirectoryEntry>> + Send + 'a>> {
        let rows = listing(parent)
            .into_iter()
            .enumerate()
            .skip(offset as usize)
            .map(|(i, (inode, name))| {
                Ok(DirectoryEntry {
                    inode,
                    kind: attr(inode).kind,
                    name,
                    offset: i as i64 + 1,
                })
            });
        Ok(ReplyDirectory {
            entries: stream::iter(rows),
        })
    }
    async fn readdirplus<'a>(
        &'a self,
        _: Request,
        parent: u64,
        _: u64,
        offset: u64,
        _: u64,
    ) -> Result<ReplyDirectoryPlus<impl Stream<Item = Result<DirectoryEntryPlus>> + Send + 'a>>
    {
        let rows = listing(parent)
            .into_iter()
            .enumerate()
            .skip(offset as usize)
            .map(|(i, (inode, name))| {
                Ok(DirectoryEntryPlus {
                    inode,
                    generation: 0,
                    kind: attr(inode).kind,
                    name,
                    offset: i as i64 + 1,
                    attr: attr(inode),
                    entry_ttl: Duration::ZERO,
                    attr_ttl: Duration::ZERO,
                })
            });
        Ok(ReplyDirectoryPlus {
            entries: stream::iter(rows),
        })
    }
    async fn getxattr(&self, _: Request, _: u64, name: &OsStr, size: u32) -> Result<ReplyXAttr> {
        if name != "user.probe" {
            return Err(libc::ENODATA.into());
        }
        if size == 0 {
            Ok(ReplyXAttr::Size(7))
        } else if size < 7 {
            Err(libc::ERANGE.into())
        } else {
            Ok(ReplyXAttr::Data(Bytes::from_static(b"checked")))
        }
    }
    #[cfg(feature = "file-lock")]
    async fn getlk(
        &self,
        _: Request,
        _: u64,
        _: u64,
        _: u64,
        _: u64,
        _: u64,
        _: u32,
        _: u32,
    ) -> Result<ReplyLock> {
        Err(libc::EOPNOTSUPP.into())
    }
    #[cfg(feature = "file-lock")]
    async fn setlk(
        &self,
        _: Request,
        _: u64,
        _: u64,
        _: u64,
        _: u64,
        _: u64,
        _: u32,
        _: u32,
        _: bool,
    ) -> Result<()> {
        Err(libc::EOPNOTSUPP.into())
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires Linux /dev/fuse and fusermount3; run explicitly with --ignored"]
async fn cache_notification_xattr_and_paginated_directory_on_real_kernel() {
    let directory = tempfile::tempdir().unwrap();
    let state = Arc::new(State {
        content: Mutex::new(b"before".to_vec()),
        notifier: Mutex::new(None),
        ready: tokio::sync::Notify::new(),
    });
    let mut options = MountOptions::default();
    options.fs_name("fuse3-regression").write_back(false);
    let mount = Session::new(options)
        .mount_with_unprivileged(Probe(state.clone()), directory.path())
        .await
        .expect("FUSE mount unavailable: this explicitly requested test cannot count as passed");
    let initialized = tokio::time::timeout(Duration::from_secs(10), state.ready.notified()).await;
    let result = if initialized.is_ok() {
        let root = directory.path().to_path_buf();
        Some(
            tokio::task::spawn_blocking(move || {
                let mut file = std::fs::File::open(root.join("file")).unwrap();
                let mut data = [0; 6];
                file.read_exact(&mut data).unwrap();
                assert_eq!(&data, b"before");
                *state.content.lock().unwrap() = b"after!".to_vec();
                file.seek(SeekFrom::Start(0)).unwrap();
                file.read_exact(&mut data).unwrap();
                assert_eq!(
                    &data, b"before",
                    "test must first demonstrate the cached page"
                );
                let notifier = state.notifier.lock().unwrap().clone().unwrap();
                tokio::runtime::Handle::current().block_on(notifier.invalid_inode(2, 0, 0));
                file.seek(SeekFrom::Start(0)).unwrap();
                file.read_exact(&mut data).unwrap();
                assert_eq!(&data, b"after!");
                drop(file);
                let path = CString::new(root.join("file").as_os_str().as_encoded_bytes()).unwrap();
                let name = CString::new("user.probe").unwrap();
                assert_eq!(
                    unsafe {
                        libc::getxattr(path.as_ptr(), name.as_ptr(), std::ptr::null_mut(), 0)
                    },
                    7
                );
                let mut value = [0u8; 7];
                assert_eq!(
                    unsafe {
                        libc::getxattr(
                            path.as_ptr(),
                            name.as_ptr(),
                            value.as_mut_ptr().cast(),
                            value.len(),
                        )
                    },
                    7
                );
                assert_eq!(&value, b"checked");
                let mut names = std::fs::read_dir(root.join("entries"))
                    .unwrap()
                    .map(|e| e.unwrap().file_name())
                    .collect::<Vec<_>>();
                names.sort();
                assert_eq!(names.len(), 400);
                assert_eq!(names.first().unwrap(), "000");
                assert_eq!(names.last().unwrap(), "399");
                assert_eq!(
                    std::fs::read(root.join("file")).unwrap(),
                    b"after!",
                    "mount must survive the xattr size probe"
                );
            })
            .await,
        )
    } else {
        None
    };
    let unmounted = tokio::time::timeout(Duration::from_secs(10), mount.unmount()).await;
    assert!(initialized.is_ok(), "ready callback did not complete");
    result.expect("mount initialized").unwrap();
    unmounted.unwrap().unwrap();
}
