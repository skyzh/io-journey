use std::io;

use crate::filesystem::{FileSystem, FILE_CAPACITY};
use crate::Trace;

// Each record starts with a little-endian payload length and CRC-32.
pub(crate) const HEADER_BYTES: usize = 8;

pub(crate) fn append(fs: &mut FileSystem<'_>, payload: &[u8], trace: &mut Trace) -> io::Result<()> {
    if payload.len() > FILE_CAPACITY - HEADER_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "log record exceeds file capacity",
        ));
    }
    trace.push(format!("app    append {} payload bytes", payload.len()));
    fs.append(&encode(payload), trace)
}

pub(crate) fn recover(fs: &mut FileSystem<'_>, trace: &mut Trace) -> io::Result<Vec<Vec<u8>>> {
    decode(&fs.read_all(trace)?)
}

fn encode(payload: &[u8]) -> Vec<u8> {
    let mut record = Vec::with_capacity(HEADER_BYTES + payload.len());
    record.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    record.extend_from_slice(&crc32(payload).to_le_bytes());
    record.extend_from_slice(payload);
    record
}

fn decode(mut bytes: &[u8]) -> io::Result<Vec<Vec<u8>>> {
    let mut records = Vec::new();
    while !bytes.is_empty() {
        let Some((header, rest)) = bytes.split_at_checked(HEADER_BYTES) else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "truncated record header",
            ));
        };
        let length = u32::from_le_bytes(header[..4].try_into().unwrap()) as usize;
        let checksum = u32::from_le_bytes(header[4..].try_into().unwrap());
        let Some((payload, rest)) = rest.split_at_checked(length) else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "truncated record payload",
            ));
        };
        if crc32(payload) != checksum {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "record checksum mismatch",
            ));
        }
        records.push(payload.to_vec());
        bytes = rest;
    }
    Ok(records)
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            let polynomial = if crc & 1 == 1 { 0xedb8_8320 } else { 0 };
            crc = (crc >> 1) ^ polynomial;
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovery_rejects_an_incomplete_or_corrupt_record() {
        let mut bytes = encode(b"first");
        let prefix = bytes.len();
        bytes.extend_from_slice(&encode(b"second"));
        assert_eq!(
            decode(&bytes).unwrap(),
            [b"first".to_vec(), b"second".to_vec()]
        );
        assert_eq!(
            decode(&bytes[..prefix + 1]).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert_eq!(
            decode(&bytes[..bytes.len() - 1]).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        *bytes.last_mut().unwrap() ^= 0x80;
        assert_eq!(
            decode(&bytes).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }
}
