use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::fs;

use anyhow::Result;

use crate::qmdrmfdinfo::QmDrmFdinfo;


#[derive(Debug, Clone)]
pub struct QmProcInfo
{
    pub pid: u32,
    pub comm: String,
    pub proc_dir: PathBuf,
}

impl QmProcInfo
{
    pub fn get_children_procs(&self) -> Result<VecDeque<String>>
    {
        let mut chids: VecDeque<String> = VecDeque::new();

        let tpath = self.proc_dir.join("task");
        for et in tpath.read_dir()? {
            let et = et?;

            if et.path().is_dir() {
                let children = et.path().join("children");
                let line: String = fs::read_to_string(&children)?;
                for id in line.split_whitespace() {
                    chids.push_back(id.to_string());
                }
            }
        }

        Ok(chids)
    }

    pub fn get_drm_fdinfos(&self) -> Result<Vec<QmDrmFdinfo>>
    {
        let mut res: Vec<QmDrmFdinfo> = Vec::new();
        let fddir = self.proc_dir.join("fd");
        let fdinfodir = self.proc_dir.join("fdinfo");

        for et in fddir.read_dir()? {
            let et = et?;

            let mut mn: u32 = 0;
            if !QmDrmFdinfo::is_drm_fd(&et.path(), &mut mn)? {
                continue;
            }

            let fipath = fdinfodir.join(et.path().file_name().unwrap());
            let finfo = QmDrmFdinfo::from(&fipath, mn)?;

            res.push(finfo);
        }

        Ok(res)
    }

    pub fn from(npid: &String) -> Result<QmProcInfo>
    {
         let mut qmpi = QmProcInfo {
             pid: npid.parse()?,
             comm: String::from(""),
             proc_dir: Path::new("/proc").join(npid.as_str()),
         };

         let cpath = qmpi.proc_dir.join("comm");
         let cstr = fs::read_to_string(&cpath)?;
         qmpi.comm = cstr.strip_suffix("\n").unwrap().to_string();

         Ok(qmpi)
    }
}
