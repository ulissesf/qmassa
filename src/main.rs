use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::io;
use std::fs;
use std::os::linux::fs::MetadataExt;
use libc;

use clap::Parser;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    #[arg(short, long, default_value = "1")]
    pid: Option<String>,
}

fn is_drm_fd(file: &Path) -> Result<i64, io::Error> {
    let met = fs::metadata(file)?;
    let st_mode = met.st_mode();
    let st_rdev = met.st_rdev();

    // check it's char device and major 226 for DRM device
    let mj: u32;
    let mn: u32;
    unsafe {
        mj = libc::major(st_rdev);
        mn = libc::minor(st_rdev);
    }
    if st_mode & libc::S_IFMT == libc::S_IFCHR && mj == 226 {
        return Ok(mn.into());
    }
    Ok(-1)
}

fn main() -> Result<(), io::Error> {
    let args = Args::parse();
    let base_pid = args.pid.unwrap();

    let mut pbuf = PathBuf::from("/proc");
    let mut pidq = VecDeque::from([base_pid,]);

    while !pidq.is_empty() {
        let npid = pidq.pop_front();
        pbuf.push(npid.unwrap());

        // search for all DRM fds
        pbuf.push("fd");
        for et in pbuf.read_dir()? {
            let et = et?;
            if is_drm_fd(&et.path())? >= 0 {
                println!("DRM fd = {:?}", et);
            }
        }
        pbuf.pop();      // pop "fd" 

        // add all child processes to search
        pbuf.push("task");
        for et in pbuf.read_dir()? {
            let et = et?;
            if et.path().is_dir() {
                let children = et.path().join("children");
                let line: String = fs::read_to_string(&children)?;
                for chid in line.split_whitespace() {
                    pidq.push_back(chid.to_string());
                }
            }
        }

        pbuf.pop();     // pop "task"
        pbuf.pop();     // pop "pid dir"
    }

    Ok(())
}
