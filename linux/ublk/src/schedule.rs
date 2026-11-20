use std::io;
use std::time::Instant;

use libublk::sys::{UBLK_IO_OP_FLUSH, UBLK_IO_OP_READ, UBLK_IO_OP_WRITE};

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

pub struct Arrival {
    pub tag: u16,
    pub op: u32,
    pub sector: u64,
    pub sectors: u32,
    pub at: Instant,
}

impl Arrival {
    pub fn operation(&self) -> char {
        match self.op {
            UBLK_IO_OP_READ => 'R',
            UBLK_IO_OP_WRITE => 'W',
            UBLK_IO_OP_FLUSH => 'F',
            _ => '?',
        }
    }

    fn disjoint(&self, other: &Self) -> bool {
        let end = self.sector.checked_add(u64::from(self.sectors));
        let other_end = other.sector.checked_add(u64::from(other.sectors));
        end.zip(other_end)
            .is_some_and(|(end, other_end)| end <= other.sector || other_end <= self.sector)
    }
}

pub fn next_index(policy: Policy, batch: &[Arrival]) -> usize {
    if matches!(policy, Policy::Fifo) {
        return 0;
    }
    for (index, read) in batch.iter().enumerate() {
        match read.op {
            UBLK_IO_OP_READ => {
                if batch[..index]
                    .iter()
                    .all(|earlier| earlier.op != UBLK_IO_OP_WRITE || read.disjoint(earlier))
                {
                    return index;
                }
            }
            UBLK_IO_OP_WRITE => {}
            _ => break,
        }
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(op: u32, sector: u64, sectors: u32) -> Arrival {
        Arrival {
            tag: 0,
            op,
            sector,
            sectors,
            at: Instant::now(),
        }
    }

    #[test]
    fn only_a_disjoint_read_can_pass_a_write_and_flush_is_a_boundary() {
        let write = || request(UBLK_IO_OP_WRITE, 8, 16);
        let disjoint = || request(UBLK_IO_OP_READ, 24, 8);
        let overlapping = || request(UBLK_IO_OP_READ, 16, 16);
        assert_eq!(next_index(Policy::Fifo, &[write(), disjoint()]), 0);
        assert_eq!(next_index(Policy::ReadFirst, &[write(), disjoint()]), 1);
        assert_eq!(next_index(Policy::ReadFirst, &[write(), overlapping()]), 0);
        let flush = request(UBLK_IO_OP_FLUSH, 0, 0);
        assert_eq!(
            next_index(Policy::ReadFirst, &[write(), flush, disjoint()]),
            0
        );
    }
}
