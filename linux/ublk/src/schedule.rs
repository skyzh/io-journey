use std::time::Instant;

pub use io_journey::dispatch::Policy;
use io_journey::dispatch::{self, DeviceRequest, Kind};
use libublk::sys::{UBLK_IO_OP_FLUSH, UBLK_IO_OP_READ, UBLK_IO_OP_WRITE};

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

    fn request(&self) -> DeviceRequest {
        let kind = match self.op {
            UBLK_IO_OP_READ => Kind::Read,
            UBLK_IO_OP_WRITE => Kind::Write,
            _ => Kind::Barrier,
        };
        DeviceRequest {
            kind,
            sector: self.sector,
            sectors: self.sectors,
        }
    }
}

pub fn next_index(policy: Policy, batch: &[Arrival]) -> usize {
    dispatch::next_index(policy, batch, Arrival::request)
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
