use std::error::Error;
use std::fs::{self, OpenOptions};
use std::io;
use std::os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use libublk::{ctrl::UblkCtrl, sys};

pub fn character_identity(id: u32) -> io::Result<Option<String>> {
    match fs::symlink_metadata(format!("/dev/ublkc{id}")) {
        Ok(meta) if meta.file_type().is_char_device() => Ok(Some(format!(
            "{}:{}:{}",
            meta.dev(),
            meta.ino(),
            meta.rdev()
        ))),
        Ok(_) => Err(io::Error::other(
            "ublk character path has the wrong file type",
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn runtime_json(id: u32) -> PathBuf {
    PathBuf::from(UblkCtrl::run_dir()).join(format!("{id:04}.json"))
}

fn verify_metadata(path: &Path, token: &str) -> Result<bool, Box<dyn Error>> {
    let file = match OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error.into()),
    };
    if !file.metadata()?.is_file() {
        return Err(io::Error::other("ublk runtime metadata must be a regular file").into());
    }
    let json: serde_json::Value = serde_json::from_reader(file)?;
    if json["target_data"].as_str() != Some(token) {
        return Err(io::Error::other("ublk runtime metadata is owned by a different run").into());
    }
    Ok(true)
}

fn open_owned_device(
    id: u32,
    pid: i32,
    identity: &str,
    server_gone: bool,
) -> Result<UblkCtrl, Box<dyn Error>> {
    if pid <= 0 || character_identity(id)?.as_deref() != Some(identity) {
        return Err(io::Error::other("ublk character device does not match the allocation").into());
    }
    let ctrl = UblkCtrl::new_simple(i32::try_from(id)?)?;
    ctrl.read_dev_info()?;
    let info = ctrl.dev_info();
    if info.dev_id != id || (info.ublksrv_pid != pid && !(server_gone && info.ublksrv_pid <= 0)) {
        return Err(io::Error::other("ublk device is owned by a different server").into());
    }
    Ok(ctrl)
}

pub fn check_running(id: u32, pid: i32, identity: &str, token: &str) -> Result<(), Box<dyn Error>> {
    open_owned_device(id, pid, identity, false)?;
    if !verify_metadata(&runtime_json(id), token)? {
        return Err(io::Error::other("ublk runtime metadata is missing").into());
    }
    Ok(())
}

pub fn remove_after_reap(
    id: u32,
    pid: i32,
    identity: &str,
    token: &str,
) -> Result<(), Box<dyn Error>> {
    if pid <= 0 || Path::new(&format!("/proc/{pid}")).exists() {
        return Err(io::Error::other("reap the ublk server before deleting its device").into());
    }
    let metadata = runtime_json(id);
    let had_metadata = verify_metadata(&metadata, token)?;
    if character_identity(id)?.is_some() {
        let ctrl = open_owned_device(id, pid, identity, true)?;
        if u32::from(ctrl.dev_info().state) == sys::UBLK_S_DEV_LIVE {
            ctrl.kill_dev()?;
        }
        ctrl.del_dev()?;
    } else if had_metadata {
        fs::remove_file(metadata)?;
    }
    Ok(())
}
