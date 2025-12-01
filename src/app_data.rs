use core::fmt::Debug;
use std::collections::{HashMap, HashSet, VecDeque};
use std::cell::{RefCell, Ref};
use std::cmp::max;
use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::rc::Rc;
use std::time;

use anyhow::{bail, Result};
use log::{error, info};
use serde::{Deserialize, Serialize};
use serde_json;

use crate::CliArgs;
use crate::drm_devices::{
    DrmDeviceFreqLimits, DrmDeviceFreqs, DrmDevicePower,
    DrmDeviceMemInfo, DrmDeviceType, DrmDeviceTemperature, DrmDeviceFan,
    DrmDeviceInfo, DrmDevices};
use crate::drm_clients::{DrmClientMemInfo, DrmClientInfo};
use crate::proc_info::ProcInfo;


const APP_DATA_MAX_NR_STATS: usize = 40;

fn limited_vec_push<T>(vlst: &mut VecDeque<T>, vitem: T)
{
    if vlst.len() == APP_DATA_MAX_NR_STATS {
        vlst.pop_front();
    }
    vlst.push_back(vitem);
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AppDataDeviceStats
{
    pub freqs: VecDeque<Vec<DrmDeviceFreqs>>,
    pub power: VecDeque<DrmDevicePower>,
    pub mem_info: VecDeque<DrmDeviceMemInfo>,
    pub eng_usage: HashMap<String, VecDeque<f64>>,
    pub temps: VecDeque<Vec<DrmDeviceTemperature>>,
    pub fans: VecDeque<Vec<DrmDeviceFan>>,
}

impl AppDataDeviceStats
{
    fn update_stats(&mut self,
        eng_names: &Vec<String>, dinfo: &DrmDeviceInfo)
    {
        if dinfo.has_driver() {
            limited_vec_push(&mut self.freqs, dinfo.freqs.clone());
            limited_vec_push(&mut self.power, dinfo.power.clone());
            limited_vec_push(&mut self.mem_info, dinfo.mem_info.clone());
        }

        for en in eng_names.iter() {
            if !self.eng_usage.contains_key(en) {
                self.eng_usage.insert(en.clone(), VecDeque::new());
            }
            let mut est = self.eng_usage.get_mut(en).unwrap();
            limited_vec_push(&mut est, dinfo.eng_utilization(en));
        }

        if !dinfo.temps.is_empty() {
            limited_vec_push(&mut self.temps, dinfo.temps.clone());
        }
        if !dinfo.fans.is_empty() {
            limited_vec_push(&mut self.fans, dinfo.fans.clone());
        }
    }

    fn new(eng_names: &Vec<String>) -> AppDataDeviceStats
    {
        let mut estats = HashMap::new();
        for en in eng_names.iter() {
            let n_est = VecDeque::new();
            estats.insert(en.clone(), n_est);
        }

        AppDataDeviceStats {
            freqs: VecDeque::new(),
            power: VecDeque::new(),
            mem_info: VecDeque::new(),
            eng_usage: estats,
            temps: VecDeque::new(),
            fans: VecDeque::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppDataClientStats
{
    pub clients: Vec<(u32, u32)>,       // list of (drm_minor, client_id)
    pub pid: u32,
    pub comm: String,
    pub cmdline: String,
    pub cpu_usage: VecDeque<f64>,
    pub eng_usage: HashMap<String, VecDeque<f64>>,
    pub mem_info: VecDeque<DrmClientMemInfo>,
    pub is_active: bool,
}

impl AppDataClientStats
{
    pub fn drm_minor(&self) -> u32
    {
        self.clients[0].0
    }

    pub fn client_id(&self) -> u32
    {
        self.clients[0].1
    }

    pub fn client_key(&self) -> (u32, u32)
    {
        self.clients[0]
    }

    pub fn is_single_client(&self) -> bool
    {
        self.clients.len() == 1
    }

    fn acum_mem_info(&mut self, mi: &VecDeque<DrmClientMemInfo>)
    {
        let dst_len = self.mem_info.len();
        let src_len = mi.len();
        let mut cnt = max(dst_len, src_len);

        let mut new_vmi = VecDeque::new();
        while cnt > 0 {
            let mut nmi = DrmClientMemInfo::new();

            if dst_len >= cnt {
                nmi.acum(&self.mem_info[dst_len - cnt]);
            }
            if src_len >= cnt {
                nmi.acum(&mi[src_len - cnt]);
            }

            new_vmi.push_back(nmi);
            cnt -= 1;
        }

        self.mem_info = new_vmi;
    }

    fn acum_eng_usage(&mut self, eu: &HashMap<String, VecDeque<f64>>)
    {
        let dst_set: HashSet<String> =
            HashSet::from_iter(self.eng_usage.keys().cloned());
        let src_set: HashSet<String> = HashSet::from_iter(eu.keys().cloned());

        for new_en in src_set.difference(&dst_set) {
            self.eng_usage.insert(new_en.clone(), eu[new_en].clone());
        }

        for en in dst_set.intersection(&src_set) {
            let dst_len = self.eng_usage[en].len();
            let src_len = eu[en].len();
            let mut cnt = max(dst_len, src_len);

            let mut new_veu = VecDeque::new();
            while cnt > 0 {
                let mut neu = 0.0;

                if dst_len >= cnt {
                    neu += self.eng_usage[en][dst_len - cnt];
                }
                if src_len >= cnt {
                    neu += eu[en][src_len - cnt];
                }

                new_veu.push_back(neu);
                cnt -= 1;
            }

            self.eng_usage.remove(en);
            self.eng_usage.insert(en.clone(), new_veu);
        }
    }

    fn acum_stats(&mut self, cli: &AppDataClientStats)
    {
        if self.pid != cli.pid ||
            self.comm != cli.comm ||
            self.cmdline != cli.cmdline {
            error!("Trying to acum stats {:?} to {:?}: different processes!",
                cli, self);
            return;
        }
        if !cli.is_single_client() {
            error!("Source stats {:?} need to be single DRM client stats",
                cli);
            return;
        }

        self.clients.push(cli.client_key());
        self.is_active = self.is_active || cli.is_active;
        if cli.cpu_usage.len() > self.cpu_usage.len() {
            self.cpu_usage = cli.cpu_usage.clone();
        }
        self.acum_mem_info(&cli.mem_info);
        self.acum_eng_usage(&cli.eng_usage);
    }

    fn update_stats(&mut self,
        eng_names: &Vec<String>, cinfo: &DrmClientInfo)
    {
        limited_vec_push(&mut self.cpu_usage, cinfo.proc.cpu_utilization());

        for en in eng_names.iter() {
            if !self.eng_usage.contains_key(en) {
                self.eng_usage.insert(en.clone(), VecDeque::new());
            }
            let mut est = self.eng_usage.get_mut(en).unwrap();
            limited_vec_push(&mut est, cinfo.eng_utilization(en));
        }
        limited_vec_push(&mut self.mem_info, cinfo.mem_info());

        self.is_active = cinfo.is_active();
    }

    fn from(eng_names: &Vec<String>,
        cinfo: &DrmClientInfo) -> AppDataClientStats
    {
        let mut estats = HashMap::new();
        for en in eng_names.iter() {
            let n_est = VecDeque::new();
            estats.insert(en.clone(), n_est);
        }

        AppDataClientStats {
            clients: vec![(cinfo.drm_minor, cinfo.client_id)],
            pid: cinfo.proc.pid,
            comm: cinfo.proc.comm.clone(),
            cmdline: cinfo.proc.cmdline.clone(),
            cpu_usage: VecDeque::new(),
            eng_usage: estats,
            mem_info: VecDeque::new(),
            is_active: false,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AppDataDeviceState
{
    pub pci_dev: String,
    pub vdr_dev_rev: String,
    pub dev_type: DrmDeviceType,
    pub drv_name: String,
    pub dev_nodes: String,
    pub eng_names: Vec<String>,
    pub freq_limits: Vec<DrmDeviceFreqLimits>,
    pub dev_stats: AppDataDeviceStats,
    clis_stats: Vec<AppDataClientStats>,
}

impl AppDataDeviceState
{
    pub fn find_pid_client_stats(&self,
        pid: u32) -> Option<AppDataClientStats>
    {
        let mut res: Option<AppDataClientStats> = None;

        for cli in self.clis_stats.iter() {
            if cli.pid == pid {
                res = res.map_or_else(
                    || Some(cli.clone()),
                    |mut st| { st.acum_stats(cli); Some(st) });
            }
        }

        res
    }

    pub fn clients_stats_by_pid(&self) -> Vec<AppDataClientStats>
    {
        let mut st_by_pid: HashMap<u32, AppDataClientStats> = HashMap::new();
        for cli in self.clis_stats.iter() {
            st_by_pid.entry(cli.pid)
                .and_modify(|st| st.acum_stats(cli))
                .or_insert(cli.clone());
        }

        let mut pid_ord: Vec<_> = st_by_pid.keys().cloned().collect();
        pid_ord.sort();

        let mut res = Vec::new();
        for pid in pid_ord.into_iter() {
            let st = st_by_pid.remove(&pid).unwrap();
            res.push(st);
        }

        res
    }

    pub fn find_client_stats(&self,
        pid: u32, client_key: (u32, u32)) -> Option<&AppDataClientStats>
    {
        for cst in self.clis_stats.iter() {
            if cst.is_single_client() &&
                cst.pid == pid && cst.client_key() == client_key {
                return Some(cst);
            }
        }

        None
    }

    pub fn clients_stats(&self) -> Vec<&AppDataClientStats>
    {
        self.clis_stats.iter().collect()
    }

    pub fn has_clients_stats(&self) -> bool
    {
        !self.clis_stats.is_empty()
    }

    fn remove_client_stat(&mut self,
        minor: u32, id: u32) -> Option<AppDataClientStats>
    {
        let mut idx = 0;
        for cli_st in &self.clis_stats {
            if cli_st.client_key() == (minor, id) {
                break;
            }
            idx += 1;
        }

        if idx >= self.clis_stats.len() {
            return None;
        }

        Some(self.clis_stats.swap_remove(idx))
    }

    fn update_eng_names(&mut self, dinfo: &DrmDeviceInfo)
    {
        let mut tst: HashSet<&str> = HashSet::new();
        let nengs = dinfo.engines();

        for en in self.eng_names.iter() {
            tst.insert(en);
        }
        for en in nengs.iter() {
            tst.insert(en);
        }

        let mut neng_names = Vec::new();
        for en in tst.iter() {
            neng_names.push(en.to_string());
        }
        neng_names.sort();

        self.eng_names = neng_names;
    }

    fn update_stats(&mut self, dinfo: &DrmDeviceInfo,
        cinfos_b: &Option<Ref<'_, Vec<DrmClientInfo>>>)
    {
        self.update_eng_names(dinfo);

        self.dev_stats.update_stats(&self.eng_names, dinfo);

        let mut ncstats: Vec<AppDataClientStats> = Vec::new();
        if let Some(clis_b) = cinfos_b {
            for cinf in clis_b.iter() {
                let mut ncli_st: AppDataClientStats;
                if let Some(cli_st) = self.remove_client_stat(
                    cinf.drm_minor, cinf.client_id) {
                    ncli_st = cli_st;
                } else {
                    ncli_st = AppDataClientStats::from(
                        &self.eng_names, cinf);
                }

                ncli_st.update_stats(&self.eng_names, cinf);
                ncstats.push(ncli_st);
            }
        }

        self.clis_stats = ncstats;
    }

    fn card_from(devnode: &String) -> &str
    {
        if devnode.starts_with("/dev/dri/") {
            &devnode["/dev/dri/".len()..]
        } else {
            devnode
        }
    }

    fn from(dinfo: &DrmDeviceInfo) -> AppDataDeviceState
    {
        let cn = AppDataDeviceState::card_from(
            &dinfo.drm_minors[0].devnode);
        let mut dnodes = String::from(cn);

        for idx in 1..dinfo.drm_minors.len() {
            let cn = AppDataDeviceState::card_from(
                &dinfo.drm_minors[idx].devnode);
            dnodes.push_str(", ");
            dnodes.push_str(cn);
        }

        let enames = dinfo.engines();
        let dstats = AppDataDeviceStats::new(&enames);

        AppDataDeviceState {
            pci_dev: dinfo.pci_dev.clone(),
            vdr_dev_rev: if !dinfo.vendor.is_empty() {
                format!("{} {} (rev {})",
                    dinfo.vendor, dinfo.device, dinfo.revision)
            } else {
                dinfo.pci_dev.clone()
            },
            dev_type: dinfo.dev_type.clone(),
            drv_name: dinfo.drv_name.clone(),
            dev_nodes: dnodes,
            eng_names: enames,
            freq_limits: dinfo.freq_limits.clone(),
            dev_stats: dstats,
            clis_stats: Vec::new(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AppDataState
{
    pub timestamps: VecDeque<u128>,
    pub devs_state: Vec<AppDataDeviceState>,
}

impl AppDataState
{
    fn remove_device(&mut self, dev: &String) -> Option<AppDataDeviceState>
    {
        let mut idx = 0;
        for ds in &self.devs_state {
            if ds.pci_dev == *dev {
                break;
            }
            idx += 1;
        }

        if idx >= self.devs_state.len() {
            return None;
        }

        Some(self.devs_state.swap_remove(idx))
    }

    fn new() -> AppDataState
    {
        AppDataState {
                timestamps: VecDeque::new(),
                devs_state: Vec::new(),
        }
    }
}

pub trait AppData
{
    fn start_json_file(&mut self) -> Result<()>
    {
        Ok(())
    }

    fn update_json_file(&mut self) -> Result<()>
    {
        Ok(())
    }

    fn args(&self) -> &CliArgs;

    fn args_mut(&mut self) -> &mut CliArgs;

    fn timestamps(&self) -> &VecDeque<u128>;

    fn devices(&self) -> &Vec<AppDataDeviceState>;

    fn get_device(&self, dev: &String) -> Option<&AppDataDeviceState>;

    fn refresh(&mut self) -> Result<bool>;
}

impl Debug for dyn AppData
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "AppData()")
    }
}

#[derive(Debug)]
pub struct AppDataJson
{
    args: CliArgs,
    states: VecDeque<AppDataState>,
    freader: BufReader<File>,
}

impl AppData for AppDataJson
{
    fn args(&self) -> &CliArgs
    {
        &self.args
    }

    fn args_mut(&mut self) -> &mut CliArgs
    {
        &mut self.args
    }

    fn timestamps(&self) -> &VecDeque<u128>
    {
        let state = self.states.front().unwrap();

        &state.timestamps
    }

    fn devices(&self) -> &Vec<AppDataDeviceState>
    {
        let state = self.states.front().unwrap();

        &state.devs_state
    }

    fn get_device(&self, dev: &String) -> Option<&AppDataDeviceState>
    {
        let state = self.states.front().unwrap();

        for ds in state.devs_state.iter() {
            if ds.pci_dev == *dev {
                return Some(ds);
            }
        }

        None
    }

    fn refresh(&mut self) -> Result<bool>
    {
        let curr = self.next_state()?;
        if curr.is_none() {
            // End of JSON data
            return Ok(false);
        }
        let curr = curr.unwrap();

        self.states.pop_front();
        self.states.push_back(curr);

        Ok(true)
    }
}

impl AppDataJson
{
    pub fn states(&self) -> &VecDeque<AppDataState>
    {
        &self.states
    }

    pub fn is_empty(&self) -> bool
    {
        self.states.is_empty()
    }

    fn next_state(&mut self) -> Result<Option<AppDataState>>
    {
        // try to read and deserialize one more state from JSON
        let mut buf = String::new();
        if self.freader.read_line(&mut buf)? == 0 {
            // End of JSON file!
            return Ok(None);
        }

        let curr: AppDataState = serde_json::from_str(&buf)?;

        Ok(Some(curr))
    }

    pub fn load_states(&mut self) -> Result<()>
    {
        loop {
            let curr = self.next_state()?;
            if curr.is_none() {
                return Ok(());
            }
            let curr = curr.unwrap();

            self.states.push_back(curr);
        }
    }

    pub fn from(json_fname: &str) -> Result<AppDataJson>
    {
        let file = File::open(json_fname)?;
        let mut freader = BufReader::new(file);

        let mut buf = String::new();
        if freader.read_line(&mut buf)? == 0 {
            bail!("Invalid JSON {}: no version information", json_fname);
        }
        let version: String = serde_json::from_str(&buf)?;

        let wanted = env!("CARGO_PKG_VERSION");
        if version != wanted {
            bail!("Incompatible version in JSON {:?}: expected {}, got {}.",
                json_fname, wanted, version);
        }

        buf.clear();
        if freader.read_line(&mut buf)? == 0 {
            bail!("Invalid JSON {}: no args information", json_fname);
        }
        let args: CliArgs = serde_json::from_str(&buf)?;

        info!("Opened JSON {:?}: version {}, arguments {:?}",
            json_fname, &version, &args);

        Ok(AppDataJson {
            args,
            states: VecDeque::new(),
            freader,
        })
    }
}

#[derive(Debug)]
pub struct AppDataLive
{
    args: CliArgs,
    qmds: DrmDevices,
    state: AppDataState,
    start_time: time::Instant,
    json: Option<File>,
}

impl AppData for AppDataLive
{
    fn start_json_file(&mut self) -> Result<()>
    {
        if let Some(fname) = &self.args.to_json {
            // drop saving to JSON & no TUI options
            let mut args = self.args.clone();
            args.to_json = None;
            args.no_tui = false;

            // create file and write initial JSON
            let mut jf = File::create(fname)?;
            serde_json::to_writer(&mut jf, env!("CARGO_PKG_VERSION"))?;
            writeln!(jf)?;
            serde_json::to_writer(&mut jf, &args)?;
            writeln!(jf)?;

            self.json = Some(jf);
        }

        Ok(())
    }

    fn update_json_file(&mut self) -> Result<()>
    {
        if let Some(jf) = &mut self.json {
            serde_json::to_writer(&mut *jf, &self.state)?;
            writeln!(jf)?;
        }

        Ok(())
    }

    fn args(&self) -> &CliArgs
    {
        &self.args
    }

    fn args_mut(&mut self) -> &mut CliArgs
    {
        &mut self.args
    }

    fn timestamps(&self) -> &VecDeque<u128>
    {
        &self.state.timestamps
    }

    fn devices(&self) -> &Vec<AppDataDeviceState>
    {
        &self.state.devs_state
    }

    fn get_device(&self, dev: &String) -> Option<&AppDataDeviceState>
    {
        for ds in self.state.devs_state.iter() {
            if ds.pci_dev == *dev {
                return Some(ds);
            }
        }

        None
    }

    fn refresh(&mut self) -> Result<bool>
    {
        self.qmds.refresh()?;

        let mut nstate = AppDataState::new();
        for d in self.qmds.devices() {
            let dinfo = self.qmds.device_info(d).unwrap();

            let o_up_ref: Option<Rc<RefCell<Vec<DrmClientInfo>>>>;
            let mut cinfos_b: Option<Ref<'_, Vec<DrmClientInfo>>> = None;
            if let Some(cinfos_ref) = dinfo.clients() {
                o_up_ref = cinfos_ref.upgrade();
                if let Some(up_ref) = &o_up_ref {
                    cinfos_b = Some(up_ref.borrow());
                }
            }

            let mut ndst: AppDataDeviceState;
            if let Some(dst) = self.state.remove_device(&d) {
                ndst = dst;
            } else {
                ndst = AppDataDeviceState::from(dinfo);
            }

            ndst.update_stats(dinfo, &cinfos_b);
            nstate.devs_state.push(ndst);
        }

        nstate.timestamps.append(&mut self.state.timestamps);
        limited_vec_push(&mut nstate.timestamps,
            self.start_time.elapsed().as_millis());

        self.state = nstate;

        // if tracking a PID tree, stop when it's not longer running
        if let Some(pid) = &self.args.pid {
            Ok(ProcInfo::is_valid_pid(pid))
        } else {
            Ok(true)
        }
    }
}

impl AppDataLive
{
    pub fn from(args: CliArgs, qmds: DrmDevices) -> AppDataLive
    {
        AppDataLive {
            args,
            qmds,
            state: AppDataState::new(),
            start_time: time::Instant::now(),
            json: None,
        }
    }
}
