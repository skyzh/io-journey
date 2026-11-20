use std::io;

#[cfg_attr(feature = "learner-queue", path = "../course/learner/dispatch.rs")]
#[cfg_attr(not(feature = "learner-queue"), path = "dispatch/reference.rs")]
mod implementation;
pub use implementation::next_index;

#[derive(Clone, Copy)]
pub enum Policy {
    Fifo,
    ReadFirst,
}

impl Policy {
    pub fn parse(value: &str) -> io::Result<Self> {
        match value {
            "fifo" => Ok(Self::Fifo),
            "read-first" => Ok(Self::ReadFirst),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "expected fifo or read-first",
            )),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Read,
    Write,
    Barrier,
}

#[derive(Clone, Copy)]
pub struct DeviceRequest {
    pub kind: Kind,
    pub sector: u64,
    pub sectors: u32,
}

impl DeviceRequest {
    pub fn disjoint(self, other: Self) -> bool {
        let end = self.sector.checked_add(u64::from(self.sectors));
        let other_end = other.sector.checked_add(u64::from(other.sectors));
        end.zip(other_end)
            .is_some_and(|(end, other_end)| end <= other.sector || other_end <= self.sector)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(kind: Kind, sector: u64, sectors: u32) -> DeviceRequest {
        DeviceRequest {
            kind,
            sector,
            sectors,
        }
    }

    #[test]
    fn only_disjoint_reads_advance_before_a_barrier() {
        let write = request(Kind::Write, 8, 16);
        let disjoint = request(Kind::Read, 24, 8);
        let overlapping = request(Kind::Read, 16, 16);
        let flush = request(Kind::Barrier, 0, 0);
        assert_eq!(
            next_index(Policy::Fifo, &[write, disjoint], |entry| *entry),
            0
        );
        assert_eq!(
            next_index(Policy::ReadFirst, &[write, disjoint], |entry| *entry),
            1
        );
        assert_eq!(
            next_index(Policy::ReadFirst, &[write, overlapping], |entry| *entry),
            0
        );
        assert_eq!(
            next_index(Policy::ReadFirst, &[write, flush, disjoint], |entry| *entry),
            0
        );
        let overflow = request(Kind::Read, u64::MAX - 1, 8);
        assert_eq!(
            next_index(Policy::ReadFirst, &[write, overflow], |entry| *entry),
            0
        );
    }
}
