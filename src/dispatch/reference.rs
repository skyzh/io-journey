use super::{DeviceRequest, Kind, Policy};

pub fn next_index<T>(policy: Policy, batch: &[T], request: impl Fn(&T) -> DeviceRequest) -> usize {
    if matches!(policy, Policy::Fifo) {
        return 0;
    }
    for (index, read) in batch.iter().map(&request).enumerate() {
        match read.kind {
            Kind::Read => {
                if batch[..index]
                    .iter()
                    .map(&request)
                    .all(|earlier| earlier.kind != Kind::Write || read.disjoint(earlier))
                {
                    return index;
                }
            }
            Kind::Write => {}
            Kind::Barrier => break,
        }
    }
    0
}
