use std::cmp::min;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::time;
use std::fs;

use anyhow::Result;
use log::{debug, warn};
use libc;

use crate::drm_fdinfo::DrmFdinfo;


thread_local! {
    static HERTZ: i64 = unsafe { libc::sysconf(libc::_SC_CLK_TCK) };
}

#[derive(Debug)]
pub struct ProcPids
{
    proc_iter: fs::ReadDir,
}

impl Iterator for ProcPids
{
    type Item = Result<ProcInfo>;

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

            let fpath = nval.path();
            if !fpath.is_dir() {
                continue;
            }

            let fp = fpath.file_name().unwrap().to_str().unwrap();
            if !fp.chars().next().unwrap().is_digit(10) {
                continue;
            }

            let nproc = ProcInfo::from(&fp.to_string());
            if let Err(err) = nproc {
                debug!("ERR: skipping pid {:?}: {:?}", fp, err);
                continue;
            }

            return Some(nproc);
        }
    }
}

#[derive(Debug, Clone)]
pub struct ProcInfo
{
    pub pid: u32,
    pub comm: String,
    pub cmdline: String,
    pub proc_dir: PathBuf,
    cputime_last: u64,
    cputime_delta: u64,
    nr_threads: u64,
    nr_updates: u64,
    ms_elapsed: u64,
    last_update: time::Instant,
}

impl Default for ProcInfo
{
    fn default() -> ProcInfo
    {
        ProcInfo {
            pid: 0,
            comm: String::new(),
            cmdline: String::new(),
            proc_dir: PathBuf::new(),
            cputime_last: 0,
            cputime_delta: 0,
            nr_threads: 0,
            nr_updates: 0,
            ms_elapsed: 0,
            last_update: time::Instant::now(),
        }
    }
}

impl PartialEq for ProcInfo
{
    fn eq(&self, other: &ProcInfo) -> bool {
        self.pid == other.pid &&
            self.comm == other.comm &&
            self.cmdline == other.cmdline
    }
}
impl Eq for ProcInfo {}

impl ProcInfo
{
    pub fn is_valid_pid(pid: &str) -> bool
    {
        if !pid.chars().next().unwrap().is_digit(10) {
            return false;
        }

        let ppath = Path::new("/proc").join(pid);
        if !ppath.is_dir() {
            return false;
        }

        true
    }

    pub fn children_pids(&self) -> Result<VecDeque<String>>
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

    pub fn drm_fdinfos(&self) -> Result<Vec<DrmFdinfo>>
    {
        let mut res: Vec<DrmFdinfo> = Vec::new();
        let fddir = self.proc_dir.join("fd");
        let fdinfodir = self.proc_dir.join("fdinfo");

        for et in fddir.read_dir()? {
            let et = et?;

            let mut mn: u32 = 0;
            let is_drm_fd = DrmFdinfo::is_drm_fd(&et.path(), &mut mn);
            if let Err(err) = is_drm_fd {
                debug!("ERR: failed to find fd {:?}: {:?}", et.path(), err);
                continue;
            }
            let is_drm_fd = is_drm_fd.unwrap();
            if !is_drm_fd {
                continue;
            }

            let fipath = fdinfodir.join(et.path().file_name().unwrap());
            let finfo = DrmFdinfo::from(&fipath, mn);
            if let Err(err) = finfo {
                debug!("ERR: failed to parse DRM fdinfo {:?}: {:?}", fipath, err);
                continue;
            }
            let finfo = finfo.unwrap();

            if finfo.pci_dev.is_empty() {
                debug!("INF: DRM fdinfo {:?} with no PCI dev, ignoring.",
                    fipath);
                continue;
            }

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

        let nr_cpus = unsafe { libc::sysconf(libc::_SC_NPROCESSORS_ONLN) };
        let hz = HERTZ.with(|hertz|
            if *hertz > 0 {
                *hertz as f64
            } else {
                100.0
            }
        );

        let delta_ms = (self.cputime_delta as f64 / hz) * 1000.0;
        let mut res = (delta_ms / self.ms_elapsed as f64) * 100.0;

        let max_pct = min(self.nr_threads, nr_cpus as u64) as f64 * 100.0;
        if res > max_pct {
            warn!("Process {:?} (pid {}) CPU utilization at {:.1}%, \
                clamped to max {:.1}% (# CPUs: {}, # threads: {}).",
                self.comm, self.pid, res, max_pct, nr_cpus, self.nr_threads);
            res = max_pct;
        }
        res
    }

    pub fn update(&mut self) -> Result<()>
    {
        let stpath = self.proc_dir.join("stat");
        let ststr = fs::read_to_string(&stpath)?;

        let idx = ststr.rfind(')').unwrap();
        let stv: Vec<&str> = ststr[idx + 1..].split_whitespace().collect();

        let utime: u64 = stv[11].parse()?;
        let stime: u64 = stv[12].parse()?;
        self.nr_threads = stv[17].parse()?;

        let cputime = utime + stime;
        self.cputime_delta = cputime - self.cputime_last;
        self.cputime_last = cputime;

        self.ms_elapsed = self.last_update.elapsed().as_millis() as u64;
        self.last_update = time::Instant::now();
        self.nr_updates += 1;

        Ok(())
    }

    pub fn from(npid: &String) -> Result<ProcInfo>
    {
        let mut qmpi = ProcInfo {
            pid: npid.parse()?,
            comm: String::new(),
            cmdline: String::new(),
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

    pub fn iter_proc_pids() -> Result<ProcPids>
    {
        Ok(ProcPids { proc_iter: Path::new("/proc").read_dir()?, })
    }
}
