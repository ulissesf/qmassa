use std::collections::{BTreeMap, HashMap, VecDeque};
use std::cell::{RefCell, RefMut};
use std::path::PathBuf;
use std::rc::{Rc, Weak};
use std::time;

use anyhow::{bail, Result};
use log::{debug, warn};
use serde::{Deserialize, Serialize};

use crate::proc_info::ProcInfo;
use crate::drm_fdinfo::{DrmEngine, DrmMemRegion, DrmFdinfo};
use crate::drm_drivers::DrmDriver;


#[derive(Debug)]
pub struct DrmEnginesAcum
{
    pub acum_time: u64,
    pub acum_cycles: u64,
    pub acum_total_cycles: u64,
}

impl DrmEnginesAcum
{
    pub fn new() -> DrmEnginesAcum
    {
        DrmEnginesAcum {
            acum_time: 0,
            acum_cycles: 0,
            acum_total_cycles: 0,
        }
    }
}

#[derive(Debug)]
pub struct DrmEngineDelta
{
    pub delta_time: u64,
    pub delta_cycles: u64,
    pub delta_total_cycles: u64,
}

impl DrmEngineDelta
{
    pub fn new() -> DrmEngineDelta
    {
        DrmEngineDelta {
            delta_time: 0,
            delta_cycles: 0,
            delta_total_cycles: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DrmClientMemInfo
{
    pub smem_used: u64,
    pub smem_rss: u64,
    pub vram_used: u64,
    pub vram_rss: u64,
}

impl DrmClientMemInfo
{
    pub fn acum(&mut self, mi: &DrmClientMemInfo)
    {
        self.smem_used += mi.smem_used;
        self.smem_rss += mi.smem_rss;
        self.vram_used += mi.vram_used;
        self.vram_rss += mi.vram_rss;
    }

    pub fn new() -> DrmClientMemInfo
    {
        DrmClientMemInfo {
            smem_used: 0,
            smem_rss: 0,
            vram_used: 0,
            vram_rss: 0,
        }
    }
}

pub type ProcInfoRef = Rc<RefCell<ProcInfo>>;

#[derive(Debug)]
pub struct DrmClientInfo
{
    pub pci_dev: String,
    pub drm_minor: u32,
    pub client_id: u32,
    pub proc: ProcInfoRef,
    pub fdinfo_path: PathBuf,
    pub shared_fdinfos: Vec<PathBuf>,
    engs_last: HashMap<String, DrmEngine>,
    engs_delta: HashMap<String, DrmEngineDelta>,
    engs_updates: HashMap<String, u64>,
    engs_acum: DrmEnginesAcum,
    mem_regions: HashMap<String, DrmMemRegion>,
    nr_updates: u64,
    ms_elapsed: u64,
    last_update: time::Instant,
    driver: Option<Weak<RefCell<dyn DrmDriver>>>,
}

impl Default for DrmClientInfo
{
    fn default() -> DrmClientInfo
    {
        DrmClientInfo {
            pci_dev: String::new(),
            drm_minor: 0,
            client_id: 0,
            proc: Rc::new(RefCell::new(ProcInfo::default())),
            fdinfo_path: PathBuf::new(),
            shared_fdinfos: Vec::new(),
            engs_last: HashMap::new(),
            engs_delta: HashMap::new(),
            engs_updates: HashMap::new(),
            engs_acum: DrmEnginesAcum::new(),
            mem_regions: HashMap::new(),
            nr_updates: 0,
            ms_elapsed: 0,
            last_update: time::Instant::now(),
            driver: None,
        }
    }
}

impl DrmClientInfo
{
    pub fn mem_info(&self) -> DrmClientMemInfo
    {
        if let Some(w_ref) = &self.driver {
            if let Some(drv_ref) = w_ref.upgrade() {
                let mut drv_b = drv_ref.borrow_mut();
                if let Ok(res) = drv_b.client_mem_info(&self.mem_regions) {
                    return res;
                }
            }
        } else if let Some(mrg) = self.mem_regions.get("memory") {
            let mut cmi = DrmClientMemInfo::new();
            cmi.smem_rss = mrg.resident;
            cmi.smem_used = mrg.total;
            return cmi;
        }

        DrmClientMemInfo::new()
    }

    pub fn eng_utilization(&self, eng: &String) -> f64
    {
        if !self.engs_last.contains_key(eng) {
            return 0.0;
        }
        if *self.engs_updates.get(eng).unwrap() < 2 {
            return 0.0;
        }

        let acum = &self.engs_acum;
        if acum.acum_time == 0 && acum.acum_cycles == 0 {
            return 0.0;
        }

        let ed = self.engs_delta.get(eng).unwrap();
        let cap = self.engs_last.get(eng).unwrap().capacity as f64;

        let mut res: f64 = 0.0;
        if acum.acum_cycles > 0 && ed.delta_total_cycles > 0 {
            res = (ed.delta_cycles as f64 * 100.0) /
                (ed.delta_total_cycles as f64 * cap);
        } else if acum.acum_time > 0 {
            res = ((ed.delta_time as f64 / 1000000.0) * 100.0) /
                (self.ms_elapsed as f64 * cap);
        }

        if res > 100.0 {
            warn!("{}: engine {:?} utilization at {:?}%, clamped to 100%.",
                &self.pci_dev, eng, res);
            res = 100.0;
        }
        res
    }

    pub fn engines(&self) -> Vec<&String>
    {
        let mut res: Vec<&String> = self.engs_delta.keys().collect::<Vec<&_>>();
        res.sort();

        res
    }

    fn total_mem(&self) -> u64
    {
        let mut tot: u64 = 0;
        for reg in self.mem_regions.values() {
            tot += reg.total;
        }

        tot
    }

    pub fn is_active(&self) -> bool
    {
        let acum = &self.engs_acum;
        if acum.acum_time > 0 ||
            (acum.acum_cycles > 0 && acum.acum_total_cycles > 0) {
            return true;
        }

        if self.total_mem() > 0 {
            return true;
        }

        false
    }

    pub fn update(&mut self, pinfo: ProcInfoRef, fdi: DrmFdinfo)
    {
        // handle process and fdinfo path updates
        self.proc = pinfo;
        self.fdinfo_path = fdi.path;

        // handle new engines showing up in a client's DRM fdinfo
        // or the very unlikely (not possible?) removal of an engine
        for nm in fdi.engines.keys() {
            if !self.engs_last.contains_key(nm) {
                self.engs_last.insert(nm.clone(),
                    DrmEngine::from(&fdi.engines[nm]));
                self.engs_delta.insert(nm.clone(), DrmEngineDelta::new());
                self.engs_updates.insert(nm.clone(), 0);
            }
        }
        self.engs_last.retain(|k, _| fdi.engines.contains_key(k));
        self.engs_delta.retain(|k, _| fdi.engines.contains_key(k));
        self.engs_updates.retain(|k, _| fdi.engines.contains_key(k));

        // handle engines and mem regions updates
        self.engs_acum.acum_time = 0;
        self.engs_acum.acum_cycles = 0;
        self.engs_acum.acum_total_cycles = 0;

        for (nm, oeng) in self.engs_last.iter_mut() {
            let deng = self.engs_delta.get_mut(nm).unwrap();
            let neng = fdi.engines.get(nm).unwrap();

            deng.delta_time = neng.time.saturating_sub(oeng.time);
            if deng.delta_time > 0 {
                self.engs_acum.acum_time += neng.time;
            }
            oeng.time = neng.time;

            deng.delta_cycles = neng.cycles.saturating_sub(oeng.cycles);
            if deng.delta_cycles > 0 {
                self.engs_acum.acum_cycles += neng.cycles;
            }
            oeng.cycles = neng.cycles;

            deng.delta_total_cycles = neng.total_cycles
                .saturating_sub(oeng.total_cycles);
            if deng.delta_total_cycles > 0 {
                self.engs_acum.acum_total_cycles += neng.total_cycles;
            }
            oeng.total_cycles = neng.total_cycles;

            self.engs_updates.entry(nm.clone()).and_modify(|nr| *nr += 1);
        }
        self.mem_regions = fdi.mem_regions;

        // one more update for this DRM client
        self.ms_elapsed = fdi.time_sampled
            .saturating_duration_since(self.last_update).as_millis() as u64;
        self.last_update = fdi.time_sampled;
        self.nr_updates += 1;
    }

    pub fn set_driver(&mut self, drv_wref: Weak<RefCell<dyn DrmDriver>>)
    {
        self.driver = Some(drv_wref);
    }

    pub fn from(pinfo: ProcInfoRef, fdi: DrmFdinfo) -> DrmClientInfo
    {
        let mut cli = DrmClientInfo {
            pci_dev: fdi.pci_dev.clone(),
            drm_minor: fdi.drm_minor,
            client_id: fdi.client_id,
            ..Default::default()
        };

        for nm in fdi.engines.keys() {
            cli.engs_last.insert(nm.clone(),
                DrmEngine::from(&fdi.engines[nm]));
            cli.engs_delta.insert(nm.clone(), DrmEngineDelta::new());
            cli.engs_updates.insert(nm.clone(), 0);
        }

        cli.update(pinfo, fdi);

        cli
    }
}

pub type DrmClientInfoMap = BTreeMap<(u32, u32), DrmClientInfo>;
pub type DrmClientInfoMapRef = Rc<RefCell<DrmClientInfoMap>>;

#[derive(Debug)]
pub struct DrmClients
{
    base_pid: String,
    infos: HashMap<String, DrmClientInfoMapRef>,
    procs: HashMap<(u32, String, String), ProcInfoRef>,
}

impl DrmClients
{
    pub fn set_dev_clients_driver(&mut self,
        dev: &String, drv_wref: Weak<RefCell<dyn DrmDriver>>)
    {
        if !self.infos.contains_key(dev) {
            return;
        }
        let mut vlst = self.infos.get_mut(dev).unwrap().borrow_mut();

        for cliref in vlst.values_mut() {
            cliref.set_driver(Weak::clone(&drv_wref));
        }
    }

    pub fn device_clients(&self, dev: &String) -> Option<DrmClientInfoMapRef>
    {
        if let Some(vref) = self.infos.get(dev) {
            return Some(vref.clone());
        }

        None
    }

    fn map_has_client<'a>(
        map: &'a mut HashMap<String, DrmClientInfoMapRef>,
        dev: &'a String,
        minor: u32, id: u32) -> Option<RefMut<'a, DrmClientInfo>>
    {
        if !map.contains_key(dev) {
            return None;
        }

        let key = (minor, id);
        let vlst = map.get_mut(dev).unwrap().borrow_mut();
        if !vlst.contains_key(&key) {
            return None;
        }

        Some(RefMut::map(vlst, |v| v.get_mut(&key).unwrap()))
    }

    fn map_remove_client(
        map: &mut HashMap<String, DrmClientInfoMapRef>,
        dev: &String, minor: u32, id: u32) -> Option<DrmClientInfo>
    {
        if !map.contains_key(dev) {
            return None
        }

        let key = (minor, id);
        let mut vlst = map.get(dev).unwrap().borrow_mut();

        vlst.remove(&key)
    }

    fn map_insert_client(
        map: &mut HashMap<String, DrmClientInfoMapRef>,
        dev: String, cli: DrmClientInfo)
    {
        let key = (cli.drm_minor, cli.client_id);

        if !map.contains_key(&dev) {
            let mut vlst: BTreeMap<(u32, u32), DrmClientInfo> =
                BTreeMap::new();
            vlst.insert(key, cli);
            map.insert(dev, Rc::new(RefCell::new(vlst)));
        } else {
            let mut vref = map.get(&dev).unwrap().borrow_mut();
            vref.insert(key, cli);
        }
    }

    fn proc_info_ref(&mut self, proc: &ProcInfo) -> ProcInfoRef
    {
        let key = (proc.pid, proc.comm.clone(), proc.cmdline.clone());
        if !self.procs.contains_key(&key) {
            self.procs.insert(key.clone(),
                Rc::new(RefCell::new(proc.clone())));
        }

        let pref = self.procs.get(&key).unwrap();
        if let Err(err) = pref.borrow_mut().update() {
            debug!("ERR: failed to update process info for {:?}: {:?}",
                pref.borrow(), err);
        }

        pref.clone()
    }

    fn procs_info_cleanup(&mut self)
    {
        self.procs.retain(|_, pref| Rc::strong_count(&pref) >= 2);
    }

    fn process_fdinfos(&mut self,
        ninfos: &mut HashMap<String, DrmClientInfoMapRef>,
        nproc: &ProcInfo, fdinfos: Vec<DrmFdinfo>)
    {
        let nproc_ref = self.proc_info_ref(nproc);

        for fdi in fdinfos {
            if let Some(mut cliref) = DrmClients::map_has_client(ninfos,
                &fdi.pci_dev, fdi.drm_minor, fdi.client_id) {
                debug!("INF: repeated DRM client: fdinfo={:?}, \
                    drm-minor={:?}, drm-client-id={:?}",
                    fdi.path, fdi.drm_minor, fdi.client_id);
                cliref.shared_fdinfos.push(fdi.path);
                continue;
            }

            let pci_dev = fdi.pci_dev.clone();
            if let Some(mut cli) = DrmClients::map_remove_client(
                &mut self.infos, &fdi.pci_dev, fdi.drm_minor, fdi.client_id) {
                cli.shared_fdinfos.clear();
                cli.update(nproc_ref.clone(), fdi);
                DrmClients::map_insert_client(ninfos, pci_dev, cli);
            } else {
                let cli = DrmClientInfo::from(nproc_ref.clone(), fdi);
                DrmClients::map_insert_client(ninfos, pci_dev, cli);
            }
        }
    }

    fn scan_all_pids(&mut self) -> Result<()>
    {
        let mut ninfos: HashMap<String, DrmClientInfoMapRef> = HashMap::new();

        let proc_iter = ProcInfo::iter_proc_pids();
        if let Err(err) = proc_iter {
            debug!("ERR: couldn't get pids info in /proc: {:?}", err);
        } else {
            let proc_iter = proc_iter.unwrap();
            for nproc in proc_iter {
                // got next process info
                if let Err(err) = nproc {
                    debug!("ERR: error iterating through /proc pids: {:?}",
                        err);
                    break;
                }
                let nproc = nproc.unwrap();

                // search and parse all DRM fdinfo from npid process
                let fdinfos = nproc.drm_fdinfos();
                if let Err(err) = fdinfos {
                    debug!("ERR: failed to get DRM fdinfos from {:?}: {:?}",
                        nproc.pid, err);
                    continue;
                }
                let fdinfos = fdinfos.unwrap();

                // process DRM client infos based on DRM fdinfos
                if !fdinfos.is_empty() {
                    self.process_fdinfos(&mut ninfos, &nproc, fdinfos);
                }
            }
        }

        // update DRM client infos
        self.infos = ninfos;

        // cleanup unused process info
        self.procs_info_cleanup();

        Ok(())
    }

    fn scan_pid_tree(&mut self) -> Result<()>
    {
        let mut ninfos: HashMap<String, DrmClientInfoMapRef> = HashMap::new();
        let mut pidq = VecDeque::from([self.base_pid.clone(),]);

        while !pidq.is_empty() {
            let npid = pidq.pop_front().unwrap();

            // new process info
            let nproc = ProcInfo::from(&npid);
            if let Err(err) = nproc {
                debug!("ERR: Couldn't get proc info for {:?}: {:?}",
                    npid, err);
                continue;
            }
            let nproc = nproc.unwrap();

            // search and parse all DRM fdinfo from npid process
            let fdinfos = nproc.drm_fdinfos();
            if let Err(err) = fdinfos {
                debug!("ERR: failed to get DRM fdinfos from {:?}: {:?}",
                    npid, err);
                continue;
            }
            let fdinfos = fdinfos.unwrap();

            // prcess DRM client infos based on DRM fdinfos
            if !fdinfos.is_empty() {
                self.process_fdinfos(&mut ninfos, &nproc, fdinfos);
            }

            // add all child processes
            let chids = nproc.children_pids();
            if let Err(err) = chids {
                debug!("ERR: failed to get children pids for {:?}: {:?}",
                    npid, err);
            } else {
                let mut chids = chids.unwrap();
                pidq.append(&mut chids);
            }
        }

        // update DRM client infos
        self.infos = ninfos;

        // cleanup unused process info
        self.procs_info_cleanup();

        Ok(())
    }

    pub fn refresh(&mut self) -> Result<()>
    {
        if self.base_pid.is_empty() {
            self.scan_all_pids()?;
        } else {
            self.scan_pid_tree()?;
        }

        Ok(())
    }

    pub fn from_pid_tree(at_pid: &str) -> Result<DrmClients>
    {
        if !at_pid.is_empty() && !ProcInfo::is_valid_pid(at_pid) {
            bail!("Not a valid PID: {}", at_pid);
        }

        Ok(DrmClients {
            base_pid: at_pid.to_string(),
            infos: HashMap::new(),
            procs: HashMap::new(),
        })
    }
}
