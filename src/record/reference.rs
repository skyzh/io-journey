use std::io;

use super::decode;

pub fn decode_acknowledged(bytes: &[u8], acknowledged: usize) -> io::Result<Vec<Vec<u8>>> {
    let prefix = bytes.get(..acknowledged).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "receipt extends beyond the file",
        )
    })?;
    decode(prefix)
}
