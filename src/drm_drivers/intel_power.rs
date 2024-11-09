use core::fmt::Debug;
use std::path::{Path, PathBuf};
use std::time;
use std::mem;
use std::fs;

use anyhow::Result;
use libc;
use log::debug;

use crate::perf_event::{
    perf_event_attr, PERF_SAMPLE_IDENTIFIER, PERF_FORMAT_GROUP, PerfEvent
};
use crate::hwmon::Hwmon;
use crate::drm_devices::DrmDevicePower;


pub trait GpuPowerIntel
{
    fn power_usage(&mut self) -> Result<DrmDevicePower>;
}

impl Debug for dyn GpuPowerIntel
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "GpuPowerIntel")
    }
}

#[derive(Debug)]
struct SensorSet
{
    gpu_sensor: String,
    pkg_sensor: String,
    gpu_item: String,
    pkg_item: String,
}

impl SensorSet
{
    fn new() -> SensorSet
    {
        SensorSet {
            gpu_sensor: String::new(),
            pkg_sensor: String::new(),
            gpu_item: String::new(),
            pkg_item: String::new(),
        }
    }
}

#[derive(Debug)]
pub struct DGpuPowerIntel
{
    hwmon: Hwmon,
    pwr_func: Option<fn(&mut DGpuPowerIntel) -> Result<DrmDevicePower>>,
    pwr_sensors: SensorSet,
    last_gpu_val: u64,
    last_pkg_val: u64,
    delta_gpu_val: u64,
    delta_pkg_val: u64,
    nr_updates: u64,
    last_update: time::Instant,
}

impl GpuPowerIntel for DGpuPowerIntel
{
    fn power_usage(&mut self) -> Result<DrmDevicePower>
    {
        if self.pwr_func.is_none() {
            return Ok(DrmDevicePower::new());
        }
        let func = self.pwr_func.as_ref().unwrap();

        func(self)
    }
}

impl DGpuPowerIntel
{
    fn read_from_power(&mut self) -> Result<DrmDevicePower>
    {
        let sens = &self.pwr_sensors;

        let mut gpu_pwr: f64 = 0.0;
        if !sens.gpu_sensor.is_empty() {
            gpu_pwr = self.hwmon.read_sensor(
                &sens.gpu_sensor, &sens.gpu_item)? as f64 / 1000000.0;
        }

        let mut pkg_pwr: f64 = 0.0;
        if !sens.pkg_sensor.is_empty() {
            pkg_pwr = self.hwmon.read_sensor(
                &sens.pkg_sensor, &sens.pkg_item)? as f64 / 1000000.0;
        }

        Ok(DrmDevicePower {
            gpu_cur_power: gpu_pwr,
            pkg_cur_power: pkg_pwr,
        })
    }

    fn read_from_energy(&mut self) -> Result<DrmDevicePower>
    {
        let sens = &self.pwr_sensors;

        let mut gpu_val: u64 = 0;
        if !sens.gpu_sensor.is_empty() {
            gpu_val = self.hwmon.read_sensor(
                &sens.gpu_sensor, &sens.gpu_item)?;
        }

        let mut pkg_val: u64 = 0;
        if !sens.pkg_sensor.is_empty() {
            pkg_val = self.hwmon.read_sensor(
                &sens.pkg_sensor, &sens.pkg_item)?;
        }

        self.nr_updates += 1;
        let delta_time = self.last_update.elapsed().as_secs_f64();
        self.last_update = time::Instant::now();

        if self.nr_updates >= 2 {
            if gpu_val > 0 {
                self.delta_gpu_val = gpu_val - self.last_gpu_val;
            }
            if pkg_val > 0 {
                self.delta_pkg_val = pkg_val - self.last_pkg_val;
            }
        }
        self.last_gpu_val = gpu_val;
        self.last_pkg_val = pkg_val;

        let gpu_pwr = (self.delta_gpu_val as f64 / 1000000.0) / delta_time;
        let pkg_pwr = (self.delta_pkg_val as f64 / 1000000.0) / delta_time;

        Ok(DrmDevicePower {
            gpu_cur_power: gpu_pwr,
            pkg_cur_power: pkg_pwr,
        })
    }

    fn set_power_method(&mut self) -> bool
    {
        let mut gpu_sensor = "";
        let mut pkg_sensor = "";
        let mut gpu_item = "";
        let mut pkg_item = "";

        // try power*_input or power*_average
        let pwrlst = self.hwmon.sensors("power");
        for s in pwrlst.iter() {
            if s.has_item("input") || s.has_item("average") {
                if s.label == "card" {
                    gpu_sensor = &s.sensor;
                    gpu_item = if s.has_item("input") {
                        "input" } else { "average" };
                } else if s.label == "pkg" || s.label.is_empty() {
                    pkg_sensor = &s.sensor;
                    pkg_item = if s.has_item("input") {
                        "input" } else { "average" };
                }
            }
        }

        if !gpu_sensor.is_empty() || !pkg_sensor.is_empty() {
            self.pwr_func = Some(DGpuPowerIntel::read_from_power);
            self.pwr_sensors.gpu_sensor = gpu_sensor.to_string();
            self.pwr_sensors.pkg_sensor = pkg_sensor.to_string();
            self.pwr_sensors.gpu_item = gpu_item.to_string();
            self.pwr_sensors.pkg_item = pkg_item.to_string();

            return true;
        }

        // try energy*_input
        let elst = self.hwmon.sensors("energy");
        for s in elst.iter() {
            if s.has_item("input") {
                if s.label == "card" {
                    gpu_sensor = &s.sensor;
                    gpu_item = "input";
                } else if s.label == "pkg" || s.label.is_empty() {
                    pkg_sensor = &s.sensor;
                    pkg_item = "input";
                }
            }
        }

        if !gpu_sensor.is_empty() || !pkg_sensor.is_empty() {
            self.pwr_func = Some(DGpuPowerIntel::read_from_energy);
            self.pwr_sensors.gpu_sensor = gpu_sensor.to_string();
            self.pwr_sensors.pkg_sensor = pkg_sensor.to_string();
            self.pwr_sensors.gpu_item = gpu_item.to_string();
            self.pwr_sensors.pkg_item = pkg_item.to_string();

            return true;
        }

        false
    }

    pub fn from(dev_dir: &PathBuf) -> Result<Option<Box<dyn GpuPowerIntel>>>
    {
        let base_dir = dev_dir.join("hwmon");
        let hwmon_path = fs::read_dir(base_dir)?
            .into_iter()
            .filter(|r| r.is_ok())
            .map(|r| r.unwrap().path())
            .find(|r| r.file_name().unwrap()
                .to_str().unwrap().starts_with("hwmon"));
        if hwmon_path.is_none() {
            debug!("INF: no {:?}/hwmon* directory, aborting.", dev_dir);
            return Ok(None);
        }

        let hwmon = Hwmon::from(hwmon_path.unwrap().to_path_buf())?;
        if hwmon.is_none() {
            debug!("INF: no Hwmon support, no dGPU power reporting.");
            return Ok(None);
        }

        let mut pwr = DGpuPowerIntel {
            hwmon: hwmon.unwrap(),
            pwr_func: None,
            pwr_sensors: SensorSet::new(),
            last_gpu_val: 0,
            last_pkg_val: 0,
            delta_gpu_val: 0,
            delta_pkg_val: 0,
            nr_updates: 0,
            last_update: time::Instant::now(),
        };

        if !pwr.set_power_method() {
            debug!("No method to get power via Hwmon, aborting.");
            return Ok(None);
        }

        return Ok(Some(Box::new(pwr)));
    }
}

#[derive(Debug)]
pub struct IGpuPowerIntel
{
    pf_evt: PerfEvent,
    last_gpu_val: u64,
    last_pkg_val: u64,
    delta_gpu_val: u64,
    delta_pkg_val: u64,
    gpu_scale: f64,
    pkg_scale: f64,
    nr_updates: u64,
    last_update: time::Instant,
}

impl GpuPowerIntel for IGpuPowerIntel
{
    fn power_usage(&mut self) -> Result<DrmDevicePower>
    {
        let vals = self.pf_evt.read(3)?;  // reads #evts, gpu, pkg
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
}

impl IGpuPowerIntel
{
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

        if !evt_dir.join("energy-gpu").exists() ||
            !evt_dir.join("energy-pkg").exists() {
            debug!("Missing either energy-gpu or energy-pkg, aborting.");
            return Ok(None);
        }

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

        let cfg = IGpuPowerIntel::get_perf_config(&evt_dir, "energy-gpu")?;
        if cfg.is_none() {
            return Ok(None);
        }
        let gpu_cfg = cfg.unwrap();

        let cfg = IGpuPowerIntel::get_perf_config(&evt_dir, "energy-pkg")?;
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

    pub fn new() -> Result<Option<Box<dyn GpuPowerIntel>>>
    {
        let tup_res = IGpuPowerIntel::new_rapl_perf_event()?;
        if tup_res.is_none() {
            return Ok(None);
        }
        let (pf_evt, gpu_scale, pkg_scale) = tup_res.unwrap();

        Ok(Some(Box::new(IGpuPowerIntel {
            pf_evt,
            last_gpu_val: 0,
            last_pkg_val: 0,
            delta_gpu_val: 0,
            delta_pkg_val: 0,
            gpu_scale,
            pkg_scale,
            nr_updates: 0,
            last_update: time::Instant::now(),
        })))
    }
}
