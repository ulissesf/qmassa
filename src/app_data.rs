use core::fmt::Debug;
use std::collections::{HashMap, HashSet, VecDeque};
use std::cell::{RefCell, Ref};
use std::fs::{self, File};
use std::io::{Write, Seek, SeekFrom};
use std::rc::Rc;
use std::time;

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json;

use crate::CliArgs;
use crate::drm_devices::{
    DrmDeviceFreqLimits, DrmDeviceFreqs, DrmDevicePower,
    DrmDeviceMemInfo, DrmDeviceType, DrmDeviceInfo, DrmDevices};
use crate::drm_clients::{DrmClientMemInfo, DrmClientInfo};


const APP_DATA_MAX_NR_STATS: usize = 40;

fn limited_vec_push<T>(vlst: &mut VecDeque<T>, vitem: T)
{
    if vlst.len() == APP_DATA_MAX_NR_STATS {
        vlst.pop_front();
    }
    vlst.push_back(vitem);
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AppDataEngineStats
{
    pub usage: VecDeque<f64>,
}

impl AppDataEngineStats
{
    fn new() -> AppDataEngineStats
    {
        AppDataEngineStats {
            usage: VecDeque::new(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AppDataDeviceStats
{
    pub freqs: VecDeque<Vec<DrmDeviceFreqs>>,
    pub power: VecDeque<DrmDevicePower>,
    pub mem_info: VecDeque<DrmDeviceMemInfo>,
    pub eng_stats: HashMap<String, AppDataEngineStats>,
}

impl AppDataDeviceStats
{
    fn update_stats(&mut self,
        eng_names: &Vec<String>, dinfo: &DrmDeviceInfo)
    {
        limited_vec_push(&mut self.freqs, dinfo.freqs.clone());
        limited_vec_push(&mut self.power, dinfo.power.clone());
        limited_vec_push(&mut self.mem_info, dinfo.mem_info.clone());

        for en in eng_names.iter() {
            if !self.eng_stats.contains_key(en) {
                self.eng_stats.insert(en.clone(), AppDataEngineStats::new());
            }
            let est = self.eng_stats.get_mut(en).unwrap();
            limited_vec_push(&mut est.usage, dinfo.eng_utilization(en));
        }
    }

    fn new(eng_names: &Vec<String>) -> AppDataDeviceStats
    {
        let mut estats = HashMap::new();
        for en in eng_names.iter() {
            let n_est = AppDataEngineStats::new();
            estats.insert(en.clone(), n_est);
        }

        AppDataDeviceStats {
            freqs: VecDeque::new(),
            power: VecDeque::new(),
            mem_info: VecDeque::new(),
            eng_stats: estats,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AppDataClientStats
{
    pub drm_minor: u32,
    pub client_id: u32,
    pub pid: u32,
    pub comm: String,
    pub cmdline: String,
    pub cpu_usage: VecDeque<f64>,
    pub eng_stats: HashMap<String, AppDataEngineStats>,
    pub mem_info: VecDeque<DrmClientMemInfo>,
    pub is_active: bool,
}

impl AppDataClientStats
{
    fn update_stats(&mut self,
        eng_names: &Vec<String>, cinfo: &DrmClientInfo)
    {
        limited_vec_push(&mut self.cpu_usage, cinfo.proc.cpu_utilization());

        for en in eng_names.iter() {
            if !self.eng_stats.contains_key(en) {
                self.eng_stats.insert(en.clone(), AppDataEngineStats::new());
            }
            let est = self.eng_stats.get_mut(en).unwrap();
            limited_vec_push(&mut est.usage, cinfo.eng_utilization(en));
        }
        limited_vec_push(&mut self.mem_info, cinfo.mem_info());

        self.is_active = cinfo.is_active();
    }

    fn from(eng_names: &Vec<String>,
        cinfo: &DrmClientInfo) -> AppDataClientStats
    {
        let mut estats = HashMap::new();
        for en in eng_names.iter() {
            let n_est = AppDataEngineStats::new();
            estats.insert(en.clone(), n_est);
        }

        AppDataClientStats {
            drm_minor: cinfo.drm_minor,
            client_id: cinfo.client_id,
            pid: cinfo.proc.pid,
            comm: cinfo.proc.comm.clone(),
            cmdline: cinfo.proc.cmdline.clone(),
            cpu_usage: VecDeque::new(),
            eng_stats: estats,
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
    pub clis_stats: Vec<AppDataClientStats>,
}

impl AppDataDeviceState
{
    fn remove_client_stat(&mut self,
        minor: u32, id: u32) -> Option<AppDataClientStats>
    {
        let mut idx = 0;
        for cli_st in &self.clis_stats {
            if cli_st.drm_minor == minor && cli_st.client_id == id {
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
            vdr_dev_rev: format!("{} {} (rev {})",
                dinfo.vendor, dinfo.device, dinfo.revision),
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

#[derive(Debug, Deserialize, Serialize)]
pub struct AppDataJson
{
    args: CliArgs,
    states: VecDeque<AppDataState>,
}

impl AppData for AppDataJson
{
    fn args(&self) -> &CliArgs
    {
        &self.args
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
        self.states.pop_front();
        if self.states.is_empty() {
            // End of JSON data!
            return Ok(false);
        }

        Ok(true)
    }
}

impl AppDataJson
{
    fn new(args: CliArgs) -> AppDataJson
    {
        AppDataJson {
            args,
            states: VecDeque::new(),
        }
    }

    pub fn from(json_fname: &str) -> Result<AppDataJson>
    {
        let json_str = fs::read_to_string(json_fname)?;
        let res: AppDataJson = serde_json::from_str(&json_str)?;

        Ok(res)
    }

    pub fn states(&self) -> &VecDeque<AppDataState>
    {
        &self.states
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
    is_json_initial: bool,
}

impl AppData for AppDataLive
{
    fn start_json_file(&mut self) -> Result<()>
    {
        if let Some(fname) = &self.args.to_json {
            // create JSON structure, drop saving to JSON & no TUI options
            let mut args = self.args.clone();
            args.to_json = None;
            args.no_tui = false;
            let jd = AppDataJson::new(args);

            // create file and write initial JSON
            let mut jf = File::create(fname)?;
            serde_json::to_writer_pretty(&mut jf, &jd)?;
            writeln!(jf)?;

            self.json = Some(jf);
            self.is_json_initial = true;
        }

        Ok(())
    }

    fn update_json_file(&mut self) -> Result<()>
    {
        if let Some(jf) = &mut self.json {
            // overwrite last 4 bytes ("]\n}\n") with new state
            jf.seek(SeekFrom::End(-4))?;
            if !self.is_json_initial {
                writeln!(jf, ",")?;
            }
            serde_json::to_writer_pretty(&mut *jf, &self.state)?;

            // make it a valid JSON again
            writeln!(jf, "]\n}}")?;

            self.is_json_initial = false;
        }

        Ok(())
    }

    fn args(&self) -> &CliArgs
    {
        &self.args
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

        Ok(true)
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
            is_json_initial: true,
        }
    }
}
