use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::time;
use std::fs;

use anyhow::Result;
use log::debug;
use libc;

use crate::qmdrmfdinfo::QmDrmFdinfo;


thread_local! {
    static HERTZ: i64 = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
    static NR_CPUS_ONLINE: i64 = unsafe {
        libc::sysconf(libc::_SC_NPROCESSORS_ONLN)};
}

#[derive(Debug)]
pub struct QmProcPids
{
    proc_iter: fs::ReadDir,
}

impl Iterator for QmProcPids
{
    type Item = Result<QmProcInfo>;

    fn next(&mut self) -> Option<Self::Item>
    {
        loop {
            let nval = self.proc_iter.next();
            if nval.is_none() {
                return None;
            }
            let nval = nval.unwrap();

            if let Err(err) = nval {
                return Some(Err(err.into()));
            }
            let nval = nval.unwrap();

            if !nval.path().is_dir() {
                continue;
            }

            let fpath = nval.path();
            let fp = fpath.file_name().unwrap().to_str().unwrap();

            if !fp.chars().next().unwrap().is_digit(10) {
                continue;
            }

            let nproc = QmProcInfo::from(&fp.to_string());
            if let Err(err) = nproc {
                debug!("ERR: skipping pid {:?}: {:?}", fp, err);
                continue;
            }

            return Some(nproc);
        }
    }
}

#[derive(Debug, Clone)]
pub struct QmProcInfo
{
    pub pid: u32,
    pub comm: String,
    pub cmdline: String,
    pub proc_dir: PathBuf,
    cputime_last: u64,
    cputime_delta: u64,
    nr_updates: u64,
    ms_elapsed: u64,
    last_update: time::Instant,
}

impl Default for QmProcInfo
{
    fn default() -> QmProcInfo
    {
        QmProcInfo {
            pid: 0,
            comm: String::from(""),
            cmdline: String::from(""),
            proc_dir: PathBuf::new(),
            cputime_last: 0,
            cputime_delta: 0,
            nr_updates: 0,
            ms_elapsed: 0,
            last_update: time::Instant::now(),
        }
    }
}

impl PartialEq for QmProcInfo
{
    fn eq(&self, other: &QmProcInfo) -> bool {
        self.pid == other.pid &&
            self.comm == other.comm &&
            self.cmdline == other.cmdline
    }
}
impl Eq for QmProcInfo {}

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
            let is_drm_fd = QmDrmFdinfo::is_drm_fd(&et.path(), &mut mn);
            if let Err(err) = is_drm_fd {
                debug!("ERR: failed to find fd {:?}: {:?}", et.path(), err);
                continue;
            }
            let is_drm_fd = is_drm_fd.unwrap();
            if !is_drm_fd {
                continue;
            }

            let fipath = fdinfodir.join(et.path().file_name().unwrap());
            let finfo = QmDrmFdinfo::from(&fipath, mn);
            if let Err(err) = finfo {
                debug!("ERR: failed to parse DRM fdinfo {:?}: {:?}", fipath, err);
                continue;
            }
            let finfo = finfo.unwrap();

            res.push(finfo);
        }

        Ok(res)
    }

    pub fn cpu_utilization(&self) -> f64
    {
        if self.nr_updates < 2 {
            return 0.0;
        }
        if self.cputime_delta == 0 || self.ms_elapsed == 0 {
            return 0.0;
        }

        let mut total_cpu_time: f64 = self.ms_elapsed as f64;
        NR_CPUS_ONLINE.with(|nr_cpus| {
            if *nr_cpus > 1 {
                total_cpu_time *= *nr_cpus as f64;
            }
        });

        (self.cputime_delta as f64 / total_cpu_time ) * 100.0
    }

    pub fn update(&mut self) -> Result<()>
    {
        let stpath = self.proc_dir.join("stat");
        let ststr = fs::read_to_string(&stpath)?;

        let idx = ststr.rfind(')').unwrap();
        let stv: Vec<&str> = ststr[idx + 1..].split_whitespace().collect();

        // cputime == utime + stime
        let mut utime: u64 = stv[11].parse()?;
        let mut stime: u64 = stv[12].parse()?;
        HERTZ.with(|hertz| {
            let hz: u64 = if *hertz > 0 { *hertz as u64 } else { 100 };
            utime = (utime / hz) * 1000;
            stime = (stime / hz) * 1000;
        });

        let cputime = utime + stime;
        self.cputime_delta = cputime - self.cputime_last;
        self.cputime_last = cputime;

        self.nr_updates += 1;
        self.ms_elapsed = self.last_update.elapsed().as_millis() as u64;
        self.last_update = time::Instant::now();

        Ok(())
    }

    pub fn from(npid: &String) -> Result<QmProcInfo>
    {
        let mut qmpi = QmProcInfo {
            pid: npid.parse()?,
            comm: String::from(""),
            cmdline: String::from(""),
            proc_dir: Path::new("/proc").join(npid.as_str()),
            ..Default::default()
        };

        let cpath = qmpi.proc_dir.join("comm");
        let cstr = fs::read_to_string(&cpath)?;
        qmpi.comm.push_str(cstr.trim_end());

        let cpath = qmpi.proc_dir.join("cmdline");
        let cstr = fs::read_to_string(&cpath)?;
        qmpi.cmdline.push_str(&cstr.replace("\0", " "));

        qmpi.update()?;

        Ok(qmpi)
    }

    pub fn iter_proc_pids() -> Result<QmProcPids>
    {
        Ok(QmProcPids { proc_iter: Path::new("/proc").read_dir()?, })
    }
}
