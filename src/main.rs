mod export;
mod protocol;
mod server;

use std::env;
use std::io::{self, ErrorKind};
use std::path::PathBuf;

fn main() {
    if let Err(error) = run() {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

fn run() -> io::Result<()> {
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        Some("serve") => {
            let root = required_path(args.next(), "serve ROOT [CONNECTIONS]")?;
            let connections = args
                .next()
                .map(|value| value.parse::<usize>())
                .transpose()
                .map_err(|_| io::Error::new(ErrorKind::InvalidInput, "invalid connection count"))?
                .unwrap_or(1);
            ensure_no_extra(args)?;
            server::serve(&root, connections)
        }
        Some("probe") => {
            let socket = required_path(args.next(), "probe SOCKET")?;
            ensure_no_extra(args)?;
            protocol::probe(&socket)
        }
        Some("verify") => {
            let root = required_path(args.next(), "verify NEW_ROOT")?;
            ensure_no_extra(args)?;
            server::verify(&root)
        }
        _ => Err(io::Error::new(
            ErrorKind::InvalidInput,
            "usage: rust-nbd-lab {serve ROOT [CONNECTIONS]|probe SOCKET|verify NEW_ROOT}",
        )),
    }
}

fn required_path(value: Option<String>, usage: &str) -> io::Result<PathBuf> {
    value.map(PathBuf::from).ok_or_else(|| {
        io::Error::new(
            ErrorKind::InvalidInput,
            format!("usage: rust-nbd-lab {usage}"),
        )
    })
}

fn ensure_no_extra(mut args: impl Iterator<Item = String>) -> io::Result<()> {
    if args.next().is_some() {
        return Err(io::Error::new(
            ErrorKind::InvalidInput,
            "unexpected argument",
        ));
    }
    Ok(())
}
