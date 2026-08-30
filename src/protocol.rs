use std::io::{self, ErrorKind, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;

pub(crate) const NBD_MAGIC: u64 = 0x4e42444d41474943;
pub(crate) const IHAVEOPT: u64 = 0x49484156454f5054;
pub(crate) const OPTION_REPLY_MAGIC: u64 = 0x0003e889045565a9;
pub(crate) const REQUEST_MAGIC: u32 = 0x25609513;
pub(crate) const SIMPLE_REPLY_MAGIC: u32 = 0x67446698;

pub(crate) const FLAG_FIXED_NEWSTYLE: u16 = 1;
pub(crate) const FLAG_NO_ZEROES: u16 = 2;
pub(crate) const CLIENT_FIXED_NEWSTYLE: u32 = 1;
pub(crate) const CLIENT_NO_ZEROES: u32 = 2;

pub(crate) const OPT_EXPORT_NAME: u32 = 1;
pub(crate) const OPT_ABORT: u32 = 2;
pub(crate) const OPT_LIST: u32 = 3;
pub(crate) const OPT_INFO: u32 = 6;
pub(crate) const OPT_GO: u32 = 7;
pub(crate) const REP_ACK: u32 = 1;
pub(crate) const REP_SERVER: u32 = 2;
pub(crate) const REP_INFO: u32 = 3;
pub(crate) const REP_ERR_UNSUP: u32 = 0x8000_0001;

pub(crate) const INFO_EXPORT: u16 = 0;
pub(crate) const INFO_BLOCK_SIZE: u16 = 3;

pub(crate) const TRANSMISSION_HAS_FLAGS: u16 = 1;
pub(crate) const TRANSMISSION_SEND_FLUSH: u16 = 1 << 2;
pub(crate) const TRANSMISSION_SEND_TRIM: u16 = 1 << 5;
pub(crate) const TRANSMISSION_FLAGS: u16 =
    TRANSMISSION_HAS_FLAGS | TRANSMISSION_SEND_FLUSH | TRANSMISSION_SEND_TRIM;

pub(crate) const CMD_READ: u16 = 0;
pub(crate) const CMD_WRITE: u16 = 1;
pub(crate) const CMD_DISC: u16 = 2;
pub(crate) const CMD_FLUSH: u16 = 3;
pub(crate) const CMD_TRIM: u16 = 4;

pub(crate) const EINVAL: u32 = 22;
pub(crate) const ENOTSUP: u32 = 95;
pub(crate) const DEFAULT_EXPORT_BYTES: u64 = 1024 * 1024;
pub(crate) const BLOCK: usize = 4096;

pub(crate) fn negotiate(stream: &mut UnixStream, export_bytes: u64) -> io::Result<bool> {
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

pub(crate) fn parse_info_requests(payload: &[u8]) -> io::Result<Vec<u16>> {
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

pub(crate) fn option_reply(
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

pub(crate) fn command_name(command: u16) -> &'static str {
    match command {
        CMD_READ => "read",
        CMD_WRITE => "write",
        CMD_DISC => "disconnect",
        CMD_FLUSH => "flush",
        CMD_TRIM => "trim",
        _ => "unknown",
    }
}

pub(crate) fn simple_reply(
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

pub(crate) fn connect_probe(socket: &Path) -> io::Result<UnixStream> {
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

pub(crate) fn probe(socket: &Path) -> io::Result<()> {
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

pub(crate) fn get_option_reply(
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

pub(crate) fn get_slice_u16(bytes: &[u8]) -> u16 {
    u16::from_be_bytes(bytes.try_into().unwrap())
}

pub(crate) fn get_slice_u32(bytes: &[u8]) -> u32 {
    u32::from_be_bytes(bytes.try_into().unwrap())
}

pub(crate) fn get_slice_u64(bytes: &[u8]) -> u64 {
    u64::from_be_bytes(bytes.try_into().unwrap())
}

pub(crate) fn request(
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

pub(crate) fn send_request_header(
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

pub(crate) fn put_u16(writer: &mut impl Write, value: u16) -> io::Result<()> {
    writer.write_all(&value.to_be_bytes())
}

pub(crate) fn put_u32(writer: &mut impl Write, value: u32) -> io::Result<()> {
    writer.write_all(&value.to_be_bytes())
}

pub(crate) fn put_u64(writer: &mut impl Write, value: u64) -> io::Result<()> {
    writer.write_all(&value.to_be_bytes())
}

pub(crate) fn get_u16(reader: &mut impl Read) -> io::Result<u16> {
    let mut bytes = [0_u8; 2];
    reader.read_exact(&mut bytes)?;
    Ok(u16::from_be_bytes(bytes))
}

pub(crate) fn get_u32(reader: &mut impl Read) -> io::Result<u32> {
    let mut bytes = [0_u8; 4];
    reader.read_exact(&mut bytes)?;
    Ok(u32::from_be_bytes(bytes))
}

pub(crate) fn get_u64(reader: &mut impl Read) -> io::Result<u64> {
    let mut bytes = [0_u8; 8];
    reader.read_exact(&mut bytes)?;
    Ok(u64::from_be_bytes(bytes))
}
