use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::os::linux::fs::MetadataExt;
use std::fs;

use anyhow::Result;
use log::debug;
use libc;

fn is_drm_fd(file: &Path) -> Result<i64>
{
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

pub fn find_drm_fds_for_pid_tree_at(base_pid: &String) -> Result<Vec<PathBuf>>
{
    let mut pbuf = PathBuf::from("/proc");
    let mut pidq = VecDeque::from([base_pid.clone(),]);
    let mut fds: Vec<PathBuf> = Vec::new();

    while !pidq.is_empty() {
        let npid = pidq.pop_front();
        pbuf.push(npid.unwrap());

        // search for all DRM fds
        pbuf.push("fd");

        let iter = pbuf.read_dir();
        if let Err(err) = iter {
            debug!("Error reading dir {:?}: {:?}", pbuf, err);
            pbuf.pop();
            continue;
        }

        let mut iter = iter.unwrap();
        while let Some(item) = iter.next() {
            if let Err(err) = item {
                debug!("Error reading dir {:?}: {:?}", pbuf, err);
                pbuf.pop();
                continue;
            }

            let etp = item.unwrap().path();
            let res = is_drm_fd(&etp);
            if let Err(err) = res {
                debug!("Error checking if {:?} is a DRM fd: {:?}", etp, err);
                pbuf.pop();
                continue;
            }

            let res = res.unwrap();
            if res >= 0 {
                fds.push(etp);
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

    Ok(fds)
}
