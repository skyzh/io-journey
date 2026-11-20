mod ownership;
mod target;

use std::env;
use std::error::Error;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::Shutdown;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use io_journey::record::{decode, encode, DEMO_RECORDS};
use libublk::ctrl::{UblkCtrl, UblkCtrlBuilder};
use libublk::io::UblkDev;
use libublk::{sys, UblkFlags};
use target::{Counts, LabDevice, DEVICE_BYTES, IO_BUFFER_BYTES, QUEUE_DEPTH};

const CONTROL_TIMEOUT: Duration = Duration::from_secs(5);
const TAIL: &[u8] = b"volatile";

fn main() -> Result<(), Box<dyn Error>> {
    let arguments = env::args_os().skip(1).collect::<Vec<_>>();
    match arguments.as_slice() {
        [verb, path] if verb == "serve" => serve(Path::new(path)),
        [verb, path, command] if verb == "control" => {
            let command = command
                .to_str()
                .ok_or_else(|| invalid("invalid control command"))?;
            Ok(send_control(Path::new(path), command)?)
        }
        [verb, id, pid, identity, token] if verb == "check-owned" || verb == "remove-owned" => {
            let id = id
                .to_str()
                .ok_or_else(|| invalid("invalid device ID"))?
                .parse()?;
            let pid = pid
                .to_str()
                .ok_or_else(|| invalid("invalid server PID"))?
                .parse()?;
            let identity = identity
                .to_str()
                .ok_or_else(|| invalid("invalid device identity"))?;
            let token = token
                .to_str()
                .ok_or_else(|| invalid("invalid ownership marker"))?;
            if verb == "remove-owned" {
                ownership::remove_after_reap(id, pid, identity, token)
            } else {
                ownership::check_running(id, pid, identity, token)
            }
        }
        [verb, mount] if verb == "append" => Ok(append_records(Path::new(mount))?),
        [verb, mount] if verb == "tail" => Ok(append_tail(Path::new(mount))?),
        [verb, mount] if verb == "verify" => Ok(verify(Path::new(mount), &DEMO_RECORDS)?),
        _ => Err(
            invalid("use serve SOCKET, control SOCKET COMMAND, or append/tail/verify MOUNT").into(),
        ),
    }
}

fn serve(socket: &Path) -> Result<(), Box<dyn Error>> {
    let listener = UnixListener::bind(socket)?;
    let state = Arc::new(Mutex::new(LabDevice::new()?));
    let ctrl = UblkCtrlBuilder::default()
        .name("io-journey")
        .id(-1)
        .nr_queues(1)
        .depth(QUEUE_DEPTH)
        .io_buf_bytes(IO_BUFFER_BYTES)
        .dev_flags(UblkFlags::UBLK_DEV_F_ADD_DEV)
        .build()?;
    let id = ctrl.dev_info().dev_id;
    let deadline = Instant::now() + CONTROL_TIMEOUT;
    let identity = loop {
        if let Some(identity) = ownership::character_identity(id)? {
            break identity;
        }
        if Instant::now() >= deadline {
            return Err(io::Error::other("allocated ublk character node is missing").into());
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let token = socket
        .to_str()
        .ok_or_else(|| invalid("invalid control path"))?
        .to_owned();
    println!("allocated {id} {} {identity}", std::process::id());
    io::stdout().flush()?;
    let worker_state = state.clone();
    let outcome = Arc::new(Mutex::new(None));
    let listener_outcome = outcome.clone();

    ctrl.run_target(
        move |dev: &mut UblkDev| {
            dev.set_default_params(DEVICE_BYTES as u64);
            dev.tgt.params.basic.logical_bs_shift = 12;
            dev.tgt.params.basic.attrs = sys::UBLK_ATTR_VOLATILE_CACHE;
            dev.set_target_json(serde_json::Value::String(token));
            Ok(())
        },
        move |qid, dev| {
            if let Err(error) = target::run_queue(qid, dev, &worker_state) {
                eprintln!("ublk queue failed: {error}");
            }
        },
        move |ctrl| {
            let result = (|| {
                println!("ready {id}");
                io::stdout().flush()?;
                control_loop(listener, ctrl, &state)
            })();
            if result.is_err() {
                let _ = ctrl.kill_dev();
            }
            *listener_outcome.lock().expect("control outcome") = Some(result);
        },
    )?;
    outcome
        .lock()
        .expect("control outcome")
        .take()
        .ok_or_else(|| io::Error::other("ublk server exited before serving control"))??;
    Ok(())
}

fn control_loop(
    listener: UnixListener,
    ctrl: &UblkCtrl,
    state: &Mutex<LabDevice>,
) -> io::Result<()> {
    for stream in listener.incoming() {
        let mut stream = stream?;
        stream.set_read_timeout(Some(CONTROL_TIMEOUT))?;
        stream.set_write_timeout(Some(CONTROL_TIMEOUT))?;
        let mut command = String::new();
        BufReader::new((&stream).take(64)).read_line(&mut command)?;

        if command.trim() == "stop" {
            ctrl.kill_dev()
                .map_err(|error| io::Error::other(error.to_string()))?;
            writeln!(stream, "stopped")?;
            return Ok(());
        }

        let counts = {
            let mut device = state
                .lock()
                .map_err(|_| io::Error::other("device lock is poisoned"))?;
            match command.trim() {
                "stats" => Ok(device.counts()),
                "checkpoint" => Ok(device.save_checkpoint()),
                "cut" => device.cut(),
                _ => Err(invalid("expected stats, checkpoint, cut, or stop")),
            }
        };
        match counts {
            Ok(Counts {
                reads,
                writes,
                flushes,
            }) => writeln!(stream, "{reads} {writes} {flushes}")?,
            Err(error) => writeln!(stream, "error {error}")?,
        }
    }
    Err(io::Error::other("ublk control listener stopped"))
}

fn send_control(socket: &Path, command: &str) -> io::Result<()> {
    let mut stream = UnixStream::connect(socket)?;
    stream.set_read_timeout(Some(CONTROL_TIMEOUT))?;
    stream.set_write_timeout(Some(CONTROL_TIMEOUT))?;
    writeln!(stream, "{command}")?;
    stream.shutdown(Shutdown::Write)?;
    let mut reply = String::new();
    stream.read_to_string(&mut reply)?;
    if let Some(error) = reply.strip_prefix("error ") {
        return Err(io::Error::other(error.trim().to_owned()));
    }
    if reply.is_empty() {
        return Err(io::Error::other("ublk server did not reply"));
    }
    print!("{reply}");
    Ok(())
}

fn append_records(mount: &Path) -> io::Result<()> {
    let mut log = OpenOptions::new()
        .create_new(true)
        .append(true)
        .open(mount.join("events.log"))?;
    for record in DEMO_RECORDS {
        log.write_all(&encode(record)?)?;
    }
    log.sync_all()?;
    File::open(mount)?.sync_all()?;
    println!("ext4 acknowledged first and second after file and directory sync");
    Ok(())
}

fn append_tail(mount: &Path) -> io::Result<()> {
    let mut log = OpenOptions::new()
        .append(true)
        .open(mount.join("events.log"))?;
    log.write_all(&encode(TAIL)?)?;
    verify(mount, &[DEMO_RECORDS[0], DEMO_RECORDS[1], TAIL])
}

fn verify(mount: &Path, expected: &[&[u8]]) -> io::Result<()> {
    let records = decode(&fs::read(mount.join("events.log"))?)?;
    if !records
        .iter()
        .map(Vec::as_slice)
        .eq(expected.iter().copied())
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "unexpected log records",
        ));
    }
    let names: Vec<_> = records
        .iter()
        .map(|record| String::from_utf8_lossy(record))
        .collect();
    println!("ext4 verified {names:?}");
    Ok(())
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}
