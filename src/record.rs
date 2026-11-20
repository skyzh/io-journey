use std::io;

#[cfg_attr(feature = "learner-append", path = "../course/learner/record.rs")]
#[cfg_attr(not(feature = "learner-append"), path = "record/reference.rs")]
mod receipt;
pub use receipt::decode_acknowledged;

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

pub(crate) fn crc32(bytes: &[u8]) -> u32 {
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
    fn known_receipt_recovers_the_prefix_before_an_incomplete_suffix() {
        let mut bytes = encode(b"first").unwrap();
        let acknowledged = bytes.len();
        let tail = encode(b"second").unwrap();
        bytes.extend_from_slice(&tail[..tail.len() - 1]);
        assert_eq!(
            decode_acknowledged(&bytes, acknowledged).unwrap(),
            [b"first".to_vec()]
        );
        assert!(decode_acknowledged(&bytes, 0).unwrap().is_empty());
        assert!(decode_acknowledged(&bytes, bytes.len()).is_err());
    }

    #[test]
    fn corruption_or_missing_bytes_inside_the_receipt_fails() {
        let mut bytes = encode(b"first").unwrap();
        let acknowledged = bytes.len();
        bytes[HEADER_BYTES] ^= 1;
        let error = decode_acknowledged(&bytes, acknowledged).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(error.to_string(), "record checksum mismatch");
        assert_eq!(
            decode_acknowledged(&bytes[..acknowledged - 1], acknowledged)
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );
    }
}
