use std::cell::{RefCell, Ref};
use std::rc::Rc;
use std::time;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::drm_devices::{
    DrmDeviceFreqLimits, DrmDeviceFreqs,
    DrmDeviceMemInfo, DrmDeviceInfo, DrmDevices};
use crate::drm_clients::{DrmClientMemInfo, DrmClientInfo};


const APP_DATA_MAX_NR_STATS: usize = 40;

fn limited_vec_push<T>(vlst: &mut Vec<T>, vitem: T)
{
    if vlst.len() == APP_DATA_MAX_NR_STATS {
        vlst.drain(..1);
    }
    vlst.push(vitem);
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AppDataEngineStats
{
    pub usage: Vec<f64>,
}

impl AppDataEngineStats
{
    fn new() -> AppDataEngineStats
    {
        AppDataEngineStats {
            usage: Vec::new(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AppDataDeviceStats
{
    pub freqs: Vec<DrmDeviceFreqs>,
    pub mem_info: Vec<DrmDeviceMemInfo>,
    pub eng_stats: Vec<AppDataEngineStats>,
}

impl AppDataDeviceStats
{
    fn update_stats(&mut self,
        eng_names: &Vec<String>, dinfo: &DrmDeviceInfo)
    {
        limited_vec_push(&mut self.freqs, dinfo.freqs.clone());
        limited_vec_push(&mut self.mem_info, dinfo.mem_info.clone());

        for (en, est) in eng_names.iter().zip(self.eng_stats.iter_mut()) {
            limited_vec_push(&mut est.usage, dinfo.eng_utilization(en));
        }
    }

    fn new(eng_names: &Vec<String>) -> AppDataDeviceStats
    {
        let mut estats: Vec<AppDataEngineStats> = Vec::new();
        for _ in 0..eng_names.len() {
            let n_est = AppDataEngineStats::new();
            estats.push(n_est);
        }

        AppDataDeviceStats {
            freqs: Vec::new(),
            mem_info: Vec::new(),
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
    pub cpu_usage: Vec<f64>,
    pub eng_stats: Vec<AppDataEngineStats>,
    pub mem_info: Vec<DrmClientMemInfo>,
    pub is_active: bool,
}

impl AppDataClientStats
{
    fn update_stats(&mut self,
        eng_names: &Vec<String>, cinfo: &DrmClientInfo)
    {
        limited_vec_push(&mut self.cpu_usage, cinfo.proc.cpu_utilization());

        for (en, est) in eng_names.iter().zip(self.eng_stats.iter_mut()) {
            limited_vec_push(&mut est.usage, cinfo.eng_utilization(en));
        }
        limited_vec_push(&mut self.mem_info, cinfo.mem_info());

        self.is_active = cinfo.is_active();
    }

    fn from(eng_names: &Vec<String>,
        cinfo: &DrmClientInfo) -> AppDataClientStats
    {
        let mut estats: Vec<AppDataEngineStats> = Vec::new();
        for _ in 0..eng_names.len() {
            let n_est = AppDataEngineStats::new();
            estats.push(n_est);
        }

        AppDataClientStats {
            drm_minor: cinfo.drm_minor,
            client_id: cinfo.client_id,
            pid: cinfo.proc.pid,
            comm: cinfo.proc.comm.clone(),
            cmdline: cinfo.proc.cmdline.clone(),
            cpu_usage: Vec::new(),
            eng_stats: estats,
            mem_info: Vec::new(),
            is_active: false,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AppDataDeviceState
{
    pub pci_dev: String,
    pub vdr_dev_rev: String,
    pub dev_type: String,
    pub drv_name: String,
    pub dev_nodes: String,
    pub eng_names: Vec<String>,
    pub freq_limits: DrmDeviceFreqLimits,
    pub dev_stats: AppDataDeviceStats,
    pub dev_stats_enabled: bool,
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

    fn update_stats(&mut self, dinfo: &DrmDeviceInfo,
        cinfos_b: &Option<Ref<'_, Vec<DrmClientInfo>>>)
    {
        self.eng_names = dinfo.engines();

        if self.dev_stats_enabled {
            self.dev_stats.update_stats(&self.eng_names, dinfo);
        }

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
            dev_type: dinfo.dev_type.to_string(),
            drv_name: dinfo.drv_name.clone(),
            dev_nodes: dnodes,
            eng_names: enames,
            freq_limits: dinfo.freq_limits.clone(),
            dev_stats: dstats,
            dev_stats_enabled: dinfo.dev_stats_enabled,
            clis_stats: Vec::new(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct AppDataState
{
    pub timestamps: Vec<u128>,
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
                timestamps: Vec::new(),
                devs_state: Vec::new(),
        }
    }
}

#[derive(Debug)]
pub struct AppData
{
    state: AppDataState,
    qmds: DrmDevices,
    start_time: time::Instant,
}

impl AppData
{
    pub fn timestamps(&self) -> &Vec<u128>
    {
        &self.state.timestamps
    }

    pub fn devices(&self) -> &Vec<AppDataDeviceState>
    {
        &self.state.devs_state
    }

    pub fn get_device(&self, dev: &String) -> Option<&AppDataDeviceState>
    {
        for ds in self.state.devs_state.iter() {
            if ds.pci_dev == *dev {
                return Some(ds);
            }
        }

        None
    }

    pub fn state(&self) -> &AppDataState
    {
        &self.state
    }

    pub fn refresh(&mut self) -> Result<()>
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

        Ok(())
    }

    pub fn from(qmds: DrmDevices) -> AppData
    {
        AppData {
            state: AppDataState::new(),
            qmds,
            start_time: time::Instant::now(),
        }
    }
}
