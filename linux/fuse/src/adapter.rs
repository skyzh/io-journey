use std::ffi::OsStr;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use fuser::consts::FOPEN_DIRECT_IO;
use fuser::{
    FileAttr, FileType, Filesystem, ReplyAttr, ReplyCreate, ReplyData, ReplyDirectory, ReplyEmpty,
    ReplyEntry, ReplyOpen, ReplyStatfs, ReplyWrite, Request, TimeOrNow,
};
use io_journey::journal_fs::{FileInfo, JournalFs, MAX_FILES, MAX_NAME_BYTES, ROOT_INODE};
use io_journey::nand::BLOCK_BYTES;

use crate::disk::FileDisk;

const ATTRIBUTE_TTL: Duration = Duration::ZERO;

pub struct Adapter {
    fs: JournalFs<FileDisk>,
    uid: u32,
    gid: u32,
}

impl Adapter {
    pub fn new(fs: JournalFs<FileDisk>) -> Self {
        let (uid, gid) = unsafe { (libc::geteuid(), libc::getegid()) };
        Self { fs, uid, gid }
    }

    fn attr(&self, info: Option<FileInfo>) -> FileAttr {
        let (ino, size, blocks, mode, nlink, kind) = match info {
            Some(file) => (
                file.ino,
                file.len as u64,
                file.blocks as u64 * 8,
                file.mode,
                u32::from(file.linked),
                FileType::RegularFile,
            ),
            None => (ROOT_INODE, 0, 0, 0o755, 2, FileType::Directory),
        };
        FileAttr {
            ino,
            size,
            blocks,
            kind,
            nlink,
            atime: UNIX_EPOCH,
            mtime: UNIX_EPOCH,
            ctime: UNIX_EPOCH,
            crtime: UNIX_EPOCH,
            perm: mode,
            uid: self.uid,
            gid: self.gid,
            rdev: 0,
            blksize: BLOCK_BYTES as u32,
            flags: 0,
        }
    }
}

impl Filesystem for Adapter {
    fn lookup(&mut self, _request: &Request<'_>, parent: u64, name: &OsStr, reply: ReplyEntry) {
        if parent != ROOT_INODE {
            reply.error(libc::ENOTDIR);
            return;
        }
        match self.fs.lookup(name.as_bytes()) {
            Ok(info) => reply.entry(&ATTRIBUTE_TTL, &self.attr(Some(info)), 0),
            Err(error) => reply.error(errno(error)),
        }
    }

    fn getattr(&mut self, _request: &Request<'_>, ino: u64, _fh: Option<u64>, reply: ReplyAttr) {
        if ino == ROOT_INODE {
            reply.attr(&ATTRIBUTE_TTL, &self.attr(None));
            return;
        }
        match self.fs.stat(ino) {
            Ok(info) => reply.attr(&ATTRIBUTE_TTL, &self.attr(Some(info))),
            Err(error) => reply.error(errno(error)),
        }
    }

    fn setattr(
        &mut self,
        _request: &Request<'_>,
        ino: u64,
        mode: Option<u32>,
        uid: Option<u32>,
        gid: Option<u32>,
        size: Option<u64>,
        atime: Option<TimeOrNow>,
        mtime: Option<TimeOrNow>,
        _ctime: Option<SystemTime>,
        _fh: Option<u64>,
        _crtime: Option<SystemTime>,
        _chgtime: Option<SystemTime>,
        _bkuptime: Option<SystemTime>,
        flags: Option<u32>,
        reply: ReplyAttr,
    ) {
        if mode.is_some()
            || uid.is_some()
            || gid.is_some()
            || flags.is_some()
            || (size.is_none() && (atime.is_some() || mtime.is_some()))
        {
            reply.error(libc::EOPNOTSUPP);
            return;
        }
        let result = size
            .map_or(Ok(()), |size| self.fs.truncate(ino, size))
            .and_then(|()| self.fs.stat(ino));
        match result {
            Ok(info) => reply.attr(&ATTRIBUTE_TTL, &self.attr(Some(info))),
            Err(error) => reply.error(errno(error)),
        }
    }

    fn create(
        &mut self,
        _request: &Request<'_>,
        parent: u64,
        name: &OsStr,
        mode: u32,
        umask: u32,
        _flags: i32,
        reply: ReplyCreate,
    ) {
        if parent != ROOT_INODE {
            reply.error(libc::ENOTDIR);
            return;
        }
        let result = self
            .fs
            .create(name.as_bytes(), (mode & !umask) as u16)
            .and_then(|ino| {
                self.fs.retain_file(ino)?;
                self.fs.stat(ino)
            });
        match result {
            Ok(info) => reply.created(
                &ATTRIBUTE_TTL,
                &self.attr(Some(info)),
                0,
                info.ino,
                FOPEN_DIRECT_IO,
            ),
            Err(error) => reply.error(errno(error)),
        }
    }

    fn open(&mut self, _request: &Request<'_>, ino: u64, _flags: i32, reply: ReplyOpen) {
        match self.fs.retain_file(ino) {
            Ok(()) => reply.opened(ino, FOPEN_DIRECT_IO),
            Err(error) => reply.error(errno(error)),
        }
    }

    fn read(
        &mut self,
        _request: &Request<'_>,
        ino: u64,
        _fh: u64,
        offset: i64,
        size: u32,
        _flags: i32,
        _lock_owner: Option<u64>,
        reply: ReplyData,
    ) {
        let result =
            absolute_offset(offset).and_then(|offset| self.fs.read(ino, offset, size as usize));
        match result {
            Ok(bytes) => reply.data(&bytes),
            Err(error) => reply.error(errno(error)),
        }
    }

    fn write(
        &mut self,
        _request: &Request<'_>,
        ino: u64,
        _fh: u64,
        offset: i64,
        data: &[u8],
        _write_flags: u32,
        flags: i32,
        _lock_owner: Option<u64>,
        reply: ReplyWrite,
    ) {
        let offset = if flags & libc::O_APPEND != 0 {
            self.fs.stat(ino).map(|info| info.len as u64)
        } else {
            absolute_offset(offset)
        };
        match offset.and_then(|offset| self.fs.write(ino, offset, data)) {
            Ok(count) => reply.written(count as u32),
            Err(error) => reply.error(errno(error)),
        }
    }

    fn flush(
        &mut self,
        _request: &Request<'_>,
        _ino: u64,
        _fh: u64,
        _owner: u64,
        reply: ReplyEmpty,
    ) {
        reply.ok();
    }

    fn release(
        &mut self,
        _request: &Request<'_>,
        ino: u64,
        _fh: u64,
        _flags: i32,
        _lock_owner: Option<u64>,
        _flush: bool,
        reply: ReplyEmpty,
    ) {
        self.fs.release_file(ino);
        reply.ok();
    }

    fn fsync(
        &mut self,
        _request: &Request<'_>,
        ino: u64,
        _fh: u64,
        _data_only: bool,
        reply: ReplyEmpty,
    ) {
        reply_empty(self.fs.sync_file(ino), reply);
    }

    fn fsyncdir(
        &mut self,
        _request: &Request<'_>,
        ino: u64,
        _fh: u64,
        _data_only: bool,
        reply: ReplyEmpty,
    ) {
        if ino != ROOT_INODE {
            reply.error(libc::ENOTDIR);
        } else {
            reply_empty(self.fs.sync_dir(), reply);
        }
    }

    fn readdir(
        &mut self,
        _request: &Request<'_>,
        ino: u64,
        _fh: u64,
        offset: i64,
        mut reply: ReplyDirectory,
    ) {
        if ino != ROOT_INODE {
            reply.error(libc::ENOTDIR);
            return;
        }
        let Ok(start) = usize::try_from(offset) else {
            reply.error(libc::EINVAL);
            return;
        };
        match self.fs.entries() {
            Ok(entries) => {
                let root = [
                    (ROOT_INODE, FileType::Directory, OsStr::new(".")),
                    (ROOT_INODE, FileType::Directory, OsStr::new("..")),
                ];
                let files = entries
                    .map(|(name, info)| (info.ino, FileType::RegularFile, OsStr::from_bytes(name)));
                for (index, (ino, kind, name)) in
                    root.into_iter().chain(files).enumerate().skip(start)
                {
                    if reply.add(ino, (index + 1) as i64, kind, name) {
                        break;
                    }
                }
                reply.ok();
            }
            Err(error) => reply.error(errno(error)),
        }
    }

    fn unlink(&mut self, _request: &Request<'_>, parent: u64, name: &OsStr, reply: ReplyEmpty) {
        if parent != ROOT_INODE {
            reply.error(libc::ENOTDIR);
        } else {
            reply_empty(self.fs.unlink(name.as_bytes()), reply);
        }
    }

    fn rename(
        &mut self,
        _request: &Request<'_>,
        parent: u64,
        name: &OsStr,
        new_parent: u64,
        new_name: &OsStr,
        flags: u32,
        reply: ReplyEmpty,
    ) {
        if parent != ROOT_INODE || new_parent != ROOT_INODE {
            reply.error(libc::ENOTDIR);
        } else if flags != 0 && flags != libc::RENAME_NOREPLACE {
            reply.error(libc::EOPNOTSUPP);
        } else {
            reply_empty(
                self.fs
                    .rename(name.as_bytes(), new_name.as_bytes(), flags == 0),
                reply,
            );
        }
    }

    fn statfs(&mut self, _request: &Request<'_>, _ino: u64, reply: ReplyStatfs) {
        match self.fs.entries() {
            Ok(entries) => {
                let free = self.fs.free_blocks() as u64;
                reply.statfs(
                    self.fs.data_blocks() as u64,
                    free,
                    free,
                    MAX_FILES as u64,
                    (MAX_FILES - entries.count()) as u64,
                    BLOCK_BYTES as u32,
                    MAX_NAME_BYTES as u32,
                    BLOCK_BYTES as u32,
                );
            }
            Err(error) => reply.error(errno(error)),
        }
    }
}

fn absolute_offset(offset: i64) -> io::Result<u64> {
    u64::try_from(offset).map_err(|_| crate::invalid("negative file offset"))
}

fn reply_empty(result: io::Result<()>, reply: ReplyEmpty) {
    match result {
        Ok(()) => reply.ok(),
        Err(error) => reply.error(errno(error)),
    }
}

fn errno(error: io::Error) -> i32 {
    error.raw_os_error().unwrap_or_else(|| match error.kind() {
        io::ErrorKind::NotFound => libc::ENOENT,
        io::ErrorKind::AlreadyExists => libc::EEXIST,
        io::ErrorKind::InvalidInput => libc::EINVAL,
        io::ErrorKind::StorageFull => libc::ENOSPC,
        io::ErrorKind::FileTooLarge => libc::EFBIG,
        _ => libc::EIO,
    })
}
