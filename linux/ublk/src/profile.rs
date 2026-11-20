use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::PathBuf;

pub struct Timing {
    pub tag: u16,
    pub operation: char,
    pub sector: u64,
    pub sectors: u32,
    pub wait_ns: u128,
    pub service_ns: u128,
    pub result: i32,
    pub advanced: bool,
}

pub struct Profile {
    output: PathBuf,
    active: bool,
    rows: Vec<Timing>,
    programs: usize,
    copies: usize,
    erases: usize,
}

impl Profile {
    pub fn new(output: PathBuf) -> Self {
        Self {
            output,
            active: false,
            rows: Vec::new(),
            programs: 0,
            copies: 0,
            erases: 0,
        }
    }

    pub fn start(&mut self) {
        self.active = true;
        self.rows.clear();
        self.programs = 0;
        self.copies = 0;
        self.erases = 0;
    }

    pub fn record(&mut self, timing: Timing, trace: &[String]) {
        if !self.active {
            return;
        }
        for event in trace {
            if event.starts_with("nand   program page") {
                self.programs += 1;
            } else if event.starts_with("ftl    gc copy") {
                self.programs += 1;
                self.copies += 1;
            } else if event.starts_with("nand   erase block") {
                self.erases += 1;
            }
        }
        self.rows.push(timing);
    }

    pub fn save(&mut self) -> io::Result<usize> {
        let mut file = BufWriter::new(File::create_new(&self.output)?);
        writeln!(file, "# {} {} {}", self.programs, self.copies, self.erases)?;
        writeln!(
            file,
            "tag\top\tsector\tsectors\twait_ns\tservice_ns\tresult\tadvanced"
        )?;
        for row in &self.rows {
            writeln!(
                file,
                "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                row.tag,
                row.operation,
                row.sector,
                row.sectors,
                row.wait_ns,
                row.service_ns,
                row.result,
                u8::from(row.advanced)
            )?;
        }
        file.flush()?;
        self.active = false;
        Ok(self.rows.len())
    }
}
