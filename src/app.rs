use std::io;

use crate::filesystem::FileSystem;
use crate::Trace;

use io_journey::record::{decode, encode, HEADER_BYTES};

pub(crate) fn append(fs: &mut FileSystem<'_>, payload: &[u8], trace: &mut Trace) -> io::Result<()> {
    if payload.len() > fs.capacity() - HEADER_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "log record exceeds file capacity",
        ));
    }
    trace.push(format!("app    append {} payload bytes", payload.len()));
    fs.append(&encode(payload)?, trace)
}

pub(crate) fn recover(fs: &mut FileSystem<'_>, trace: &mut Trace) -> io::Result<Vec<Vec<u8>>> {
    decode(&fs.read_all(trace)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovery_rejects_an_incomplete_or_corrupt_record() {
        let mut bytes = encode(b"first").unwrap();
        let prefix = bytes.len();
        bytes.extend_from_slice(&encode(b"second").unwrap());
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
