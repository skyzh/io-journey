use std::io;
use std::sync::Arc;

use super::{Completion, Request, DEPTH};

pub(super) fn retire_ring_wave(
    wave: &[Request],
    retained: &mut Vec<Arc<[u8]>>,
    mut poll: impl FnMut(&mut Vec<Completion>) -> io::Result<()>,
) -> io::Result<()> {
    let mut completed = Vec::with_capacity(wave.len());
    while completed.len() < wave.len() {
        poll(&mut completed)?;
    }
    let result = check_completions(wave, completed);
    if !matches!(&result, Err(error) if error.kind() == io::ErrorKind::InvalidData) {
        retained.clear();
    }
    result
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

fn invalid_completion(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
