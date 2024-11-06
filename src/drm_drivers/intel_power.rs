use std::path::Path;
use std::time;
use std::mem;
use std::fs;

use anyhow::Result;
use libc;
use log::debug;

use crate::perf_event::{
    perf_event_attr, PERF_SAMPLE_IDENTIFIER, PERF_FORMAT_GROUP, PerfEvent
};
use crate::drm_devices::{DrmDeviceType, DrmDevicePower};


#[derive(Debug)]
pub struct GpuPowerIntel
{
    dev_type: DrmDeviceType,
    pf_evt: Option<PerfEvent>,
    last_gpu_val: u64,
    last_pkg_val: u64,
    delta_gpu_val: u64,
    delta_pkg_val: u64,
    gpu_scale: f64,
    pkg_scale: f64,
    nr_updates: u64,
    last_update: time::Instant,
}

impl GpuPowerIntel
{
    pub fn power_usage(&mut self) -> Result<DrmDevicePower>
    {
        if self.dev_type.is_discrete() {
            return Ok(DrmDevicePower::new());
        }

        let pf_evt = self.pf_evt.as_mut().unwrap();

        let vals = pf_evt.read(3)?;  // reads  #evts, gpu, pkg
        self.nr_updates += 1;

        let delta_time = self.last_update.elapsed().as_secs_f64();
        self.last_update = time::Instant::now();

        if self.nr_updates >= 2 {
            self.delta_gpu_val = vals[1] - self.last_gpu_val;
            self.delta_pkg_val = vals[2] - self.last_pkg_val;
        }
        self.last_gpu_val = vals[1];
        self.last_pkg_val = vals[2];

        let gpu_pwr = (self.delta_gpu_val as f64 * self.gpu_scale) /
            delta_time;
        let pkg_pwr = (self.delta_pkg_val as f64 * self.pkg_scale) /
            delta_time;

        Ok(DrmDevicePower {
            gpu_cur_power: gpu_pwr,
            pkg_cur_power: pkg_pwr,
        })
    }

    fn get_perf_config(evt_dir: &Path, name: &str) -> Result<Option<u64>>
    {
        let raw = fs::read_to_string(evt_dir.join(name))?;
        let cfg_str = raw.trim();

        let cfg: Vec<_> = cfg_str.split(',').map(|it| it.trim()).collect();
        let mut config: Option<u64> = None;
        let mut umask: u64 = 0;

        for c in cfg.iter() {
            let kv: Vec<_> = c.split('=').map(|it| it.trim()).collect();
            if kv[0].starts_with("event") {
                config = Some(u64::from_str_radix(
                        kv[1].trim_start_matches("0x"), 16)?);
            } else if kv[0].starts_with("umask") {
                umask = kv[1].parse()?;
            } else {
                debug!("ERR: unknwon key {:?} in {:?} perf config file, aborting.",
                    kv[0], name);
                return Ok(None);
            }
        }
        if config.is_none() {
            debug!("No valid config info in {:?} perf config file, aborting.",
                name);
            return Ok(None);
        }

        let config = (umask << 8) | config.unwrap();

        Ok(Some(config))
    }

    fn new_rapl_perf_event() -> Result<Option<(PerfEvent, f64, f64)>>
    {
        if !PerfEvent::is_capable() {
            debug!("INF: no perf event support, no rapl power reporting.");
            return Ok(None);
        }

        let evt_dir = Path::new("/sys/devices/power/events");

        let gpu_unit = fs::read_to_string(evt_dir.join("energy-gpu.unit"))?;
        let pkg_unit = fs::read_to_string(evt_dir.join("energy-pkg.unit"))?;
        if gpu_unit.trim() != "Joules" || pkg_unit.trim() != "Joules" {
            debug!("ERR: gpu [{:?}] and pkg [{:?}] units need to be Joules, aborting.",
                gpu_unit, pkg_unit);
            return Ok(None);
        }

        let gpu_scale: f64 = fs::read_to_string(
            evt_dir.join("energy-gpu.scale"))?.trim().parse()?;
        let pkg_scale: f64 = fs::read_to_string(
            evt_dir.join("energy-pkg.scale"))?.trim().parse()?;
        if gpu_scale == 0.0 || pkg_scale == 0.0 {
            debug!("ERR: gpu [{:?}] and pkg [{:?}] scales need to be > 0.0, aborting.",
                gpu_scale, pkg_scale);
            return Ok(None);
        }

        let type_: u32 = fs::read_to_string(
            Path::new("/sys/devices/power/type"))?.trim().parse()?;
        let cpu: i32 = unsafe { libc::sched_getcpu() };

        let cfg = GpuPowerIntel::get_perf_config(&evt_dir, "energy-gpu")?;
        if cfg.is_none() {
            return Ok(None);
        }
        let gpu_cfg = cfg.unwrap();

        let cfg = GpuPowerIntel::get_perf_config(&evt_dir, "energy-pkg")?;
        if cfg.is_none() {
            return Ok(None);
        }
        let pkg_cfg = cfg.unwrap();

        let mut pf_attr = perf_event_attr::new();
        pf_attr.type_ = type_;
        pf_attr.size = mem::size_of::<perf_event_attr>() as u32;
        pf_attr.config = gpu_cfg;
        pf_attr.sample_type = PERF_SAMPLE_IDENTIFIER;
        pf_attr.read_format = PERF_FORMAT_GROUP;

        let mut pf_evt = PerfEvent::open(&pf_attr, -1, cpu, 0)?;

        pf_attr.config = pkg_cfg;
        pf_evt.group_open(&pf_attr, -1, cpu, 0)?;

        Ok(Some((pf_evt, gpu_scale, pkg_scale)))
    }

    pub fn from(dtype: DrmDeviceType) -> Result<Option<GpuPowerIntel>>
    {
        if dtype.is_discrete() {
            // TODO: implement hwmon to expose power for dgpu
            return Ok(None);
        }

        // dev_type is integrated
        let tup_res = GpuPowerIntel::new_rapl_perf_event()?;
        if tup_res.is_none() {
            return Ok(None);
        }
        let (pf_evt, gpu_scale, pkg_scale) = tup_res.unwrap();

        Ok(Some(GpuPowerIntel {
            dev_type: dtype,
            pf_evt: Some(pf_evt),
            last_gpu_val: 0,
            last_pkg_val: 0,
            delta_gpu_val: 0,
            delta_pkg_val: 0,
            gpu_scale,
            pkg_scale,
            nr_updates: 0,
            last_update: time::Instant::now(),
        }))
    }
}
