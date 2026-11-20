mod ownership;
mod profile;
mod schedule;
mod target;

use std::env;
use std::error::Error;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::Shutdown;
use std::os::unix::net::{UnixListener, UnixStream};
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use io_journey::record::{decode, encode, DEMO_RECORDS};
use libublk::ctrl::UblkCtrlBuilder;
use libublk::io::UblkDev;
use libublk::{sys, UblkFlags};
use schedule::Policy;
use target::{Counts, LabDevice, DEVICE_BYTES, IO_BUFFER_BYTES, QUEUE_DEPTH};

const CONTROL_TIMEOUT: Duration = Duration::from_secs(5);
const TAIL: &[u8] = b"volatile";

fn main() -> Result<(), Box<dyn Error>> {
    let arguments = env::args_os().skip(1).collect::<Vec<_>>();
    match arguments.as_slice() {
        [verb, path] if verb == "serve" => serve(Path::new(path), Policy::Fifo, None),
        [verb, path, policy, output] if verb == "serve" => {
            let policy = policy
                .to_str()
                .ok_or_else(|| invalid("invalid scheduling policy"))?;
            serve(
                Path::new(path),
                Policy::parse(policy)?,
                Some(PathBuf::from(output)),
            )
        }
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

fn serve(socket: &Path, policy: Policy, profile: Option<PathBuf>) -> Result<(), Box<dyn Error>> {
    let listener = UnixListener::bind(socket)?;
    let state = Arc::new(Mutex::new(LabDevice::new(profile)?));
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
    let listener_state = state.clone();
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
            run_worker(&worker_state, || {
                target::run_queue(qid, dev, &worker_state, policy)
            });
        },
        move |ctrl| {
            let result = (|| {
                println!("ready {id}");
                io::stdout().flush()?;
                control_loop(
                    listener.incoming(),
                    &listener_state,
                    CONTROL_TIMEOUT,
                    || {
                        ctrl.kill_dev()
                            .map(|_| ())
                            .map_err(|error| io::Error::other(error.to_string()))
                    },
                )
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
    drop(lock_ready_device(&state)?);
    Ok(())
}

fn run_worker<E: std::fmt::Display>(state: &Mutex<LabDevice>, run: impl FnOnce() -> Result<(), E>) {
    let message = match panic::catch_unwind(AssertUnwindSafe(run)) {
        Ok(Ok(())) => return,
        Ok(Err(error)) => format!("ublk queue failed: {error}"),
        Err(_) => "ublk queue worker panicked".into(),
    };
    state
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .fail_worker(message.clone());
    eprintln!("{message}");
}

fn lock_ready_device(state: &Mutex<LabDevice>) -> io::Result<MutexGuard<'_, LabDevice>> {
    let device = match state.lock() {
        Ok(device) => device,
        Err(poisoned) => {
            poisoned.into_inner().check_worker()?;
            return Err(io::Error::other("device lock is poisoned"));
        }
    };
    device.check_worker()?;
    Ok(device)
}

fn control_loop(
    streams: impl IntoIterator<Item = io::Result<UnixStream>>,
    state: &Mutex<LabDevice>,
    timeout: Duration,
    stop: impl FnOnce() -> io::Result<()>,
) -> io::Result<()> {
    for stream in streams {
        let mut stream = match stream {
            Ok(stream) => stream,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };
        let Ok(command) = read_control_command(&stream, timeout) else {
            continue;
        };

        if command.trim() == "stop" {
            stop()?;
            // The stop succeeded even if the client left before its reply.
            let _ = writeln!(stream, "stopped");
            return Ok(());
        }

        let reply = lock_ready_device(state).and_then(|mut device| match command.trim() {
            "stats" => Ok(count_line(device.counts())),
            "checkpoint" => Ok(count_line(device.save_checkpoint())),
            "cut" => device.cut().map(count_line),
            "profile-start" => device.start_profile().map(|()| "profiling".into()),
            "profile-save" => device.save_profile().map(|rows| rows.to_string()),
            _ => Err(invalid("unknown ublk control command")),
        });
        // Reply delivery belongs to this client connection.
        let _ = match reply {
            Ok(reply) => writeln!(stream, "{reply}"),
            Err(error) => writeln!(stream, "error {error}"),
        };
    }
    Err(io::Error::other("ublk control listener stopped"))
}

fn read_control_command(stream: &UnixStream, timeout: Duration) -> io::Result<String> {
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;
    let mut command = String::new();
    BufReader::new(stream.take(64)).read_line(&mut command)?;
    if !command.ends_with('\n') {
        return Err(invalid("unterminated ublk control command"));
    }
    Ok(command)
}

fn count_line(
    Counts {
        reads,
        writes,
        flushes,
    }: Counts,
) -> String {
    format!("{reads} {writes} {flushes}")
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

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_TIMEOUT: Duration = Duration::from_millis(50);

    fn client(command: &[u8]) -> (UnixStream, UnixStream) {
        let (server, mut peer) = UnixStream::pair().unwrap();
        peer.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
        peer.write_all(command).unwrap();
        (server, peer)
    }

    fn check_stats_then_stop(first: io::Result<UnixStream>) {
        let (stats, mut stats_peer) = client(b"stats\n");
        let (stop, mut stop_peer) = client(b"stop\n");
        let state = Mutex::new(LabDevice::new(None).unwrap());
        let mut stopped = false;
        control_loop([first, Ok(stats), Ok(stop)], &state, TEST_TIMEOUT, || {
            stopped = true;
            Ok(())
        })
        .unwrap();

        let mut reply = String::new();
        stats_peer.read_to_string(&mut reply).unwrap();
        assert_eq!(reply, "0 0 0\n");
        reply.clear();
        stop_peer.read_to_string(&mut reply).unwrap();
        assert_eq!(reply, "stopped\n");
        assert!(stopped);
    }

    #[test]
    fn timed_out_client_does_not_stop_control_loop() {
        let (server, mut peer) = client(b"sta");
        check_stats_then_stop(Ok(server));
        assert_eq!(peer.read(&mut [0]).unwrap(), 0);
    }

    #[test]
    fn unterminated_stop_does_not_stop_control_loop() {
        let (server, peer) = client(b"stop");
        peer.shutdown(Shutdown::Write).unwrap();
        check_stats_then_stop(Ok(server));
    }

    #[test]
    fn truncated_command_does_not_stop_control_loop() {
        let mut command = b"stop".to_vec();
        command.resize(64, b' ');
        command.extend_from_slice(b"other\n");
        let (server, _peer) = client(&command);
        check_stats_then_stop(Ok(server));
    }

    #[test]
    fn failed_reply_does_not_stop_control_loop() {
        let (server, peer) = client(b"stats\n");
        peer.shutdown(Shutdown::Read).unwrap();
        check_stats_then_stop(Ok(server));
    }

    #[test]
    fn interrupted_accept_keeps_serving_until_explicit_stop() {
        check_stats_then_stop(Err(io::ErrorKind::Interrupted.into()));
    }

    #[test]
    fn terminal_accept_error_preserves_the_error() {
        let (server, _peer) = client(b"stop\n");
        let state = Mutex::new(LabDevice::new(None).unwrap());
        let error = control_loop(
            [Err(io::Error::from_raw_os_error(libc::EBADF)), Ok(server)],
            &state,
            TEST_TIMEOUT,
            || panic!("a terminal accept error must not reach the stop command"),
        )
        .unwrap_err();
        assert_eq!(error.raw_os_error(), Some(libc::EBADF));
    }

    fn check_failed_worker(state: &Mutex<LabDevice>, message: &str) {
        let (stats, mut stats_peer) = client(b"stats\n");
        let (checkpoint, mut checkpoint_peer) = client(b"checkpoint\n");
        let (stop, mut stop_peer) = client(b"stop\n");
        let mut stopped = false;
        control_loop(
            [Ok(stats), Ok(checkpoint), Ok(stop)],
            state,
            TEST_TIMEOUT,
            || {
                stopped = true;
                Ok(())
            },
        )
        .unwrap();

        for peer in [&mut stats_peer, &mut checkpoint_peer] {
            let mut reply = String::new();
            peer.read_to_string(&mut reply).unwrap();
            assert_eq!(reply, format!("error {message}\n"));
        }
        let mut reply = String::new();
        stop_peer.read_to_string(&mut reply).unwrap();
        assert_eq!(reply, "stopped\n");
        assert!(stopped);
        assert_eq!(lock_ready_device(state).err().unwrap().to_string(), message);
        let mut device = state.lock().unwrap_or_else(PoisonError::into_inner);
        assert!(device.cut().is_err());
    }

    #[test]
    fn failed_worker_rejects_device_commands_but_accepts_stop() {
        let state = Mutex::new(LabDevice::new(None).unwrap());
        run_worker(&state, || Err(io::Error::other("injected failure")));
        check_failed_worker(&state, "ublk queue failed: injected failure");
    }

    #[test]
    fn panicked_worker_reports_failure_with_or_without_a_poisoned_model() {
        let state = Mutex::new(LabDevice::new(None).unwrap());
        run_worker(&state, || -> io::Result<()> {
            panic!("injected worker panic")
        });
        assert!(!state.is_poisoned());
        check_failed_worker(&state, "ublk queue worker panicked");

        let state = Mutex::new(LabDevice::new(None).unwrap());
        run_worker(&state, || -> io::Result<()> {
            let _locked = state.lock().unwrap();
            panic!("injected worker panic with the model locked");
        });
        assert!(state.is_poisoned());
        check_failed_worker(&state, "ublk queue worker panicked");
    }

    #[test]
    fn poisoned_model_without_a_worker_result_accepts_stop() {
        let state = Mutex::new(LabDevice::new(None).unwrap());
        let result = panic::catch_unwind(AssertUnwindSafe(|| {
            let _locked = state.lock().unwrap();
            panic!("injected panic before the worker result is recorded");
        }));
        assert!(result.is_err());
        check_failed_worker(&state, "device lock is poisoned");
    }

    #[test]
    fn stop_succeeds_when_its_reply_is_lost() {
        let (server, peer) = client(b"stop\n");
        peer.shutdown(Shutdown::Read).unwrap();
        let state = Mutex::new(LabDevice::new(None).unwrap());
        let mut stopped = false;
        control_loop([Ok(server)], &state, TEST_TIMEOUT, || {
            stopped = true;
            Ok(())
        })
        .unwrap();
        assert!(stopped);
    }
}
