use std::cell::{RefCell, Ref};
use std::rc::Rc;
use std::time;

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::qmdrmdevices::{
    QmDrmDeviceFreqs, QmDrmDeviceMemInfo, QmDrmDeviceInfo, QmDrmDevices};
use crate::qmdrmclients::{QmDrmClientMemInfo, QmDrmClientInfo};


const QM_APP_DATA_MAX_NR_STATS: usize = 10;

fn limited_vec_push<T>(vlst: &mut Vec<T>, vitem: T)
{
    if vlst.len() == QM_APP_DATA_MAX_NR_STATS {
        vlst.drain(..1);
    }
    vlst.push(vitem);
}

#[derive(Debug, Serialize, Deserialize)]
pub struct QmAppDataDeviceStats
{
    pub freqs: Vec<QmDrmDeviceFreqs>,
    pub mem_info: Vec<QmDrmDeviceMemInfo>,
}

impl QmAppDataDeviceStats
{
    fn update_stats(&mut self, dinfo: &QmDrmDeviceInfo)
    {
        limited_vec_push(&mut self.freqs, dinfo.freqs.clone());
        limited_vec_push(&mut self.mem_info, dinfo.mem_info.clone());
    }

    fn new() -> QmAppDataDeviceStats
    {
        QmAppDataDeviceStats {
            freqs: Vec::new(),
            mem_info: Vec::new(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct QmAppDataClientEngineStats
{
    pub usage: Vec<f64>,
}

impl QmAppDataClientEngineStats
{
    fn new() -> QmAppDataClientEngineStats
    {
        QmAppDataClientEngineStats {
            usage: Vec::new(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct QmAppDataClientStats
{
    pub drm_minor: u32,
    pub client_id: u32,
    pub pid: u32,
    pub comm: String,
    pub cmdline: String,
    pub cpu_usage: Vec<f64>,
    pub eng_stats: Vec<QmAppDataClientEngineStats>,
    pub mem_info: Vec<QmDrmClientMemInfo>,
    pub is_active: bool,
}

impl QmAppDataClientStats
{
    fn update_stats(&mut self,
        eng_names: &Vec<String>, cinfo: &QmDrmClientInfo)
    {
        limited_vec_push(&mut self.cpu_usage, cinfo.proc.cpu_utilization());

        for (en, est) in eng_names.iter().zip(self.eng_stats.iter_mut()) {
            limited_vec_push(&mut est.usage, cinfo.eng_utilization(en));
        }
        limited_vec_push(&mut self.mem_info, cinfo.mem_info());

        self.is_active = cinfo.is_active();
    }

    fn from(eng_names: &Vec<String>,
        cinfo: &QmDrmClientInfo) -> QmAppDataClientStats
    {
        let mut estats: Vec<QmAppDataClientEngineStats> = Vec::new();
        for _ in 0..eng_names.len() {
            let n_est = QmAppDataClientEngineStats::new();
            estats.push(n_est);
        }

        QmAppDataClientStats {
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
pub struct QmAppDataDeviceState
{
    pub pci_dev: String,
    pub vdr_dev_rev: String,
    pub dev_type: String,
    pub drv_name: String,
    pub dev_nodes: String,
    pub eng_names: Vec<String>,
    pub dev_stats: QmAppDataDeviceStats,
    pub clis_stats: Vec<QmAppDataClientStats>,
}

impl QmAppDataDeviceState
{
    fn remove_client_stat(&mut self,
        minor: u32, id: u32) -> Option<QmAppDataClientStats>
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

    fn update_stats(&mut self, dinfo: &QmDrmDeviceInfo,
        cinfos_b: &Option<Ref<'_, Vec<QmDrmClientInfo>>>)
    {
        self.dev_stats.update_stats(dinfo);

        let mut ncstats: Vec<QmAppDataClientStats> = Vec::new();
        if let Some(clis_b) = cinfos_b {
            for cinf in clis_b.iter() {
                let mut ncli_st: QmAppDataClientStats;
                if let Some(cli_st) = self.remove_client_stat(
                    cinf.drm_minor, cinf.client_id) {
                    ncli_st = cli_st;
                } else {
                    ncli_st = QmAppDataClientStats::from(
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

    fn from(dinfo: &QmDrmDeviceInfo,
        cinfos_b: &Option<Ref<'_, Vec<QmDrmClientInfo>>>) -> QmAppDataDeviceState
    {
        let cn = QmAppDataDeviceState::card_from(
            &dinfo.drm_minors[0].devnode);
        let mut dnodes = String::from(cn);

        for idx in 1..dinfo.drm_minors.len() {
            let cn = QmAppDataDeviceState::card_from(
                &dinfo.drm_minors[idx].devnode);
            dnodes.push_str(", ");
            dnodes.push_str(cn);
        }

        let mut enames: Vec<String> = Vec::new();
        if let Some(clis_b) = cinfos_b {
            if clis_b.len() > 0 {
                for en in clis_b[0].engines() {
                    enames.push(en.clone());
                }
            }
        }

        QmAppDataDeviceState {
            pci_dev: dinfo.pci_dev.clone(),
            vdr_dev_rev: format!("{} {} (rev {})",
                dinfo.vendor, dinfo.device, dinfo.revision),
            dev_type: if dinfo.is_discrete() {
                String::from("Discrete") } else { String::from("Integrated") },
            drv_name: dinfo.drv_name.clone(),
            dev_nodes: dnodes,
            eng_names: enames,
            dev_stats: QmAppDataDeviceStats::new(),
            clis_stats: Vec::new(),
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct QmAppDataState
{
    pub timestamps: Vec<u128>,
    pub devs_state: Vec<QmAppDataDeviceState>,
}

impl QmAppDataState
{
    fn remove_device(&mut self, dev: &String) -> Option<QmAppDataDeviceState>
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

    fn new() -> QmAppDataState
    {
        QmAppDataState {
                timestamps: Vec::new(),
                devs_state: Vec::new(),
        }
    }
}

#[derive(Debug)]
pub struct QmAppData
{
    state: QmAppDataState,
    qmds: QmDrmDevices,
    start_time: time::Instant,
}

impl QmAppData
{
    pub fn timestamps(&self) -> &Vec<u128>
    {
        &self.state.timestamps
    }

    pub fn devices(&self) -> &Vec<QmAppDataDeviceState>
    {
        &self.state.devs_state
    }

    pub fn get_device(&self, dev: &String) -> Option<&QmAppDataDeviceState>
    {
        for ds in self.state.devs_state.iter() {
            if ds.pci_dev == *dev {
                return Some(ds);
            }
        }

        None
    }

    pub fn state(&self) -> &QmAppDataState
    {
        &self.state
    }

    pub fn refresh(&mut self) -> Result<()>
    {
        self.qmds.refresh()?;

        let mut nstate = QmAppDataState::new();
        for d in self.qmds.devices() {
            let dinfo = self.qmds.device_info(d).unwrap();

            let o_up_ref: Option<Rc<RefCell<Vec<QmDrmClientInfo>>>>;
            let mut cinfos_b: Option<Ref<'_, Vec<QmDrmClientInfo>>> = None;
            if let Some(cinfos_ref) = dinfo.clients() {
                o_up_ref = cinfos_ref.upgrade();
                if let Some(up_ref) = &o_up_ref {
                    cinfos_b = Some(up_ref.borrow());
                }
            }

            let mut ndst: QmAppDataDeviceState;
            if let Some(dst) = self.state.remove_device(&d) {
                ndst = dst;
            } else {
                ndst = QmAppDataDeviceState::from(dinfo, &cinfos_b);
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

    pub fn from(qmds: QmDrmDevices) -> QmAppData
    {
        QmAppData {
            state: QmAppDataState::new(),
            qmds: qmds,
            start_time: time::Instant::now(),
        }
    }
}
