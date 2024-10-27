use std::collections::{VecDeque, HashMap};
use std::cell::{RefCell, RefMut};
use std::path::PathBuf;
use std::rc::{Rc, Weak};
use std::time;

use anyhow::Result;
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

#[derive(Debug, Serialize, Deserialize)]
pub struct DrmClientMemInfo
{
    pub smem_used: u64,
    pub smem_rss: u64,
    pub vram_used: u64,
    pub vram_rss: u64,
}

impl DrmClientMemInfo
{
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

#[derive(Debug)]
pub struct DrmClientInfo
{
    pub pci_dev: String,
    pub drm_minor: u32,
    pub client_id: u32,
    pub proc: ProcInfo,
    pub fdinfo_path: PathBuf,
    pub shared_procs: Vec<(ProcInfo, PathBuf)>,
    engs_last: HashMap<String, DrmEngine>,
    engs_delta: HashMap<String, DrmEngineDelta>,
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
            pci_dev: String::from(""),
            drm_minor: 0,
            client_id: 0,
            proc: ProcInfo::default(),
            fdinfo_path: PathBuf::new(),
            shared_procs: Vec::new(),
            engs_last: HashMap::new(),
            engs_delta: HashMap::new(),
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
        }

        DrmClientMemInfo::new()
    }

    pub fn eng_utilization(&self, eng: &String) -> f64
    {
        if !self.engs_last.contains_key(eng) {
            return 0.0;
        }
        if self.nr_updates < 2 {
            return 0.0;
        }

        let acum = &self.engs_acum;
        if acum.acum_time == 0 && acum.acum_cycles == 0 {
            return 0.0;
        }

        let ed = self.engs_delta.get(eng).unwrap();
        let cap = self.engs_last.get(eng).unwrap().capacity as f64;

        let mut res: f64 = 0.0;
        if acum.acum_cycles > 0 {
            res = (ed.delta_cycles as f64 * 100.0) /
                (ed.delta_total_cycles as f64 * cap);
        } else if acum.acum_time > 0 {
            res = ((ed.delta_time as f64 / 1000000.0) * 100.0) /
                (self.ms_elapsed as f64 * cap);
        }

        if res > 100.0 {
            warn!("Engine {:?} utilization at {:?}, clamped to 100%.",
                eng, res);
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
        if acum.acum_time > 0 || acum.acum_cycles > 0 {
            return true;
        }

        if self.total_mem() > 0 {
            return true;
        }

        false
    }

    pub fn update(&mut self, pinfo: ProcInfo, fdi: DrmFdinfo)
    {
        if self.proc != pinfo {
            self.proc = pinfo;  // fd might be shared
        }
        if let Err(err) = self.proc.update() {
            debug!("ERR: failed to update process info for {:?}: {:?}",
                self.proc, err);
        }
        self.fdinfo_path = fdi.path;

        self.engs_acum.acum_time = 0;
        self.engs_acum.acum_cycles = 0;
        self.engs_acum.acum_total_cycles = 0;

        for (nm, oeng) in self.engs_last.iter_mut() {
            let deng = self.engs_delta.get_mut(nm).unwrap();
            let neng = fdi.engines.get(nm).unwrap();

            if neng.time >= oeng.time {
                self.engs_acum.acum_time += neng.time;
                deng.delta_time = neng.time - oeng.time;
                oeng.time = neng.time;
            }

            if neng.cycles >= oeng.cycles {
                self.engs_acum.acum_cycles += neng.cycles;
                deng.delta_cycles = neng.cycles - oeng.cycles;
                oeng.cycles = neng.cycles;
            }

            if neng.total_cycles >= oeng.total_cycles {
                self.engs_acum.acum_total_cycles += neng.total_cycles;
                deng.delta_total_cycles = neng.total_cycles - oeng.total_cycles;
                oeng.total_cycles = neng.total_cycles;
            }
        }
        self.mem_regions = fdi.mem_regions;

        self.nr_updates += 1;
        self.ms_elapsed = self.last_update.elapsed().as_millis() as u64;
        self.last_update = time::Instant::now();
    }

    pub fn set_driver(&mut self, drv_wref: Weak<RefCell<dyn DrmDriver>>)
    {
        self.driver = Some(drv_wref);
    }

    pub fn from(pinfo: ProcInfo, fdi: DrmFdinfo) -> DrmClientInfo
    {
        let mut cli = DrmClientInfo {
            pci_dev: fdi.pci_dev.clone(),
            drm_minor: fdi.drm_minor,
            client_id: fdi.client_id,
            ..Default::default()
        };

        for nm in fdi.engines.keys() {
            cli.engs_last.insert(nm.clone(), DrmEngine::new(nm.as_str()));
            cli.engs_delta.insert(nm.clone(), DrmEngineDelta::new());
        }

        cli.update(pinfo, fdi);

        cli
    }
}

#[derive(Debug)]
pub struct DrmClients
{
    base_pid: String,
    infos: HashMap<String, Rc<RefCell<Vec<DrmClientInfo>>>>,
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

        for cliref in vlst.iter_mut() {
            cliref.set_driver(Weak::clone(&drv_wref));
        }
    }

    pub fn device_clients(&self,
        dev: &String) -> Option<Rc<RefCell<Vec<DrmClientInfo>>>>
    {
        if let Some(vref) = self.infos.get(dev) {
            return Some(vref.clone());
        }

        None
    }

    fn map_has_client<'a>(map: &'a mut HashMap<String,
        Rc<RefCell<Vec<DrmClientInfo>>>>, dev: &'a String,
        minor: u32, id: u32) -> Option<RefMut<'a, DrmClientInfo>>
    {
        if !map.contains_key(dev) {
            return None;
        }
        let vlst = map.get_mut(dev).unwrap().borrow_mut();

        let mut idx = 0;
        for cliref in vlst.iter() {
            if cliref.drm_minor == minor && cliref.client_id == id {
                break;
            }
            idx += 1;
        }

        if idx >= vlst.len() {
            return None;
        }

        Some(RefMut::map(vlst, |v| &mut v[idx]))
    }

    fn map_remove_client(map: &mut HashMap<String,
        Rc<RefCell<Vec<DrmClientInfo>>>>, dev: &String,
        minor: u32, id: u32) -> Option<DrmClientInfo>
    {
        if !map.contains_key(dev) {
            return None
        }
        let mut vlst = map.get(dev).unwrap().borrow_mut();

        let mut idx = 0;
        for cliref in vlst.iter() {
            if cliref.drm_minor == minor && cliref.client_id == id {
                break;
            }
            idx += 1;
        }

        if idx >= vlst.len() {
            return None;
        }

        Some(vlst.swap_remove(idx))
    }

    fn map_insert_client(map: &mut HashMap<String,
        Rc<RefCell<Vec<DrmClientInfo>>>>, dev: String, cli: DrmClientInfo)
    {
        if !map.contains_key(&dev) {
            let mut vlst: Vec<DrmClientInfo> = Vec::new();
            vlst.push(cli);
            map.insert(dev, Rc::new(RefCell::new(vlst)));
        } else {
            let mut vref = map.get(&dev).unwrap().borrow_mut();
            vref.push(cli);
        }
    }

    fn process_fdinfos(&mut self,
        ninfos: &mut HashMap<String, Rc<RefCell<Vec<DrmClientInfo>>>>,
        nproc: &ProcInfo, fdinfos: Vec<DrmFdinfo>)
    {
        for fdi in fdinfos {
            if let Some(mut cliref) = DrmClients::map_has_client(ninfos,
                &fdi.pci_dev, fdi.drm_minor, fdi.client_id) {
                cliref.shared_procs.push((nproc.clone(), fdi.path));
                debug!("INF: repeated drm client/fd info: proc={:?}, drm-minor={:?}, drm-client-id={:?}", nproc, fdi.drm_minor, fdi.client_id);
                continue;
            }

            let pci_dev = fdi.pci_dev.clone();
            if let Some(mut cli) = DrmClients::map_remove_client(
                &mut self.infos, &fdi.pci_dev, fdi.drm_minor, fdi.client_id) {
                cli.update(nproc.clone(), fdi);
                DrmClients::map_insert_client(ninfos, pci_dev, cli);
            } else {
                let cli = DrmClientInfo::from(nproc.clone(), fdi);
                DrmClients::map_insert_client(ninfos, pci_dev, cli);
            }
        }
    }

    fn scan_all_pids(&mut self) -> Result<()>
    {
        let mut ninfos: HashMap<String,
            Rc<RefCell<Vec<DrmClientInfo>>>> = HashMap::new();

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
                let fdinfos = nproc.get_drm_fdinfos();
                if let Err(err) = fdinfos {
                    debug!("ERR: failed to get DRM fdinfos from {:?}: {:?}",
                        nproc.pid, err);
                    continue;
                }
                let fdinfos = fdinfos.unwrap();

                // sort out DRM client infos based on DRM fdinfos
                self.process_fdinfos(&mut ninfos, &nproc, fdinfos);
            }
        }

        // update DRM client infos
        self.infos = ninfos;

        Ok(())
    }

    fn scan_pid_tree(&mut self) -> Result<()>
    {
        let mut ninfos: HashMap<String,
            Rc<RefCell<Vec<DrmClientInfo>>>> = HashMap::new();
        let mut pidq = VecDeque::from([self.base_pid.clone(),]);

        while !pidq.is_empty() {
            let npid = pidq.pop_front().unwrap();

            // new process info
            let nproc = ProcInfo::from(&npid);
            if let Err(err) = nproc {
                debug!("ERR: Couldn't get proc info for {:?}: {:?}", npid, err);
                continue;
            }
            let nproc = nproc.unwrap();

            // search and parse all DRM fdinfo from npid process
            let fdinfos = nproc.get_drm_fdinfos();
            if let Err(err) = fdinfos {
                debug!("ERR: failed to get DRM fdinfos from {:?}: {:?}",
                    npid, err);
                continue;
            }
            let fdinfos = fdinfos.unwrap();

            // sort out DRM client infos based on DRM fdinfos
            self.process_fdinfos(&mut ninfos, &nproc, fdinfos);

            // add all child processes
            let chids = nproc.get_children_procs();
            if let Err(err) = chids {
                debug!("ERR: failed to get children procs for {:?}: {:?}",
                    npid, err);
            } else {
                let mut chids = chids.unwrap();
                pidq.append(&mut chids);
            }
        }

        // update DRM client infos
        self.infos = ninfos;

        Ok(())
    }

    pub fn refresh(&mut self) -> Result<()>
    {
        if self.base_pid.is_empty() {
            self.scan_all_pids()?;
        } else {
            self.scan_pid_tree()?;
        }

        for vref in self.infos.values_mut() {
            let mut vcli = vref.borrow_mut();
            vcli.sort_by(|a, b| {
                if a.drm_minor == b.drm_minor {
                    a.client_id.cmp(&b.client_id)
                } else {
                    a.drm_minor.cmp(&b.drm_minor)
                }
            });
        }

        Ok(())
    }

    pub fn from_pid_tree(at_pid: &str) -> DrmClients
    {
        DrmClients {
            base_pid: at_pid.to_string(),
            infos: HashMap::new(),
        }
    }
}
