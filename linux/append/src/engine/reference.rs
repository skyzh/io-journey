use super::{invalid_completion, Completion, Request, DEPTH};
use std::io;

pub(super) fn drain_ring_wave(
    wave: &[Request],
    mut poll: impl FnMut(&mut Vec<Completion>) -> io::Result<()>,
) -> io::Result<()> {
    let mut completed = Vec::with_capacity(wave.len());
    while completed.len() < wave.len() {
        poll(&mut completed)?;
    }
    check_completions(wave, completed)
}

pub(super) fn check_completions(wave: &[Request], completed: Vec<Completion>) -> io::Result<()> {
    let mut seen = [false; DEPTH];
    let mut error = None;
    for completion in completed {
        let index = wave
            .iter()
            .position(|item| item.id == completion.id)
            .ok_or_else(|| invalid_completion("unknown completion ID"))?;
        if seen[index] {
            return Err(invalid_completion("duplicate completion ID"));
        }
        seen[index] = true;
        let expected = wave[index].bytes.len();
        let result = completion.result.and_then(|size| {
            if size != expected {
                return Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    format!("request {} wrote {size} of {expected} bytes", completion.id),
                ));
            }
            Ok(())
        });
        if let Err(failure) = result {
            error.get_or_insert(failure);
        }
    }
    if !seen[..wave.len()].iter().all(|done| *done) {
        return Err(invalid_completion("missing completion ID"));
    }
    error.map_or(Ok(()), Err)
}
