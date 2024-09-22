use std::collections::{VecDeque, HashMap};
use std::path::PathBuf;

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
}

impl Default for QmDrmClientInfo
{
    fn default() -> QmDrmClientInfo
    {
        QmDrmClientInfo {
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
        }
    }
}

impl QmDrmClientInfo
{
    pub fn total_mem(&self) -> u64
    {
        let mut tot:u64 = 0;
        for reg in self.mem_regions.values() {
            tot += reg.total;
        }

        tot
    }

    pub fn resident_mem(&self) -> u64
    {
        let mut tot:u64 = 0;
        for reg in self.mem_regions.values() {
            tot += reg.resident;
        }

        tot
    }

    pub fn eng_utilization(&self, eng: &String, interval: u64) -> f64
    {
        if !self.engs_last.contains_key(eng) {
            return 0.0;
        }
        if self.nr_updates < 2 {
            return 0.0;
        }

        let acum = &self.engs_acum;
        if acum.acum_time == 0 && acum.acum_total_cycles == 0 {
            return 0.0;
        }

        let ed = self.engs_delta.get(eng).unwrap();
        let cap = self.engs_last.get(eng).unwrap().capacity as f64;

        let mut res: f64 = 0.0;
        if acum.acum_total_cycles > 0 {
            res = (ed.delta_cycles as f64 * 100.0) /
                (ed.delta_total_cycles as f64 * cap);
        } else if acum.acum_time > 0 {
            res = ((ed.delta_time as f64 / 1000000.0) * 100.0) /
                (interval as f64 * cap);
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
        if acum.acum_time > 0 ||
            acum.acum_cycles > 0 ||
            acum.acum_total_cycles > 0 {
            return true;
        }

        for mr in self.mem_regions.values() {
            if mr.total > 0 ||
                mr.shared > 0 ||
                mr.resident > 0 ||
                mr.purgeable > 0 ||
                mr.active > 0 {
                return true;
            }
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
    }

    pub fn from(pinfo: QmProcInfo, fdi: QmDrmFdinfo) -> QmDrmClientInfo
    {
        let mut cli = QmDrmClientInfo {
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
    pub base_pid: String,
    infos: HashMap<u32, Vec<QmDrmClientInfo>>,
}

impl QmDrmClients
{
    fn map_has_client(map: &mut HashMap<u32, Vec<QmDrmClientInfo>>, minor: u32, id: u32) -> Option<&mut QmDrmClientInfo>
    {
        if !map.contains_key(&minor) {
            return None;
        }

        let vlst = map.get_mut(&minor).unwrap();
        for cliref in vlst {
            if cliref.client_id == id {
                return Some(cliref);
            }
        }

        None
    }

    fn map_remove_client(map: &mut HashMap<u32, Vec<QmDrmClientInfo>>, minor: u32, id: u32) -> Option<QmDrmClientInfo>
    {
        if !map.contains_key(&minor) {
            return None
        }
        let vlst = map.get_mut(&minor).unwrap();

        let mut idx = 0;
        for cliref in &mut *vlst {
            if cliref.client_id == id {
                break;
            }
            idx += 1;
        }

        if idx >= vlst.len() {
            return None;
        }

        Some(vlst.swap_remove(idx))
    }

    fn map_insert_client(map: &mut HashMap<u32, Vec<QmDrmClientInfo>>, minor: u32, cli: QmDrmClientInfo)
    {
        if !map.contains_key(&minor) {
            let mut vlst = Vec::<QmDrmClientInfo>::new();
            vlst.push(cli);
            map.insert(minor, vlst);
        } else {
            let vref = map.get_mut(&minor).unwrap();
            vref.push(cli);
        }
    }

    fn scan_pid_tree(&mut self) -> Result<()>
    {
        let mut ninfos: HashMap<u32, Vec<QmDrmClientInfo>> = HashMap::new();
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
                if let Some(cliref) = QmDrmClients::map_has_client(&mut ninfos, fdi.drm_minor, fdi.client_id) {
                    cliref.shared_procs.push((nproc.clone(), fdi.path));
                    debug!("INF: repeated drm client/fd info: proc={:?}, drm-minor={:?}, drm-client-id={:?}", nproc, fdi.drm_minor, fdi.client_id);
                    continue;
                }

                if let Some(mut cli) = QmDrmClients::map_remove_client(&mut self.infos, fdi.drm_minor, fdi.client_id) {
                    cli.update(nproc.clone(), fdi);
                    QmDrmClients::map_insert_client(&mut ninfos, cli.drm_minor, cli);
                } else {
                    let cli = QmDrmClientInfo::from(nproc.clone(), fdi);
                    QmDrmClients::map_insert_client(&mut ninfos, cli.drm_minor, cli);
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

       for cli in self.infos.values_mut() {
           cli.sort_by(|a, b| a.client_id.cmp(&b.client_id));
       }

       Ok(())
    }

    pub fn device_active_clients(&self, dev: &u32) -> Vec<&QmDrmClientInfo>
    {
        let mut res: Vec<&QmDrmClientInfo> = Vec::new();

        if let Some(infos) = self.infos.get(dev) {
            for cli in infos.iter() {
                if cli.is_active() {
                    res.push(cli);
                }
            }
        }

        res
    }

    pub fn device_clients(&self, dev: &u32) -> Option<&Vec<QmDrmClientInfo>>
    {
        self.infos.get(dev)
    }

    pub fn devices(&self) -> Vec<&u32>
    {
        let mut res: Vec<&u32> = self.infos.keys().collect::<Vec<&_>>();
        res.sort();

        res
    }

    pub fn infos(&self) -> &HashMap<u32, Vec<QmDrmClientInfo>>
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

    // TODO: add from_user_processes() and scan_user_processes
}
