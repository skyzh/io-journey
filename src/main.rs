use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{self, ErrorKind, Read, Seek, SeekFrom, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

const NBD_MAGIC: u64 = 0x4e42444d41474943;
const IHAVEOPT: u64 = 0x49484156454f5054;
const OPTION_REPLY_MAGIC: u64 = 0x0003e889045565a9;
const REQUEST_MAGIC: u32 = 0x25609513;
const SIMPLE_REPLY_MAGIC: u32 = 0x67446698;

const FLAG_FIXED_NEWSTYLE: u16 = 1;
const FLAG_NO_ZEROES: u16 = 2;
const CLIENT_FIXED_NEWSTYLE: u32 = 1;
const CLIENT_NO_ZEROES: u32 = 2;

const OPT_EXPORT_NAME: u32 = 1;
const OPT_ABORT: u32 = 2;
const OPT_LIST: u32 = 3;
const OPT_INFO: u32 = 6;
const OPT_GO: u32 = 7;
const REP_ACK: u32 = 1;
const REP_SERVER: u32 = 2;
const REP_INFO: u32 = 3;
const REP_ERR_UNSUP: u32 = 0x8000_0001;

const INFO_EXPORT: u16 = 0;
const INFO_BLOCK_SIZE: u16 = 3;

const TRANSMISSION_HAS_FLAGS: u16 = 1;
const TRANSMISSION_SEND_FLUSH: u16 = 1 << 2;
const TRANSMISSION_SEND_TRIM: u16 = 1 << 5;
const TRANSMISSION_FLAGS: u16 =
    TRANSMISSION_HAS_FLAGS | TRANSMISSION_SEND_FLUSH | TRANSMISSION_SEND_TRIM;

const CMD_READ: u16 = 0;
const CMD_WRITE: u16 = 1;
const CMD_DISC: u16 = 2;
const CMD_FLUSH: u16 = 3;
const CMD_TRIM: u16 = 4;

const EINVAL: u32 = 22;
const ENOTSUP: u32 = 95;
const DEFAULT_EXPORT_BYTES: u64 = 1024 * 1024;
const BLOCK: usize = 4096;

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn run() -> io::Result<()> {
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        Some("serve") => {
            let root = required_path(args.next(), "serve ROOT [CONNECTIONS]")?;
            let connections = args
                .next()
                .map(|value| value.parse::<usize>())
                .transpose()
                .map_err(|_| io::Error::new(ErrorKind::InvalidInput, "invalid connection count"))?
                .unwrap_or(1);
            ensure_no_extra(args)?;
            serve(&root, connections)
        }
        Some("probe") => {
            let socket = required_path(args.next(), "probe SOCKET")?;
            ensure_no_extra(args)?;
            probe(&socket)
        }
        Some("verify") => {
            let root = required_path(args.next(), "verify NEW_ROOT")?;
            ensure_no_extra(args)?;
            verify(&root)
        }
        _ => Err(io::Error::new(
            ErrorKind::InvalidInput,
            "usage: rust-nbd-lab {serve ROOT [CONNECTIONS]|probe SOCKET|verify NEW_ROOT}",
        )),
    }
}

fn required_path(value: Option<String>, usage: &str) -> io::Result<PathBuf> {
    value.map(PathBuf::from).ok_or_else(|| {
        io::Error::new(
            ErrorKind::InvalidInput,
            format!("usage: rust-nbd-lab {usage}"),
        )
    })
}

fn ensure_no_extra(mut args: impl Iterator<Item = String>) -> io::Result<()> {
    if args.next().is_some() {
        return Err(io::Error::new(
            ErrorKind::InvalidInput,
            "unexpected argument",
        ));
    }
    Ok(())
}

fn path_entry_exists(path: &Path) -> io::Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

fn verify(root: &Path) -> io::Result<()> {
    if path_entry_exists(root)? {
        return Err(io::Error::new(
            ErrorKind::AlreadyExists,
            "verification root must not already exist",
        ));
    }
    fs::create_dir(root)?;
    fs::set_permissions(root, fs::Permissions::from_mode(0o700))?;
    let socket = root.join("nbd.sock");
    let image = root.join("backing.img");
    let events = root.join("events.jsonl");
    let executable = env::current_exe()?;
    let mut child = Command::new(executable)
        .arg("serve")
        .arg(root)
        .arg("1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()?;

    let mut ready = false;
    for _ in 0..200 {
        if socket.exists() {
            ready = true;
            break;
        }
        if child.try_wait()?.is_some() {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }
    if !ready {
        let _ = child.kill();
        let _ = child.wait();
        return Err(io::Error::new(
            ErrorKind::TimedOut,
            "server socket was not ready",
        ));
    }

    let probe_result = probe(&socket);
    if probe_result.is_err() {
        let _ = child.kill();
    }
    let status = child.wait()?;
    probe_result?;
    if !status.success() {
        return Err(io::Error::other(format!("server exited with {status}")));
    }
    if socket.exists() {
        return Err(io::Error::other("server left its socket behind"));
    }
    let metadata = fs::metadata(&image)?;
    if metadata.len() != DEFAULT_EXPORT_BYTES {
        return Err(io::Error::other("backing file size changed"));
    }
    let mut first_block = vec![0_u8; BLOCK];
    File::open(&image)?.read_exact(&mut first_block)?;
    if first_block.iter().any(|byte| *byte != 0) {
        return Err(io::Error::other(
            "chosen trim policy did not zero the block",
        ));
    }
    let event_bytes = fs::read(&events)?;
    let event_text = String::from_utf8(event_bytes)
        .map_err(|_| io::Error::new(ErrorKind::InvalidData, "event log is not UTF-8"))?;
    let expected = [
        "\"op\":\"write\"",
        "\"op\":\"flush\"",
        "\"op\":\"read\"",
        "\"op\":\"trim\"",
        "\"op\":\"flush\"",
        "\"op\":\"read\"",
        "\"op\":\"disconnect\"",
    ];
    let mut cursor = 0;
    for marker in expected {
        let relative = event_text[cursor..]
            .find(marker)
            .ok_or_else(|| io::Error::other(format!("missing event marker {marker}")))?;
        cursor += relative + marker.len();
    }
    println!(
        "{{\"check\":\"verify\",\"export_bytes\":{DEFAULT_EXPORT_BYTES},\"event_count\":7,\"read_bytes\":8192,\"write_bytes\":4096,\"trim_bytes\":4096,\"flushes\":2,\"trim_policy\":\"deterministic_zero_readback\",\"socket_removed\":true,\"backing_size_preserved\":true}}"
    );
    Ok(())
}

struct EventLog {
    file: File,
    seq: u64,
}

impl EventLog {
    fn new(path: &Path) -> io::Result<Self> {
        let file = OpenOptions::new().write(true).create_new(true).open(path)?;
        Ok(Self { file, seq: 0 })
    }

    fn record(&mut self, op: &str, offset: u64, length: u32, status: &str) -> io::Result<()> {
        self.seq += 1;
        writeln!(
            self.file,
            "{{\"seq\":{},\"op\":\"{}\",\"offset\":{},\"length\":{},\"status\":\"{}\"}}",
            self.seq, op, offset, length, status
        )?;
        self.file.flush()
    }
}

fn serve(root: &Path, connections: usize) -> io::Result<()> {
    if connections == 0 {
        return Err(io::Error::new(
            ErrorKind::InvalidInput,
            "connection count must be positive",
        ));
    }
    let metadata = fs::symlink_metadata(root)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(io::Error::new(
            ErrorKind::InvalidInput,
            "root must be a real directory",
        ));
    }
    let socket = root.join("nbd.sock");
    let image_path = root.join("backing.img");
    let events_path = root.join("events.jsonl");
    for path in [&socket, &image_path, &events_path] {
        if path_entry_exists(path)? {
            return Err(io::Error::new(
                ErrorKind::AlreadyExists,
                format!("refusing to overwrite {}", path.display()),
            ));
        }
    }
    let mut image = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&image_path)?;
    image.set_len(DEFAULT_EXPORT_BYTES)?;
    image.sync_all()?;
    let listener = UnixListener::bind(&socket)?;
    let mut events = EventLog::new(&events_path)?;
    let result = (|| {
        for _ in 0..connections {
            let (mut stream, _) = listener.accept()?;
            if negotiate(&mut stream, DEFAULT_EXPORT_BYTES)? {
                transmission(&mut stream, &mut image, DEFAULT_EXPORT_BYTES, &mut events)?;
            }
        }
        image.sync_all()
    })();
    drop(listener);
    let remove_result = fs::remove_file(&socket);
    result.and(remove_result)
}

fn negotiate(stream: &mut UnixStream, export_bytes: u64) -> io::Result<bool> {
    put_u64(stream, NBD_MAGIC)?;
    put_u64(stream, IHAVEOPT)?;
    put_u16(stream, FLAG_FIXED_NEWSTYLE | FLAG_NO_ZEROES)?;
    stream.flush()?;
    let client_flags = get_u32(stream)?;
    let known = CLIENT_FIXED_NEWSTYLE | CLIENT_NO_ZEROES;
    if client_flags & !known != 0 || client_flags & CLIENT_FIXED_NEWSTYLE == 0 {
        return Err(io::Error::new(
            ErrorKind::InvalidData,
            "unsupported client flags",
        ));
    }
    loop {
        if get_u64(stream)? != IHAVEOPT {
            return Err(io::Error::new(
                ErrorKind::InvalidData,
                "invalid option magic",
            ));
        }
        let option = get_u32(stream)?;
        let length = get_u32(stream)? as usize;
        if length > 1024 * 1024 {
            return Err(io::Error::new(
                ErrorKind::InvalidData,
                "option payload too large",
            ));
        }
        let mut payload = vec![0_u8; length];
        stream.read_exact(&mut payload)?;
        match option {
            OPT_EXPORT_NAME => {
                put_u64(stream, export_bytes)?;
                put_u16(stream, TRANSMISSION_FLAGS)?;
                if client_flags & CLIENT_NO_ZEROES == 0 {
                    stream.write_all(&[0_u8; 124])?;
                }
                stream.flush()?;
                return Ok(true);
            }
            OPT_ABORT => {
                option_reply(stream, option, REP_ACK, &[])?;
                return Ok(false);
            }
            OPT_LIST => {
                let mut item = Vec::new();
                item.extend_from_slice(&0_u32.to_be_bytes());
                option_reply(stream, option, REP_SERVER, &item)?;
                option_reply(stream, option, REP_ACK, &[])?;
            }
            OPT_INFO | OPT_GO => {
                let requested = parse_info_requests(&payload)?;
                let mut export = Vec::with_capacity(12);
                export.extend_from_slice(&INFO_EXPORT.to_be_bytes());
                export.extend_from_slice(&export_bytes.to_be_bytes());
                export.extend_from_slice(&TRANSMISSION_FLAGS.to_be_bytes());
                option_reply(stream, option, REP_INFO, &export)?;
                if requested.contains(&INFO_BLOCK_SIZE) {
                    let mut block = Vec::with_capacity(14);
                    block.extend_from_slice(&INFO_BLOCK_SIZE.to_be_bytes());
                    block.extend_from_slice(&1_u32.to_be_bytes());
                    block.extend_from_slice(&(BLOCK as u32).to_be_bytes());
                    block.extend_from_slice(&(32_u32 * 1024 * 1024).to_be_bytes());
                    option_reply(stream, option, REP_INFO, &block)?;
                }
                option_reply(stream, option, REP_ACK, &[])?;
                if option == OPT_GO {
                    return Ok(true);
                }
            }
            _ => option_reply(stream, option, REP_ERR_UNSUP, &[])?,
        }
    }
}

fn parse_info_requests(payload: &[u8]) -> io::Result<Vec<u16>> {
    if payload.len() < 6 {
        return Err(io::Error::new(
            ErrorKind::InvalidData,
            "short INFO/GO payload",
        ));
    }
    let name_len = u32::from_be_bytes(payload[0..4].try_into().unwrap()) as usize;
    if name_len > payload.len() - 6 {
        return Err(io::Error::new(
            ErrorKind::InvalidData,
            "invalid export name length",
        ));
    }
    let count_at = 4 + name_len;
    let count = u16::from_be_bytes(payload[count_at..count_at + 2].try_into().unwrap()) as usize;
    let list = &payload[count_at + 2..];
    if list.len() != count * 2 {
        return Err(io::Error::new(
            ErrorKind::InvalidData,
            "invalid INFO request count",
        ));
    }
    Ok(list
        .chunks_exact(2)
        .map(|chunk| u16::from_be_bytes([chunk[0], chunk[1]]))
        .collect())
}

fn option_reply(
    stream: &mut UnixStream,
    option: u32,
    reply: u32,
    payload: &[u8],
) -> io::Result<()> {
    put_u64(stream, OPTION_REPLY_MAGIC)?;
    put_u32(stream, option)?;
    put_u32(stream, reply)?;
    put_u32(stream, payload.len() as u32)?;
    stream.write_all(payload)?;
    stream.flush()
}

fn transmission(
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

fn command_name(command: u16) -> &'static str {
    match command {
        CMD_READ => "read",
        CMD_WRITE => "write",
        CMD_DISC => "disconnect",
        CMD_FLUSH => "flush",
        CMD_TRIM => "trim",
        _ => "unknown",
    }
}

fn simple_reply(
    stream: &mut UnixStream,
    error: u32,
    handle: [u8; 8],
    payload: Option<&[u8]>,
) -> io::Result<()> {
    put_u32(stream, SIMPLE_REPLY_MAGIC)?;
    put_u32(stream, error)?;
    stream.write_all(&handle)?;
    if error == 0
        && let Some(bytes) = payload
    {
        stream.write_all(bytes)?;
    }
    stream.flush()
}

fn connect_probe(socket: &Path) -> io::Result<UnixStream> {
    let mut stream = UnixStream::connect(socket)?;
    if get_u64(&mut stream)? != NBD_MAGIC || get_u64(&mut stream)? != IHAVEOPT {
        return Err(io::Error::new(
            ErrorKind::InvalidData,
            "invalid server handshake",
        ));
    }
    let server_flags = get_u16(&mut stream)?;
    let required = FLAG_FIXED_NEWSTYLE | FLAG_NO_ZEROES;
    if server_flags & required != required {
        return Err(io::Error::new(
            ErrorKind::InvalidData,
            "missing handshake flags",
        ));
    }
    put_u32(&mut stream, CLIENT_FIXED_NEWSTYLE | CLIENT_NO_ZEROES)?;
    put_u64(&mut stream, IHAVEOPT)?;
    put_u32(&mut stream, OPT_GO)?;
    put_u32(&mut stream, 8)?;
    put_u32(&mut stream, 0)?;
    put_u16(&mut stream, 1)?;
    put_u16(&mut stream, INFO_BLOCK_SIZE)?;
    stream.flush()?;

    let export_reply = get_option_reply(&mut stream, OPT_GO, REP_INFO)?;
    if export_reply.len() != 12 || get_slice_u16(&export_reply[0..2]) != INFO_EXPORT {
        return Err(io::Error::new(
            ErrorKind::InvalidData,
            "invalid NBD_INFO_EXPORT reply",
        ));
    }
    let export_bytes = get_slice_u64(&export_reply[2..10]);
    let transmission_flags = get_slice_u16(&export_reply[10..12]);
    let block_reply = get_option_reply(&mut stream, OPT_GO, REP_INFO)?;
    if block_reply.len() != 14
        || get_slice_u16(&block_reply[0..2]) != INFO_BLOCK_SIZE
        || get_slice_u32(&block_reply[2..6]) != 1
        || get_slice_u32(&block_reply[6..10]) != BLOCK as u32
        || get_slice_u32(&block_reply[10..14]) != 32 * 1024 * 1024
    {
        return Err(io::Error::new(
            ErrorKind::InvalidData,
            "invalid NBD_INFO_BLOCK_SIZE reply",
        ));
    }
    if !get_option_reply(&mut stream, OPT_GO, REP_ACK)?.is_empty() {
        return Err(io::Error::new(
            ErrorKind::InvalidData,
            "NBD_REP_ACK carried a payload",
        ));
    }
    if export_bytes != DEFAULT_EXPORT_BYTES || transmission_flags != TRANSMISSION_FLAGS {
        return Err(io::Error::new(
            ErrorKind::InvalidData,
            "unexpected export contract",
        ));
    }

    Ok(stream)
}

fn probe(socket: &Path) -> io::Result<()> {
    let mut stream = connect_probe(socket)?;

    let written = vec![0xa5_u8; BLOCK];
    request(&mut stream, 1, CMD_WRITE, 0, BLOCK as u32, Some(&written))?;
    request(&mut stream, 2, CMD_FLUSH, 0, 0, None)?;
    let read = request(&mut stream, 3, CMD_READ, 0, BLOCK as u32, None)?;
    if read != written {
        return Err(io::Error::other("write was not visible to read"));
    }
    request(&mut stream, 4, CMD_TRIM, 0, BLOCK as u32, None)?;
    request(&mut stream, 5, CMD_FLUSH, 0, 0, None)?;
    let trimmed = request(&mut stream, 6, CMD_READ, 0, BLOCK as u32, None)?;
    if trimmed.iter().any(|byte| *byte != 0) {
        return Err(io::Error::other("chosen trim policy did not read as zero"));
    }
    send_request_header(&mut stream, 7, CMD_DISC, 0, 0)?;
    stream.flush()?;
    println!(
        "{{\"check\":\"probe\",\"export_bytes\":{DEFAULT_EXPORT_BYTES},\"transmission_flags\":{TRANSMISSION_FLAGS},\"write_visible\":true,\"trim_policy\":\"deterministic_zero_readback\",\"trim_zero_readback\":true}}"
    );
    Ok(())
}

fn get_option_reply(
    stream: &mut UnixStream,
    expected_option: u32,
    expected_reply: u32,
) -> io::Result<Vec<u8>> {
    if get_u64(stream)? != OPTION_REPLY_MAGIC
        || get_u32(stream)? != expected_option
        || get_u32(stream)? != expected_reply
    {
        return Err(io::Error::new(
            ErrorKind::InvalidData,
            "invalid option reply header",
        ));
    }
    let length = get_u32(stream)? as usize;
    if length > 1024 * 1024 {
        return Err(io::Error::new(
            ErrorKind::InvalidData,
            "option reply payload too large",
        ));
    }
    let mut payload = vec![0_u8; length];
    stream.read_exact(&mut payload)?;
    Ok(payload)
}

fn get_slice_u16(bytes: &[u8]) -> u16 {
    u16::from_be_bytes(bytes.try_into().unwrap())
}

fn get_slice_u32(bytes: &[u8]) -> u32 {
    u32::from_be_bytes(bytes.try_into().unwrap())
}

fn get_slice_u64(bytes: &[u8]) -> u64 {
    u64::from_be_bytes(bytes.try_into().unwrap())
}

fn request(
    stream: &mut UnixStream,
    cookie: u64,
    command: u16,
    offset: u64,
    length: u32,
    payload: Option<&[u8]>,
) -> io::Result<Vec<u8>> {
    send_request_header(stream, cookie, command, offset, length)?;
    if let Some(bytes) = payload {
        if bytes.len() != length as usize {
            return Err(io::Error::new(
                ErrorKind::InvalidInput,
                "payload length mismatch",
            ));
        }
        stream.write_all(bytes)?;
    }
    stream.flush()?;
    if get_u32(stream)? != SIMPLE_REPLY_MAGIC {
        return Err(io::Error::new(
            ErrorKind::InvalidData,
            "invalid reply magic",
        ));
    }
    let error = get_u32(stream)?;
    let mut handle = [0_u8; 8];
    stream.read_exact(&mut handle)?;
    if u64::from_be_bytes(handle) != cookie {
        return Err(io::Error::new(
            ErrorKind::InvalidData,
            "reply cookie mismatch",
        ));
    }
    if error != 0 {
        return Err(io::Error::other(format!(
            "NBD request failed with error {error}"
        )));
    }
    let mut result = Vec::new();
    if command == CMD_READ {
        result.resize(length as usize, 0);
        stream.read_exact(&mut result)?;
    }
    Ok(result)
}

fn send_request_header(
    stream: &mut UnixStream,
    cookie: u64,
    command: u16,
    offset: u64,
    length: u32,
) -> io::Result<()> {
    put_u32(stream, REQUEST_MAGIC)?;
    put_u16(stream, 0)?;
    put_u16(stream, command)?;
    stream.write_all(&cookie.to_be_bytes())?;
    put_u64(stream, offset)?;
    put_u32(stream, length)
}

fn put_u16(writer: &mut impl Write, value: u16) -> io::Result<()> {
    writer.write_all(&value.to_be_bytes())
}

fn put_u32(writer: &mut impl Write, value: u32) -> io::Result<()> {
    writer.write_all(&value.to_be_bytes())
}

fn put_u64(writer: &mut impl Write, value: u64) -> io::Result<()> {
    writer.write_all(&value.to_be_bytes())
}

fn get_u16(reader: &mut impl Read) -> io::Result<u16> {
    let mut bytes = [0_u8; 2];
    reader.read_exact(&mut bytes)?;
    Ok(u16::from_be_bytes(bytes))
}

fn get_u32(reader: &mut impl Read) -> io::Result<u32> {
    let mut bytes = [0_u8; 4];
    reader.read_exact(&mut bytes)?;
    Ok(u32::from_be_bytes(bytes))
}

fn get_u64(reader: &mut impl Read) -> io::Result<u64> {
    let mut bytes = [0_u8; 8];
    reader.read_exact(&mut bytes)?;
    Ok(u64::from_be_bytes(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_ROOT: AtomicU64 = AtomicU64::new(1);

    fn unique_path(label: &str) -> PathBuf {
        env::temp_dir().join(format!(
            "io-journey-nbd-test-{}-{label}-{}",
            std::process::id(),
            NEXT_ROOT.fetch_add(1, Ordering::Relaxed)
        ))
    }

    fn fresh_root(label: &str) -> PathBuf {
        let root = unique_path(label);
        assert!(!path_entry_exists(&root).unwrap());
        fs::create_dir(&root).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        root
    }

    fn wait_for_socket(root: &Path) {
        for _ in 0..200 {
            if root.join("nbd.sock").exists() {
                return;
            }
            thread::sleep(Duration::from_millis(5));
        }
        panic!("server socket was not ready");
    }

    #[test]
    fn serve_rejects_a_symlink_root() {
        let target = fresh_root("symlink-target");
        let link = unique_path("symlink-root");
        symlink(&target, &link).unwrap();

        let error = serve(&link, 1).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidInput);
        assert!(!target.join("backing.img").exists());

        fs::remove_file(link).unwrap();
        fs::remove_dir(target).unwrap();
    }

    #[test]
    fn serve_refuses_a_preexisting_runtime_path() {
        let root = fresh_root("existing-path");
        File::create(root.join("backing.img")).unwrap();

        let error = serve(&root, 1).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::AlreadyExists);
        assert!(!root.join("nbd.sock").exists());
        assert!(!root.join("events.jsonl").exists());

        fs::remove_file(root.join("backing.img")).unwrap();
        fs::remove_dir(root).unwrap();
    }

    #[test]
    fn out_of_range_request_is_rejected_without_growing_the_export() {
        let root = fresh_root("bounds");
        let server_root = root.clone();
        let server = thread::spawn(move || serve(&server_root, 1));
        wait_for_socket(&root);

        let mut stream = connect_probe(&root.join("nbd.sock")).unwrap();
        let cookie = 99_u64;
        send_request_header(
            &mut stream,
            cookie,
            CMD_READ,
            DEFAULT_EXPORT_BYTES - 2048,
            BLOCK as u32,
        )
        .unwrap();
        stream.flush().unwrap();

        assert_eq!(get_u32(&mut stream).unwrap(), SIMPLE_REPLY_MAGIC);
        assert_eq!(get_u32(&mut stream).unwrap(), EINVAL);
        let mut handle = [0_u8; 8];
        stream.read_exact(&mut handle).unwrap();
        assert_eq!(u64::from_be_bytes(handle), cookie);

        send_request_header(&mut stream, 100, CMD_DISC, 0, 0).unwrap();
        stream.flush().unwrap();
        drop(stream);
        server.join().unwrap().unwrap();

        assert_eq!(
            fs::metadata(root.join("backing.img")).unwrap().len(),
            DEFAULT_EXPORT_BYTES
        );
        assert_eq!(
            fs::read_to_string(root.join("events.jsonl")).unwrap(),
            concat!(
                "{\"seq\":1,\"op\":\"read\",\"offset\":1046528,\"length\":4096,\"status\":\"einval\"}\n",
                "{\"seq\":2,\"op\":\"disconnect\",\"offset\":0,\"length\":0,\"status\":\"ok\"}\n"
            )
        );

        fs::remove_file(root.join("backing.img")).unwrap();
        fs::remove_file(root.join("events.jsonl")).unwrap();
        fs::remove_dir(root).unwrap();
    }

    #[test]
    fn oversized_write_is_rejected_before_reading_its_payload() {
        let root = fresh_root("oversized-write");
        let server_root = root.clone();
        let server = thread::spawn(move || serve(&server_root, 1));
        wait_for_socket(&root);

        let mut stream = connect_probe(&root.join("nbd.sock")).unwrap();
        let cookie = 101_u64;
        send_request_header(&mut stream, cookie, CMD_WRITE, 0, u32::MAX).unwrap();
        stream.flush().unwrap();

        assert_eq!(get_u32(&mut stream).unwrap(), SIMPLE_REPLY_MAGIC);
        assert_eq!(get_u32(&mut stream).unwrap(), EINVAL);
        let mut handle = [0_u8; 8];
        stream.read_exact(&mut handle).unwrap();
        assert_eq!(u64::from_be_bytes(handle), cookie);
        assert_eq!(stream.read(&mut [0_u8; 1]).unwrap(), 0);
        server.join().unwrap().unwrap();

        assert_eq!(
            fs::metadata(root.join("backing.img")).unwrap().len(),
            DEFAULT_EXPORT_BYTES
        );
        let mut image = File::open(root.join("backing.img")).unwrap();
        let mut bytes = vec![0_u8; BLOCK];
        image.read_exact(&mut bytes).unwrap();
        assert!(bytes.iter().all(|byte| *byte == 0));
        assert_eq!(
            fs::read_to_string(root.join("events.jsonl")).unwrap(),
            "{\"seq\":1,\"op\":\"write\",\"offset\":0,\"length\":4294967295,\"status\":\"einval\"}\n"
        );

        fs::remove_file(root.join("backing.img")).unwrap();
        fs::remove_file(root.join("events.jsonl")).unwrap();
        fs::remove_dir(root).unwrap();
    }
}
