use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{self, ErrorKind, Read};
use std::os::unix::fs::{FileTypeExt, OpenOptionsExt, PermissionsExt};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::thread;
use std::time::Duration;

use crate::export::{EventLog, transmission};
use crate::protocol::*;

const READY_FILE: &str = "listener.ready";

fn path_entry_exists(path: &Path) -> io::Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

fn listener_is_ready(root: &Path) -> io::Result<bool> {
    let ready = root.join(READY_FILE);
    let socket = root.join("nbd.sock");
    let ready_metadata = match fs::symlink_metadata(&ready) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    let socket_metadata = match fs::symlink_metadata(&socket) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    Ok(ready_metadata.file_type().is_file() && socket_metadata.file_type().is_socket())
}

fn remove_listener_artifacts(root: &Path) -> io::Result<()> {
    let mut first_error = None;
    for artifact in [root.join(READY_FILE), root.join("nbd.sock")] {
        match fs::remove_file(&artifact) {
            Ok(()) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) if first_error.is_none() => first_error = Some(error),
            Err(_) => {}
        }
    }
    first_error.map_or(Ok(()), Err)
}

fn stop_child_and_remove_listener(child: &mut Child, root: &Path) {
    if child.try_wait().ok().flatten().is_none() {
        let _ = child.kill();
    }
    let _ = child.wait();
    let _ = remove_listener_artifacts(root);
}

struct ListenerRuntime {
    listener: UnixListener,
    socket: PathBuf,
    ready: PathBuf,
    armed: bool,
}

impl ListenerRuntime {
    fn bind(root: &Path) -> io::Result<Self> {
        let socket = root.join("nbd.sock");
        let ready = root.join(READY_FILE);
        let listener = UnixListener::bind(&socket)?;
        let runtime = Self {
            listener,
            socket,
            ready,
            armed: true,
        };
        fs::set_permissions(&runtime.socket, fs::Permissions::from_mode(0o600))?;
        let ready_file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&runtime.ready)?;
        ready_file.set_permissions(fs::Permissions::from_mode(0o600))?;
        ready_file.sync_all()?;
        Ok(runtime)
    }

    fn close(mut self) -> io::Result<()> {
        self.armed = false;
        remove_listener_artifacts(self.socket.parent().unwrap())
    }
}

impl Drop for ListenerRuntime {
    fn drop(&mut self) {
        if self.armed {
            let _ = remove_listener_artifacts(self.socket.parent().unwrap());
        }
    }
}

pub(crate) fn verify(root: &Path) -> io::Result<()> {
    if path_entry_exists(root)? {
        return Err(io::Error::new(
            ErrorKind::AlreadyExists,
            "verification root must not already exist",
        ));
    }
    fs::create_dir(root)?;
    fs::set_permissions(root, fs::Permissions::from_mode(0o700))?;
    let socket = root.join("nbd.sock");
    let image = root.join("backing.img");
    let events = root.join("events.jsonl");
    let executable = env::current_exe()?;
    let mut child = Command::new(executable)
        .arg("serve")
        .arg(root)
        .arg("1")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()?;

    let mut ready = false;
    for _ in 0..200 {
        if listener_is_ready(root)? {
            ready = true;
            break;
        }
        if child.try_wait()?.is_some() {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }
    if !ready {
        stop_child_and_remove_listener(&mut child, root);
        return Err(io::Error::new(
            ErrorKind::TimedOut,
            "server socket was not ready",
        ));
    }

    let probe_result = probe(&socket);
    if probe_result.is_err() {
        stop_child_and_remove_listener(&mut child, root);
        return probe_result;
    }
    let status = match child.wait() {
        Ok(status) => status,
        Err(error) => {
            stop_child_and_remove_listener(&mut child, root);
            return Err(error);
        }
    };
    if !status.success() {
        let _ = remove_listener_artifacts(root);
        return Err(io::Error::other(format!("server exited with {status}")));
    }
    if socket.exists() || root.join(READY_FILE).exists() {
        return Err(io::Error::other("server left its socket behind"));
    }
    let metadata = fs::metadata(&image)?;
    if metadata.len() != DEFAULT_EXPORT_BYTES {
        return Err(io::Error::other("backing file size changed"));
    }
    let mut first_block = vec![0_u8; BLOCK];
    File::open(&image)?.read_exact(&mut first_block)?;
    if first_block.iter().any(|byte| *byte != 0) {
        return Err(io::Error::other(
            "chosen trim policy did not zero the block",
        ));
    }
    let event_bytes = fs::read(&events)?;
    let event_text = String::from_utf8(event_bytes)
        .map_err(|_| io::Error::new(ErrorKind::InvalidData, "event log is not UTF-8"))?;
    let expected = [
        "\"op\":\"write\"",
        "\"op\":\"flush\"",
        "\"op\":\"read\"",
        "\"op\":\"trim\"",
        "\"op\":\"flush\"",
        "\"op\":\"read\"",
        "\"op\":\"disconnect\"",
    ];
    let mut cursor = 0;
    for marker in expected {
        let relative = event_text[cursor..]
            .find(marker)
            .ok_or_else(|| io::Error::other(format!("missing event marker {marker}")))?;
        cursor += relative + marker.len();
    }
    println!(
        "{{\"check\":\"verify\",\"export_bytes\":{DEFAULT_EXPORT_BYTES},\"event_count\":7,\"read_bytes\":8192,\"write_bytes\":4096,\"trim_bytes\":4096,\"flushes\":2,\"trim_policy\":\"deterministic_zero_readback\",\"socket_removed\":true,\"backing_size_preserved\":true}}"
    );
    Ok(())
}

pub(crate) fn serve(root: &Path, connections: usize) -> io::Result<()> {
    if connections == 0 {
        return Err(io::Error::new(
            ErrorKind::InvalidInput,
            "connection count must be positive",
        ));
    }
    let metadata = fs::symlink_metadata(root)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(io::Error::new(
            ErrorKind::InvalidInput,
            "root must be a real directory",
        ));
    }
    if metadata.permissions().mode() & 0o077 != 0 {
        return Err(io::Error::new(
            ErrorKind::PermissionDenied,
            "root must be owner-only",
        ));
    }
    let socket = root.join("nbd.sock");
    let image_path = root.join("backing.img");
    let events_path = root.join("events.jsonl");
    let ready_path = root.join(READY_FILE);
    for path in [&socket, &ready_path, &image_path, &events_path] {
        if path_entry_exists(path)? {
            return Err(io::Error::new(
                ErrorKind::AlreadyExists,
                format!("refusing to overwrite {}", path.display()),
            ));
        }
    }
    let mut image = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&image_path)?;
    image.set_permissions(fs::Permissions::from_mode(0o600))?;
    image.set_len(DEFAULT_EXPORT_BYTES)?;
    image.sync_all()?;
    let mut events = EventLog::new(&events_path)?;
    let runtime = ListenerRuntime::bind(root)?;
    let result = (|| {
        for _ in 0..connections {
            let (mut stream, _) = runtime.listener.accept()?;
            if negotiate(&mut stream, DEFAULT_EXPORT_BYTES)? {
                transmission(&mut stream, &mut image, DEFAULT_EXPORT_BYTES, &mut events)?;
            }
        }
        image.sync_all()
    })();
    let cleanup_result = runtime.close();
    result.and(cleanup_result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::os::unix::fs::symlink;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_ROOT: AtomicU64 = AtomicU64::new(1);

    fn unique_path(label: &str) -> PathBuf {
        env::temp_dir().join(format!(
            "io-journey-nbd-test-{}-{label}-{}",
            std::process::id(),
            NEXT_ROOT.fetch_add(1, Ordering::Relaxed)
        ))
    }

    fn fresh_root(label: &str) -> PathBuf {
        let root = unique_path(label);
        assert!(!path_entry_exists(&root).unwrap());
        fs::create_dir(&root).unwrap();
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        root
    }

    fn wait_for_listener(
        root: &Path,
        server: &thread::JoinHandle<io::Result<()>>,
    ) -> io::Result<()> {
        for _ in 0..200 {
            if listener_is_ready(root).unwrap() {
                return Ok(());
            }
            if server.is_finished() {
                return Err(io::Error::new(
                    ErrorKind::BrokenPipe,
                    "server exited before listener readiness",
                ));
            }
            thread::sleep(Duration::from_millis(5));
        }
        Err(io::Error::new(
            ErrorKind::TimedOut,
            "server listener was not ready",
        ))
    }

    fn mode(path: &Path) -> u32 {
        fs::symlink_metadata(path).unwrap().permissions().mode() & 0o777
    }

    fn wait_for_mode(path: &Path, expected: u32) {
        for _ in 0..200 {
            if path.exists() && mode(path) == expected {
                return;
            }
            thread::sleep(Duration::from_millis(5));
        }
        panic!("{} did not reach mode {expected:o}", path.display());
    }

    #[test]
    fn serve_rejects_a_symlink_root() {
        let target = fresh_root("symlink-target");
        let link = unique_path("symlink-root");
        symlink(&target, &link).unwrap();

        let error = serve(&link, 1).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidInput);
        assert!(!target.join("backing.img").exists());

        fs::remove_file(link).unwrap();
        fs::remove_dir(target).unwrap();
    }

    #[test]
    fn serve_rejects_a_root_accessible_by_group() {
        let root = fresh_root("group-access");
        fs::set_permissions(&root, fs::Permissions::from_mode(0o750)).unwrap();

        let error = serve(&root, 1).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::PermissionDenied);
        assert!(!root.join("nbd.sock").exists());
        assert!(!root.join("backing.img").exists());
        assert!(!root.join("events.jsonl").exists());

        fs::remove_dir(root).unwrap();
    }

    #[test]
    fn serve_keeps_runtime_artifacts_owner_only() {
        let root = fresh_root("owner-only");
        let server_root = root.clone();
        let server = thread::spawn(move || serve(&server_root, 1));
        wait_for_listener(&root, &server).unwrap();
        wait_for_mode(&root.join("nbd.sock"), 0o600);
        wait_for_mode(&root.join(READY_FILE), 0o600);
        wait_for_mode(&root.join("backing.img"), 0o600);
        wait_for_mode(&root.join("events.jsonl"), 0o600);

        assert_eq!(mode(&root), 0o700);
        assert_eq!(mode(&root.join("nbd.sock")), 0o600);
        assert_eq!(mode(&root.join(READY_FILE)), 0o600);
        assert_eq!(mode(&root.join("backing.img")), 0o600);
        assert_eq!(mode(&root.join("events.jsonl")), 0o600);

        probe(&root.join("nbd.sock")).unwrap();
        server.join().unwrap().unwrap();

        assert!(!root.join("nbd.sock").exists());
        assert!(!root.join(READY_FILE).exists());
        assert_eq!(mode(&root.join("backing.img")), 0o600);
        assert_eq!(mode(&root.join("events.jsonl")), 0o600);

        fs::remove_file(root.join("backing.img")).unwrap();
        fs::remove_file(root.join("events.jsonl")).unwrap();
        fs::remove_dir(root).unwrap();
    }

    #[test]
    fn listener_readiness_is_connectable_and_cleans_after_protocol_error() {
        let root = fresh_root("readiness-error");
        let server_root = root.clone();
        let server = thread::spawn(move || serve(&server_root, 1));
        wait_for_listener(&root, &server).unwrap();

        let mut stream = std::os::unix::net::UnixStream::connect(root.join("nbd.sock")).unwrap();
        let mut handshake = [0_u8; 18];
        stream.read_exact(&mut handshake).unwrap();
        stream.write_all(&u32::MAX.to_be_bytes()).unwrap();
        stream.flush().unwrap();
        drop(stream);

        let error = server.join().unwrap().unwrap_err();
        assert_eq!(error.kind(), ErrorKind::InvalidData);
        assert!(!root.join("nbd.sock").exists());
        assert!(!root.join(READY_FILE).exists());

        fs::remove_file(root.join("backing.img")).unwrap();
        fs::remove_file(root.join("events.jsonl")).unwrap();
        fs::remove_dir(root).unwrap();
    }

    #[test]
    fn readiness_waiter_fails_when_the_server_exits_first() {
        let root = fresh_root("readiness-exit");
        let server = thread::spawn(|| {
            Err::<(), io::Error>(io::Error::other("injected pre-readiness failure"))
        });

        let error = wait_for_listener(&root, &server).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::BrokenPipe);
        assert!(!root.join("nbd.sock").exists());
        assert!(!root.join(READY_FILE).exists());
        assert!(server.join().unwrap().is_err());

        fs::remove_dir(root).unwrap();
    }

    #[test]
    fn verifier_cleanup_removes_only_listener_artifacts() {
        let root = fresh_root("verifier-cleanup");
        let listener = UnixListener::bind(root.join("nbd.sock")).unwrap();
        fs::write(root.join(READY_FILE), b"").unwrap();
        fs::write(root.join("backing.img"), b"preserve").unwrap();

        remove_listener_artifacts(&root).unwrap();
        assert!(!root.join("nbd.sock").exists());
        assert!(!root.join(READY_FILE).exists());
        assert_eq!(fs::read(root.join("backing.img")).unwrap(), b"preserve");

        drop(listener);
        fs::remove_file(root.join("backing.img")).unwrap();
        fs::remove_dir(root).unwrap();
    }

    #[test]
    fn serve_refuses_a_preexisting_runtime_path() {
        let root = fresh_root("existing-path");
        File::create(root.join("backing.img")).unwrap();

        let error = serve(&root, 1).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::AlreadyExists);
        assert!(!root.join("nbd.sock").exists());
        assert!(!root.join("events.jsonl").exists());

        fs::remove_file(root.join("backing.img")).unwrap();
        fs::remove_dir(root).unwrap();
    }

    #[test]
    fn serve_never_follows_or_overwrites_a_preexisting_readiness_path() {
        let root = fresh_root("existing-readiness");
        fs::write(root.join(READY_FILE), b"sentinel").unwrap();

        let error = serve(&root, 1).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::AlreadyExists);
        assert_eq!(fs::read(root.join(READY_FILE)).unwrap(), b"sentinel");
        assert!(!root.join("nbd.sock").exists());
        assert!(!root.join("backing.img").exists());
        assert!(!root.join("events.jsonl").exists());

        fs::remove_file(root.join(READY_FILE)).unwrap();
        fs::remove_dir(root).unwrap();
    }

    #[test]
    fn out_of_range_request_is_rejected_without_growing_the_export() {
        let root = fresh_root("bounds");
        let server_root = root.clone();
        let server = thread::spawn(move || serve(&server_root, 1));
        wait_for_listener(&root, &server).unwrap();

        let mut stream = connect_probe(&root.join("nbd.sock")).unwrap();
        let cookie = 99_u64;
        send_request_header(
            &mut stream,
            cookie,
            CMD_READ,
            DEFAULT_EXPORT_BYTES - 2048,
            BLOCK as u32,
        )
        .unwrap();
        stream.flush().unwrap();

        assert_eq!(get_u32(&mut stream).unwrap(), SIMPLE_REPLY_MAGIC);
        assert_eq!(get_u32(&mut stream).unwrap(), EINVAL);
        let mut handle = [0_u8; 8];
        stream.read_exact(&mut handle).unwrap();
        assert_eq!(u64::from_be_bytes(handle), cookie);

        send_request_header(&mut stream, 100, CMD_DISC, 0, 0).unwrap();
        stream.flush().unwrap();
        drop(stream);
        server.join().unwrap().unwrap();

        assert_eq!(
            fs::metadata(root.join("backing.img")).unwrap().len(),
            DEFAULT_EXPORT_BYTES
        );
        assert_eq!(
            fs::read_to_string(root.join("events.jsonl")).unwrap(),
            concat!(
                "{\"seq\":1,\"op\":\"read\",\"offset\":1046528,\"length\":4096,\"status\":\"einval\"}\n",
                "{\"seq\":2,\"op\":\"disconnect\",\"offset\":0,\"length\":0,\"status\":\"ok\"}\n"
            )
        );

        fs::remove_file(root.join("backing.img")).unwrap();
        fs::remove_file(root.join("events.jsonl")).unwrap();
        fs::remove_dir(root).unwrap();
    }

    #[test]
    fn oversized_write_is_rejected_before_reading_its_payload() {
        let root = fresh_root("oversized-write");
        let server_root = root.clone();
        let server = thread::spawn(move || serve(&server_root, 1));
        wait_for_listener(&root, &server).unwrap();

        let mut stream = connect_probe(&root.join("nbd.sock")).unwrap();
        let cookie = 101_u64;
        send_request_header(&mut stream, cookie, CMD_WRITE, 0, u32::MAX).unwrap();
        stream.flush().unwrap();

        assert_eq!(get_u32(&mut stream).unwrap(), SIMPLE_REPLY_MAGIC);
        assert_eq!(get_u32(&mut stream).unwrap(), EINVAL);
        let mut handle = [0_u8; 8];
        stream.read_exact(&mut handle).unwrap();
        assert_eq!(u64::from_be_bytes(handle), cookie);
        assert_eq!(stream.read(&mut [0_u8; 1]).unwrap(), 0);
        server.join().unwrap().unwrap();

        assert_eq!(
            fs::metadata(root.join("backing.img")).unwrap().len(),
            DEFAULT_EXPORT_BYTES
        );
        let mut image = File::open(root.join("backing.img")).unwrap();
        let mut bytes = vec![0_u8; BLOCK];
        image.read_exact(&mut bytes).unwrap();
        assert!(bytes.iter().all(|byte| *byte == 0));
        assert_eq!(
            fs::read_to_string(root.join("events.jsonl")).unwrap(),
            "{\"seq\":1,\"op\":\"write\",\"offset\":0,\"length\":4294967295,\"status\":\"einval\"}\n"
        );

        fs::remove_file(root.join("backing.img")).unwrap();
        fs::remove_file(root.join("events.jsonl")).unwrap();
        fs::remove_dir(root).unwrap();
    }
}
