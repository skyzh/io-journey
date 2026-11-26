use std::io;
use std::time::Duration;

use super::Request;

pub fn commit_group(
    _group: &[Request],
    _write: &mut impl FnMut(&[Request]) -> io::Result<()>,
    _sync: &mut impl FnMut() -> io::Result<()>,
    _receipts: &mut Vec<Duration>,
) -> io::Result<()> {
    // TODO: Write bounded waves, sync the file, then add the receipts.
    Err(io::Error::other("implement group acknowledgement"))
}
