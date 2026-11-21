use std::ffi::OsString;
use std::io;
use std::process::ExitCode;

const HELP: &str = "cargo xtask nand-latency [--read-us N] [--program-us N] [--erase-us N]";

fn main() -> ExitCode {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("xtask: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[OsString]) -> io::Result<()> {
    match args.split_first() {
        Some((command, options)) if command == "nand-latency" => nand_latency(options),
        None => {
            println!("{HELP}");
            Ok(())
        }
        Some((command, _)) if command == "--help" || command == "-h" => {
            println!("{HELP}");
            Ok(())
        }
        Some((command, _)) => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("unknown command {command:?}. Use cargo xtask --help"),
        )),
    }
}

#[cfg(not(feature = "learner"))]
fn nand_latency(args: &[OsString]) -> io::Result<()> {
    use io_journey::ftl::latency::{self, ServiceTimes};

    if args.len() == 1 && (args[0] == "--help" || args[0] == "-h") {
        println!("{HELP}");
        return Ok(());
    }

    let mut times = ServiceTimes::default();
    let mut pairs = args.chunks_exact(2);
    for pair in &mut pairs {
        let slot = match pair[0].to_str() {
            Some("--read-us") => &mut times.read,
            Some("--program-us") => &mut times.program,
            Some("--erase-us") => &mut times.erase,
            _ => return Err(invalid_times()),
        };
        *slot = pair[1]
            .to_str()
            .and_then(|value| value.parse().ok())
            .ok_or_else(invalid_times)?;
    }
    if !pairs.remainder().is_empty() {
        return Err(invalid_times());
    }
    latency::run(times)
}

#[cfg(not(feature = "learner"))]
fn invalid_times() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        format!("expected positive integer service times in microseconds. Use {HELP}"),
    )
}

#[cfg(feature = "learner")]
fn nand_latency(_: &[OsString]) -> io::Result<()> {
    Err(io::Error::other(
        "the NAND timing lab uses the reference FTL. Run cargo xtask nand-latency without learner features",
    ))
}
