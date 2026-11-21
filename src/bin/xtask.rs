use std::ffi::OsString;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::process::{Command, ExitCode, ExitStatus};

const NAND_HELP: &str = "cargo xtask nand-latency [--read-us N] [--program-us N] [--erase-us N]";
const HELP: &str = "Run from the checkout. See course/START.md for the Linux setup.

  cargo xtask demo
  cargo xtask test [filter]
  cargo xtask learner <stage>
  cargo xtask checkpoint prepare <ftl|queue|filesystem|log>
  cargo xtask checkpoint <previous|check|faults> <directory>
  cargo xtask ftl-collection
  cargo xtask ftl-snapshots <timing|heap>
  cargo xtask nand-latency [--read-us N] [--program-us N] [--erase-us N]

Mounted labs (Linux root):
  cargo xtask ext4 [learner]
  cargo xtask queue [learner]
  cargo xtask fuse <image|ublk> [learner]
  cargo xtask append [learner]";

fn main() -> ExitCode {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    match run(&args) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("xtask: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: &[OsString]) -> io::Result<ExitCode> {
    let (command, options) = match args.split_first() {
        Some((command, options)) => (command.to_str(), options),
        None => (Some("--help"), args),
    };
    match command {
        Some("--help" | "-h") if options.is_empty() => {
            println!("{HELP}");
            Ok(ExitCode::SUCCESS)
        }
        Some("nand-latency") => nand_latency(options).map(|()| ExitCode::SUCCESS),
        Some("demo") if options.is_empty() => {
            offline_cargo("run", &["--quiet", "--bin", "io-journey"], options)
        }
        Some("test") if options.len() <= 1 => offline_cargo("test", &["--"], options),
        Some("learner") => run_helper("course/learner.sh", options, false),
        Some("checkpoint") => run_helper("course/checkpoint.sh", options, false),
        Some("ftl-collection") if options.is_empty() => {
            run_helper("course/measure-collection.sh", options, false)
        }
        Some("ftl-snapshots") => run_helper("course/measure-snapshots.sh", options, false),
        Some("ext4") if reference_or_learner(options) => {
            run_helper("linux/ublk/smoke.sh", options, true)
        }
        Some("queue") if reference_or_learner(options) => {
            run_helper("linux/ublk/queue-bench.sh", options, true)
        }
        Some("fuse") if fuse_options(options) => run_helper("linux/fuse/smoke.sh", options, true),
        Some("append") if reference_or_learner(options) => {
            run_helper("linux/append/benchmark.sh", options, true)
        }
        _ => {
            eprintln!("Use cargo xtask --help for the supported commands and arguments.");
            Ok(ExitCode::from(2))
        }
    }
}

fn reference_or_learner(options: &[OsString]) -> bool {
    options.is_empty() || matches!(options, [mode] if mode == "learner")
}

fn fuse_options(options: &[OsString]) -> bool {
    match options {
        [backend, rest @ ..] => {
            (backend == "image" || backend == "ublk") && reference_or_learner(rest)
        }
        [] => true,
    }
}

fn run_helper(path: &str, args: &[OsString], private_mounts: bool) -> io::Result<ExitCode> {
    require_linux()?;
    let mut command = if private_mounts {
        let mut unshare = Command::new("unshare");
        unshare.args(["--mount", "--propagation", "private", "bash"]);
        unshare
    } else {
        Command::new("bash")
    };
    child_status(command.arg(path).args(args))
}

fn require_linux() -> io::Result<()> {
    if cfg!(target_os = "linux") {
        Ok(())
    } else {
        Err(io::Error::other(
            "run this command from the checkout in the Linux guest",
        ))
    }
}

fn child_status(command: &mut Command) -> io::Result<ExitCode> {
    let status = command.current_dir(env!("CARGO_MANIFEST_DIR")).status()?;
    Ok(ExitCode::from(match status.code() {
        Some(code) => u8::try_from(code).unwrap_or(1),
        None => signal_code(status),
    }))
}

#[cfg(unix)]
fn signal_code(status: ExitStatus) -> u8 {
    use std::os::unix::process::ExitStatusExt;
    status
        .signal()
        .and_then(|signal| u8::try_from(128 + signal).ok())
        .unwrap_or(1)
}

#[cfg(not(unix))]
fn signal_code(_: ExitStatus) -> u8 {
    1
}

struct Scratch(PathBuf);

impl Scratch {
    fn create() -> io::Result<Self> {
        let mount = Command::new("findmnt")
            .args(["-n", "-o", "FSTYPE,OPTIONS", "-T", "/dev/shm"])
            .output()?;
        let mount_output = String::from_utf8_lossy(&mount.stdout);
        let mut fields = mount_output.split_whitespace();
        if !mount.status.success()
            || fields.next() != Some("tmpfs")
            || !fields
                .next()
                .is_some_and(|options| !options.split(',').any(|flag| flag == "noexec"))
        {
            return Err(io::Error::other("/dev/shm must be executable tmpfs"));
        }

        let created = Command::new("mktemp")
            .args(["-d", "/dev/shm/io-journey-xtask.XXXXXX"])
            .output()?;
        if !created.status.success() {
            return Err(io::Error::other(
                "could not create Cargo scratch on /dev/shm",
            ));
        }
        let output = String::from_utf8(created.stdout).map_err(io::Error::other)?;
        let path = output.trim_end_matches('\n');
        let suffix = path.strip_prefix("/dev/shm/io-journey-xtask.");
        if !suffix.is_some_and(|name| {
            name.len() == 6 && name.bytes().all(|byte| byte.is_ascii_alphanumeric())
        }) {
            return Err(io::Error::other(
                "mktemp returned an unexpected Cargo scratch path",
            ));
        }
        Ok(Self(PathBuf::from(path)))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn offline_cargo(subcommand: &str, flags: &[&str], args: &[OsString]) -> io::Result<ExitCode> {
    require_linux()?;
    let scratch = Scratch::create()?;
    let tmp = scratch.0.join("tmp");
    fs::create_dir(&tmp)?;
    child_status(
        Command::new("cargo")
            .arg(subcommand)
            .args(["--locked", "--offline"])
            .args(flags)
            .args(args)
            .env("CARGO_HOME", scratch.0.join("cargo"))
            .env("CARGO_TARGET_DIR", scratch.0.join("target"))
            .env("TMPDIR", tmp)
            .env("CARGO_INCREMENTAL", "0"),
    )
}

#[cfg(not(feature = "learner"))]
fn nand_latency(args: &[OsString]) -> io::Result<()> {
    use io_journey::ftl::latency::{self, ServiceTimes};

    if args.len() == 1 && (args[0] == "--help" || args[0] == "-h") {
        println!("{NAND_HELP}");
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
        format!("expected positive integer service times in microseconds. Use {NAND_HELP}"),
    )
}

#[cfg(feature = "learner")]
fn nand_latency(_: &[OsString]) -> io::Result<()> {
    Err(io::Error::other(
        "the NAND timing lab uses the reference FTL. Run cargo xtask nand-latency without learner features",
    ))
}
