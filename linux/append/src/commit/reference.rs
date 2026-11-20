use std::io;
use std::time::{Duration, Instant};

use super::{Request, DEPTH};

pub(super) fn commit_group(
    group: &[Request],
    write: &mut impl FnMut(&[Request]) -> io::Result<()>,
    sync: &mut impl FnMut() -> io::Result<()>,
    receipts: &mut Vec<Duration>,
) -> io::Result<()> {
    let ready = Instant::now();
    for wave in group.chunks(DEPTH) {
        write(wave)?;
    }
    sync()?;
    receipts.extend(std::iter::repeat_n(ready.elapsed(), group.len()));
    Ok(())
}
