use std::collections::{VecDeque, HashMap};
use std::path::PathBuf;
use std::time;

use anyhow::Result;
use log::debug;

use crate::qmprocinfo::QmProcInfo;
use crate::qmdrmfdinfo::{QmDrmFdinfo, QmDrmEngine, QmDrmMemRegion};


#[derive(Debug)]
pub struct QmDrmEnginesAcum
{
    pub acum_time: u64,
    pub acum_cycles: u64,
    pub acum_total_cycles: u64,
}

impl QmDrmEnginesAcum
{
    pub fn new() -> QmDrmEnginesAcum
    {
        QmDrmEnginesAcum {
            acum_time: 0,
            acum_cycles: 0,
            acum_total_cycles: 0,
        }
    }
}

#[derive(Debug)]
pub struct QmDrmEngineDelta
{
    pub delta_time: u64,
    pub delta_cycles: u64,
    pub delta_total_cycles: u64,
}

impl QmDrmEngineDelta
{
    pub fn new() -> QmDrmEngineDelta
    {
        QmDrmEngineDelta {
            delta_time: 0,
            delta_cycles: 0,
            delta_total_cycles: 0,
        }
    }
}

#[derive(Debug)]
pub struct QmDrmClientInfo
{
    pub pci_dev: String,
    pub drm_minor: u32,
    pub client_id: u32,
    pub proc: QmProcInfo,
    pub fdinfo_path: PathBuf,
    pub shared_procs: Vec<(QmProcInfo, PathBuf)>,
    pub engs_last: HashMap<String, QmDrmEngine>,
    pub engs_delta: HashMap<String, QmDrmEngineDelta>,
    pub engs_acum: QmDrmEnginesAcum,
    pub mem_regions: HashMap<String, QmDrmMemRegion>,
    pub nr_updates: u64,
    pub last_update: time::Instant,
}

impl Default for QmDrmClientInfo
{
    fn default() -> QmDrmClientInfo
    {
        QmDrmClientInfo {
            pci_dev: String::from(""),
            drm_minor: 0,
            client_id: 0,
            proc: QmProcInfo {
                pid: 0,
                comm: String::from(""),
                proc_dir: PathBuf::new(),
            },
            fdinfo_path: PathBuf::new(),
            shared_procs: Vec::new(),
            engs_last: HashMap::new(),
            engs_delta: HashMap::new(),
            engs_acum: QmDrmEnginesAcum::new(),
            mem_regions: HashMap::new(),
            nr_updates: 0,
            last_update: time::Instant::now(),
        }
    }
}

impl QmDrmClientInfo
{
    pub fn total_mem(&self) -> u64
    {
        let mut tot: u64 = 0;
        for reg in self.mem_regions.values() {
            tot += reg.total;
        }

        tot
    }

    pub fn resident_mem(&self) -> u64
    {
        let mut tot: u64 = 0;
        for reg in self.mem_regions.values() {
            tot += reg.resident;
        }

        tot
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
            let ms_elapsed = self.last_update.elapsed().as_millis() as f64;
            res = ((ed.delta_time as f64 / 1000000.0) * 100.0) /
                (ms_elapsed * cap);
        }

        if res > 100.0 {
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

    pub fn update(&mut self, pinfo: QmProcInfo, fdi: QmDrmFdinfo)
    {
        self.proc = pinfo;  // fd might be shared
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
        self.last_update = time::Instant::now();
    }

    pub fn from(pinfo: QmProcInfo, fdi: QmDrmFdinfo) -> QmDrmClientInfo
    {
        let mut cli = QmDrmClientInfo {
            pci_dev: fdi.pci_dev.clone(),
            drm_minor: fdi.drm_minor,
            client_id: fdi.client_id,
            ..Default::default()
        };

        for nm in fdi.engines.keys() {
            cli.engs_last.insert(nm.clone(), QmDrmEngine::new(nm.as_str()));
            cli.engs_delta.insert(nm.clone(), QmDrmEngineDelta::new());
        }

        cli.update(pinfo, fdi);

        cli
    }
}

#[derive(Debug)]
pub struct QmDrmClients
{
    base_pid: String,
    infos: HashMap<String, Vec<QmDrmClientInfo>>,
}

impl QmDrmClients
{
    fn map_has_client<'a>(map: &'a mut HashMap<String, Vec<QmDrmClientInfo>>,
        dev: &'a String, minor: u32, id: u32) -> Option<&'a mut QmDrmClientInfo>
    {
        if !map.contains_key(dev) {
            return None;
        }

        let vlst = map.get_mut(dev).unwrap();
        for cliref in vlst {
            if cliref.drm_minor == minor && cliref.client_id == id {
                return Some(cliref);
            }
        }

        None
    }

    fn map_remove_client(map: &mut HashMap<String, Vec<QmDrmClientInfo>>,
        dev: &String, minor: u32, id: u32) -> Option<QmDrmClientInfo>
    {
        if !map.contains_key(dev) {
            return None
        }
        let vlst = map.get_mut(dev).unwrap();

        let mut idx = 0;
        for cliref in &mut *vlst {
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

    fn map_insert_client(map: &mut HashMap<String, Vec<QmDrmClientInfo>>,
        dev: String, cli: QmDrmClientInfo)
    {
        if !map.contains_key(&dev) {
            let mut vlst = Vec::<QmDrmClientInfo>::new();
            vlst.push(cli);
            map.insert(dev, vlst);
        } else {
            let vref = map.get_mut(&dev).unwrap();
            vref.push(cli);
        }
    }

    fn scan_pid_tree(&mut self) -> Result<()>
    {
        let mut ninfos: HashMap<String, Vec<QmDrmClientInfo>> = HashMap::new();
        let mut pidq = VecDeque::from([self.base_pid.clone(),]);

        while !pidq.is_empty() {
            let npid = pidq.pop_front().unwrap();

            // new process info
            let nproc = QmProcInfo::from(&npid);
            if let Err(err) = nproc {
                debug!("ERR: Couldn't get proc info for {:?}: {:?}", npid, err);
                continue;
            }
            let nproc = nproc.unwrap();

            // search and parse all DRM fdinfo from npid process
            let fdinfos = nproc.get_drm_fdinfos();
            if let Err(err) = fdinfos {
                debug!("ERR: failed to get DRM fdinfos from {:?}: {:?}", npid, err);
                continue;
            }
            let fdinfos = fdinfos.unwrap();

            // add all child processes
            let chids = nproc.get_children_procs();
            if let Err(err) = chids {
                debug!("ERR: failed to get children procs for {:?}: {:?}", npid, err);
            } else {
                let mut chids = chids.unwrap();
                pidq.append(&mut chids);
            }

            // sort out DRM client infos based on DRM fdinfos
            for fdi in fdinfos {
                if let Some(cliref) = QmDrmClients::map_has_client(&mut ninfos,
                    &fdi.pci_dev, fdi.drm_minor, fdi.client_id) {
                    cliref.shared_procs.push((nproc.clone(), fdi.path));
                    debug!("INF: repeated drm client/fd info: proc={:?}, drm-minor={:?}, drm-client-id={:?}", nproc, fdi.drm_minor, fdi.client_id);
                    continue;
                }

                let pci_dev = fdi.pci_dev.clone();
                if let Some(mut cli) = QmDrmClients::map_remove_client(
                    &mut self.infos, &fdi.pci_dev, fdi.drm_minor, fdi.client_id) {
                    cli.update(nproc.clone(), fdi);
                    QmDrmClients::map_insert_client(&mut ninfos, pci_dev, cli);
                } else {
                    let cli = QmDrmClientInfo::from(nproc.clone(), fdi);
                    QmDrmClients::map_insert_client(&mut ninfos, pci_dev, cli);
                }
            }
        }

        // update DRM client infos
        self.infos = ninfos;

        Ok(())
    }

    pub fn refresh(&mut self) -> Result<()>
    {
       self.scan_pid_tree()?;

       for vcli in self.infos.values_mut() {
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

    fn get_device_clients(&self,
        dev: &String, only_active: bool) -> Vec<&QmDrmClientInfo>
    {
        let mut res: Vec<&QmDrmClientInfo> = Vec::new();

        if let Some(infos) = self.infos.get(dev) {
            for cli in infos.iter() {
                if !only_active || cli.is_active() {
                    res.push(cli);
                }
            }
        }

        res
    }

    pub fn device_active_clients(&self, dev: &String) -> Vec<&QmDrmClientInfo>
    {
        self.get_device_clients(dev, true)
    }

    pub fn device_clients(&self, dev: &String) -> Vec<&QmDrmClientInfo>
    {
        self.get_device_clients(dev, false)
    }

    pub fn active_devices(&self) -> Vec<&String>
    {
        let mut res: Vec<&String> = self.infos.keys().collect::<Vec<&_>>();
        res.sort();

        res
    }

    pub fn infos(&self) -> &HashMap<String, Vec<QmDrmClientInfo>>
    {
        &self.infos
    }

    pub fn from_pid_tree(at_pid: &str) -> QmDrmClients
    {
        QmDrmClients {
            base_pid: at_pid.to_string(),
            infos: HashMap::new(),
        }
    }
}
