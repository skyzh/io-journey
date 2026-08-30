use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{self, ErrorKind, Read};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::os::unix::net::UnixListener;
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use crate::export::{EventLog, transmission};
use crate::protocol::*;

fn path_entry_exists(path: &Path) -> io::Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
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
        if socket.exists() {
            ready = true;
            break;
        }
        if child.try_wait()?.is_some() {
            break;
        }
        thread::sleep(Duration::from_millis(5));
    }
    if !ready {
        let _ = child.kill();
        let _ = child.wait();
        return Err(io::Error::new(
            ErrorKind::TimedOut,
            "server socket was not ready",
        ));
    }

    let probe_result = probe(&socket);
    if probe_result.is_err() {
        let _ = child.kill();
    }
    let status = child.wait()?;
    probe_result?;
    if !status.success() {
        return Err(io::Error::other(format!("server exited with {status}")));
    }
    if socket.exists() {
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
    for path in [&socket, &image_path, &events_path] {
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
    let listener = UnixListener::bind(&socket)?;
    fs::set_permissions(&socket, fs::Permissions::from_mode(0o600))?;
    let mut events = EventLog::new(&events_path)?;
    let result = (|| {
        for _ in 0..connections {
            let (mut stream, _) = listener.accept()?;
            if negotiate(&mut stream, DEFAULT_EXPORT_BYTES)? {
                transmission(&mut stream, &mut image, DEFAULT_EXPORT_BYTES, &mut events)?;
            }
        }
        image.sync_all()
    })();
    drop(listener);
    let remove_result = fs::remove_file(&socket);
    result.and(remove_result)
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

    fn wait_for_socket(root: &Path) {
        for _ in 0..200 {
            if root.join("nbd.sock").exists() {
                return;
            }
            thread::sleep(Duration::from_millis(5));
        }
        panic!("server socket was not ready");
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
        wait_for_socket(&root);
        wait_for_mode(&root.join("nbd.sock"), 0o600);
        wait_for_mode(&root.join("backing.img"), 0o600);
        wait_for_mode(&root.join("events.jsonl"), 0o600);

        assert_eq!(mode(&root), 0o700);
        assert_eq!(mode(&root.join("nbd.sock")), 0o600);
        assert_eq!(mode(&root.join("backing.img")), 0o600);
        assert_eq!(mode(&root.join("events.jsonl")), 0o600);

        probe(&root.join("nbd.sock")).unwrap();
        server.join().unwrap().unwrap();

        assert!(!root.join("nbd.sock").exists());
        assert_eq!(mode(&root.join("backing.img")), 0o600);
        assert_eq!(mode(&root.join("events.jsonl")), 0o600);

        fs::remove_file(root.join("backing.img")).unwrap();
        fs::remove_file(root.join("events.jsonl")).unwrap();
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
    fn out_of_range_request_is_rejected_without_growing_the_export() {
        let root = fresh_root("bounds");
        let server_root = root.clone();
        let server = thread::spawn(move || serve(&server_root, 1));
        wait_for_socket(&root);

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
        wait_for_socket(&root);

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
