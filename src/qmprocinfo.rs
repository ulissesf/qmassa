use std::collections::{VecDeque, HashMap};
use std::path::{Path, PathBuf};
use std::fs;

use anyhow::Result;
use log::debug;

use crate::qmdevice::QmDevice;
use crate::qmdrmfdinfo::QmDrmFdinfo;


#[derive(Debug)]
pub struct QmProcInfo<'b>
{
    pub pid: u32,
    pub comm: String,
    pub pidpbuf: PathBuf,
    pub stats: Vec<QmDrmFdinfo<'b>>,
}

impl<'b> QmProcInfo<'b>
{
    fn find_children_procs(&self) -> Result<VecDeque<String>>
    {
        let mut chids: VecDeque<String> = VecDeque::new();

        let tpath = self.pidpbuf.join("task");
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

    pub fn get_drm_fdinfo_stats<'a>(&'a mut self, done: &'a mut HashMap<(u32,u32),bool>, qmds: &'b Vec<QmDevice>) -> Result<()>
    {
        let fdpath = self.pidpbuf.join("fd");
        let infopath = self.pidpbuf.join("fdinfo");

        for et in fdpath.read_dir()? {
            let et = et?;

            let mut mn: u32 = 0;
            if !QmDrmFdinfo::is_drm_fd(&et.path(), &mut mn)? {
                continue;
            }

            let mut qmd: &QmDevice = &qmds[0];
            for d in qmds {
                if d.devnum.1 == mn {
                    qmd = &d;
                }
            }

            let ipath = infopath.join(et.path().file_name().unwrap());
            let info = QmDrmFdinfo::from_drm_fdinfo(&ipath, qmd)?;

            if done.contains_key(&(mn, info.id)) {
                debug!("Repeated DRM client for minor {:?} and ID {:?}", mn, info.id);
                continue;
            }

            done.insert((mn, info.id), true);
            self.stats.push(info);
        }

        Ok(())
    }

    pub fn from_pid<'a>(npid: &'a String) -> Result<QmProcInfo<'b>>
    {
         let mut qmps = QmProcInfo {
             pid: npid.parse()?,
             comm: String::from(""),
             pidpbuf: Path::new("/proc").join(npid.as_str()),
             stats: Vec::new(),
         };

         let cpath = qmps.pidpbuf.join("comm");
         let cstr = fs::read_to_string(&cpath)?;
         qmps.comm = cstr.strip_suffix("\n").unwrap().to_string();

         Ok(qmps)
    }

    pub fn from_pid_tree(base_pid: &'b String, qmds: &'b Vec<QmDevice>) -> Result<Vec<QmProcInfo<'b>>>
    {
        let mut pstats: Vec<QmProcInfo> = Vec::new();
        let mut pidq = VecDeque::from([base_pid.clone(),]);
        let mut done: HashMap<(u32,u32),bool> = HashMap::new();

        while !pidq.is_empty() {
            let npid = pidq.pop_front().unwrap();

            // new npid process usage stats
            let nstat = QmProcInfo::from_pid(&npid);
            if let Err(err) = nstat {
                debug!("ERR: Couldn't get data for process {:?}: {:?}", npid, err);
                continue;
            }
            let mut nstat = nstat.unwrap();

            // search and parse all DRM fdinfo from npid process
            if let Err(err) = nstat.get_drm_fdinfo_stats(&mut done, qmds) {
                debug!("ERR: failed to get fdinfo usage stats from {:?}: {:?}", npid, err);
                continue;
            }

            // add all child processes
            let chids = nstat.find_children_procs();
            if let Err(err) = chids {
                debug!("ERR: failed to get children info for {:?}: {:?}", npid, err);
                continue;
            }
            let mut chids = chids.unwrap();

            pidq.append(&mut chids);
            if nstat.stats.len() > 0 {
                pstats.push(nstat);
            }
       }

        Ok(pstats)
    }
}
