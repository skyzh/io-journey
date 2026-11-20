use std::io;

use super::decode;

pub fn decode_acknowledged(bytes: &[u8], _acknowledged: usize) -> io::Result<Vec<Vec<u8>>> {
    // TODO: Decode only the bytes covered by the acknowledgement receipt.
    decode(bytes)
}
