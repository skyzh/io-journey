mod adapter;
mod disk;

use std::env;
use std::io;
use std::path::Path;

use adapter::Adapter;
use disk::FileDisk;
use fuser::MountOption;
use io_journey::journal_fs::{Disk, JournalFs};

fn main() -> io::Result<()> {
    let args = env::args_os().skip(1).collect::<Vec<_>>();
    match args.as_slice() {
        [action, backend] if action == "new-image" => {
            JournalFs::format(FileDisk::new_image(Path::new(backend))?)?;
        }
        [action, backend] if action == "format-device" => {
            let mut disk = FileDisk::open_device(Path::new(backend))?;
            for lba in 0..3 {
                if disk.read_block(lba)?.iter().any(|&byte| byte != 0) {
                    return Err(invalid("the lab device's metadata blocks must be empty"));
                }
            }
            JournalFs::format(disk)?;
        }
        [action, backend] if action == "inspect" => {
            let fs = JournalFs::open(FileDisk::open(Path::new(backend))?)?;
            let allocated = fs.data_blocks() - fs.free_blocks();
            println!("generation {} allocated {}", fs.generation(), allocated);
            for (name, info) in fs.entries()? {
                println!(
                    "{:?}\t{}\t{}",
                    String::from_utf8_lossy(name),
                    info.len,
                    info.blocks
                );
            }
        }
        [action, backend, mountpoint, name] if action == "mount" => {
            let fs = JournalFs::open(FileDisk::open(Path::new(backend))?)?;
            let name = name.to_str().ok_or_else(|| invalid("invalid mount name"))?;
            fuser::mount2(
                Adapter::new(fs),
                mountpoint,
                &[
                    MountOption::FSName(name.to_owned()),
                    MountOption::Subtype("io-journey".into()),
                    MountOption::DefaultPermissions,
                    MountOption::NoDev,
                    MountOption::NoSuid,
                ],
            )?;
        }
        _ => {
            return Err(invalid(
                "use new-image, format-device, inspect, or mount BACKEND MOUNT NAME",
            ))
        }
    }
    Ok(())
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}
