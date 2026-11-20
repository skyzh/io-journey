use std::io;

// Each record starts with a little-endian payload length and CRC-32.
pub const HEADER_BYTES: usize = 8;
pub const DEMO_RECORDS: [&[u8]; 2] = [b"first", b"second"];

pub fn encode(payload: &[u8]) -> io::Result<Vec<u8>> {
    let length = u32::try_from(payload.len())
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "record length exceeds u32"))?;
    let capacity = payload
        .len()
        .checked_add(HEADER_BYTES)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "record length overflows"))?;
    let mut record = Vec::with_capacity(capacity);
    record.extend_from_slice(&length.to_le_bytes());
    record.extend_from_slice(&crc32(payload).to_le_bytes());
    record.extend_from_slice(payload);
    Ok(record)
}

pub fn decode(mut bytes: &[u8]) -> io::Result<Vec<Vec<u8>>> {
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
