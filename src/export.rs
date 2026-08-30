use std::fs::{File, OpenOptions};
use std::io::{self, ErrorKind, Read, Seek, SeekFrom, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::os::unix::net::UnixStream;
use std::path::Path;

use crate::protocol::*;

pub(crate) struct EventLog {
    file: File,
    seq: u64,
}

impl EventLog {
    pub(crate) fn new(path: &Path) -> io::Result<Self> {
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)?;
        file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        Ok(Self { file, seq: 0 })
    }

    pub(crate) fn record(
        &mut self,
        op: &str,
        offset: u64,
        length: u32,
        status: &str,
    ) -> io::Result<()> {
        self.seq += 1;
        writeln!(
            self.file,
            "{{\"seq\":{},\"op\":\"{}\",\"offset\":{},\"length\":{},\"status\":\"{}\"}}",
            self.seq, op, offset, length, status
        )?;
        self.file.flush()
    }
}

pub(crate) fn transmission(
    stream: &mut UnixStream,
    image: &mut File,
    export_bytes: u64,
    events: &mut EventLog,
) -> io::Result<()> {
    loop {
        let magic = match get_u32(stream) {
            Ok(value) => value,
            Err(error) if error.kind() == ErrorKind::UnexpectedEof => return Ok(()),
            Err(error) => return Err(error),
        };
        if magic != REQUEST_MAGIC {
            return Err(io::Error::new(
                ErrorKind::InvalidData,
                "invalid request magic",
            ));
        }
        let flags = get_u16(stream)?;
        let command = get_u16(stream)?;
        let mut handle = [0_u8; 8];
        stream.read_exact(&mut handle)?;
        let offset = get_u64(stream)?;
        let length = get_u32(stream)?;
        if command == CMD_DISC {
            events.record("disconnect", offset, length, "ok")?;
            return Ok(());
        }
        let valid_range = offset
            .checked_add(length as u64)
            .is_some_and(|end| end <= export_bytes);
        if flags != 0 || !valid_range {
            events.record(command_name(command), offset, length, "einval")?;
            simple_reply(stream, EINVAL, handle, None)?;
            // An invalid WRITE may declare an arbitrarily large payload. Close
            // this connection after the error rather than buffering or trying
            // to resynchronize an untrusted frame.
            if command == CMD_WRITE {
                return Ok(());
            }
            continue;
        }
        let mut write_payload = Vec::new();
        if command == CMD_WRITE {
            write_payload.resize(length as usize, 0);
            stream.read_exact(&mut write_payload)?;
        }
        match command {
            CMD_READ => {
                let mut bytes = vec![0_u8; length as usize];
                image.seek(SeekFrom::Start(offset))?;
                image.read_exact(&mut bytes)?;
                events.record("read", offset, length, "ok")?;
                simple_reply(stream, 0, handle, Some(&bytes))?;
            }
            CMD_WRITE => {
                image.seek(SeekFrom::Start(offset))?;
                image.write_all(&write_payload)?;
                events.record("write", offset, length, "ok")?;
                simple_reply(stream, 0, handle, None)?;
            }
            CMD_FLUSH if offset == 0 && length == 0 => {
                image.sync_all()?;
                events.record("flush", 0, 0, "ok")?;
                simple_reply(stream, 0, handle, None)?;
            }
            CMD_TRIM => {
                image.seek(SeekFrom::Start(offset))?;
                let zeros = [0_u8; 64 * 1024];
                let mut remaining = length as usize;
                while remaining > 0 {
                    let chunk = remaining.min(zeros.len());
                    image.write_all(&zeros[..chunk])?;
                    remaining -= chunk;
                }
                events.record("trim", offset, length, "ok")?;
                simple_reply(stream, 0, handle, None)?;
            }
            _ => {
                events.record(command_name(command), offset, length, "enotsup")?;
                simple_reply(stream, ENOTSUP, handle, None)?;
            }
        }
    }
}
