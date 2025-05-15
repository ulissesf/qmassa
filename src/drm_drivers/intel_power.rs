use core::fmt::Debug;
use std::collections::HashMap;
use std::path::Path;
use std::fs::{self, File};
use std::os::fd::{RawFd, AsRawFd};
use std::time;
use std::mem;
use std::io;

use anyhow::Result;
use libc;
use log::{debug, error};

use crate::perf_event::{
    perf_event_attr, PERF_SAMPLE_IDENTIFIER, PERF_FORMAT_GROUP, PerfEvent
};
use crate::hwmon::Hwmon;
use crate::drm_devices::DrmDevicePower;


pub trait GpuPowerIntel
{
    fn power_usage(&mut self, hwmon: &Option<Hwmon>) -> Result<DrmDevicePower>;
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
    pwr_func: Option<fn(&mut DGpuPowerIntel,
            hwmon: &Hwmon) -> Result<DrmDevicePower>>,
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
    fn power_usage(&mut self, hwmon: &Option<Hwmon>) -> Result<DrmDevicePower>
    {
        if self.pwr_func.is_none() {
            return Ok(DrmDevicePower::new());
        }
        let func = self.pwr_func.as_ref().unwrap();

        func(self, hwmon.as_ref().unwrap())
    }
}

impl DGpuPowerIntel
{
    fn read_from_power(&mut self, hwmon: &Hwmon) -> Result<DrmDevicePower>
    {
        let sens = &self.pwr_sensors;

        let mut gpu_pwr: f64 = 0.0;
        if !sens.gpu_sensor.is_empty() {
            gpu_pwr = hwmon.read_sensor(
                &sens.gpu_sensor, &sens.gpu_item)? as f64 / 1000000.0;
        }

        let mut pkg_pwr: f64 = 0.0;
        if !sens.pkg_sensor.is_empty() {
            pkg_pwr = hwmon.read_sensor(
                &sens.pkg_sensor, &sens.pkg_item)? as f64 / 1000000.0;
        }

        Ok(DrmDevicePower {
            gpu_cur_power: gpu_pwr,
            pkg_cur_power: pkg_pwr,
        })
    }

    fn read_from_energy(&mut self, hwmon: &Hwmon) -> Result<DrmDevicePower>
    {
        let sens = &self.pwr_sensors;

        let mut gpu_val: u64 = 0;
        if !sens.gpu_sensor.is_empty() {
            gpu_val = hwmon.read_sensor(&sens.gpu_sensor, &sens.gpu_item)?;
        }

        let mut pkg_val: u64 = 0;
        if !sens.pkg_sensor.is_empty() {
            pkg_val = hwmon.read_sensor(&sens.pkg_sensor, &sens.pkg_item)?;
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

    fn set_power_method(&mut self, hwmon: &Hwmon) -> bool
    {
        let mut gpu_sensor = "";
        let mut pkg_sensor = "";
        let mut gpu_item = "";
        let mut pkg_item = "";

        // try power*_input or power*_average
        let pwrlst = hwmon.sensors("power");
        for s in pwrlst.iter() {
            if s.has_item("input") || s.has_item("average") {
                if s.label == "pkg" || s.label.is_empty() {
                    gpu_sensor = &s.stype;
                    gpu_item = if s.has_item("input") {
                        "input" } else { "average" };
                } else if s.label == "card" {
                    pkg_sensor = &s.stype;
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
        let elst = hwmon.sensors("energy");
        for s in elst.iter() {
            if s.has_item("input") {
                if s.label == "pkg" || s.label.is_empty() {
                    gpu_sensor = &s.stype;
                    gpu_item = "input";
                } else if s.label == "card" {
                    pkg_sensor = &s.stype;
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

    pub fn from(hwmon: &Hwmon) -> Result<Option<Box<dyn GpuPowerIntel>>>
    {
        let mut pwr = DGpuPowerIntel {
            pwr_func: None,
            pwr_sensors: SensorSet::new(),
            last_gpu_val: 0,
            last_pkg_val: 0,
            delta_gpu_val: 0,
            delta_pkg_val: 0,
            nr_updates: 0,
            last_update: time::Instant::now(),
        };

        if !pwr.set_power_method(hwmon) {
            debug!("No method to get power via Hwmon, aborting.");
            return Ok(None);
        }

        return Ok(Some(Box::new(pwr)));
    }
}

// from kernel's msr-index.h
const MSR_RAPL_POWER_UNIT: i64 = 0x00000606;
const MSR_PKG_ENERGY_STATUS: i64 = 0x00000611;  // "energy-pkg"
const MSR_PP1_ENERGY_STATUS: i64 = 0x00000641;  // "energy-gpu"

#[derive(Debug)]
struct MsrSum
{
    sum: u64,
    last: u64,
}

#[derive(Debug)]
struct MsrIntel
{
    _dn_file: File,
    dn_fd: RawFd,
    sums: HashMap<i64, MsrSum>,
}

impl MsrIntel
{
    fn t_read(&self, offset: i64) -> Result<(isize, u64)>
    {
        let mut val: u64 = 0;
        let val_ptr: *mut u64 = &mut val;
        let val_vptr = val_ptr as *mut libc::c_void;
        let size = mem::size_of::<u64>();

        let ret = unsafe {
            libc::pread(self.dn_fd, val_vptr, size, offset) };
        if ret < 0 {
            return Err(io::Error::last_os_error().into());
        }

        Ok((ret, val))
    }

    fn read(&self, offset: i64) -> Result<u64>
    {
        let (ret, val) = self.t_read(offset)?;
        if ret as usize != mem::size_of::<u64>() {
            error!("Read wrong # of bytes {:?} (expected {:?}) from MSR {:?}.",
                offset, mem::size_of::<u64>(), ret);
        }

        Ok(val)
    }

    fn read_sum(&mut self, offset: i64) -> Result<u64>
    {
        if !self.sums.contains_key(&offset) {
            self.sums.insert(offset, MsrSum { sum: 0, last: 0, });
        }

        let val = self.read(offset)?;
        let msrsum = self.sums.get_mut(&offset).unwrap();

        let last_val = msrsum.last;
        msrsum.last = val & 0xffffffff;

        let delta_val = ((val << 32) - (last_val << 32)) >> 32;
        msrsum.sum += delta_val;

        Ok(msrsum.sum)
    }

    fn probe(&self, offset: i64) -> Result<bool>
    {
        let res = self.t_read(offset);
        if res.is_err() {
            return Ok(false);
        }

        let (ret, _) = res.unwrap();
        if ret as usize != mem::size_of::<u64>() {
            return Ok(false);
        }

        Ok(true)
    }

    fn from(cpu: i32) -> Result<MsrIntel>
    {
        let fname = format!("/dev/cpu/{}/msr", cpu);
        let file = File::open(fname)?;
        let fd = file.as_raw_fd();

        Ok(MsrIntel {
            _dn_file: file,
            dn_fd: fd,
            sums: HashMap::new(),
        })
    }

    fn is_capable() -> bool
    {
        if !Path::new("/dev/cpu/0/msr").exists() {
            debug!("INF: couldn't find MSR device node.");
            return false;
        }

        if unsafe { libc::geteuid() } != 0 {
            debug!("INF: non-root user, no MSR device node access.");
            return false;
        }

        true
    }
}

#[derive(Debug)]
pub struct IGpuPowerIntel
{
    pf_evt: Option<PerfEvent>,
    msr: Option<MsrIntel>,
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
    fn power_usage(&mut self, _ign: &Option<Hwmon>) -> Result<DrmDevicePower>
    {
        let gpu_val: u64;
        let pkg_val: u64;

        if let Some(pf_evt) = &self.pf_evt {
            let vals = pf_evt.read(3)?;  // reads #evts, gpu, pkg
            gpu_val = vals[1];
            pkg_val = vals[2];
        } else {
            let msr = self.msr.as_mut().unwrap();
            gpu_val = msr.read_sum(MSR_PP1_ENERGY_STATUS)?;
            pkg_val = msr.read_sum(MSR_PKG_ENERGY_STATUS)?;
        }
        self.nr_updates += 1;

        let delta_time = self.last_update.elapsed().as_secs_f64();
        self.last_update = time::Instant::now();

        if self.nr_updates >= 2 {
            self.delta_gpu_val = gpu_val - self.last_gpu_val;
            self.delta_pkg_val = pkg_val - self.last_pkg_val;
        }
        self.last_gpu_val = gpu_val;
        self.last_pkg_val = pkg_val;

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

        let mut pf_evt = PerfEvent::new("power");
        pf_evt.group_open(&pf_attr, -1, cpu, 0)?;

        pf_attr.config = pkg_cfg;
        pf_evt.group_open(&pf_attr, -1, cpu, 0)?;

        Ok(Some((pf_evt, gpu_scale, pkg_scale)))
    }

    fn new_rapl_msr() -> Result<Option<(MsrIntel, f64, f64)>>
    {
        if !MsrIntel::is_capable() {
            debug!("INF: not capable of reading rapl power from MSR.");
            return Ok(None);
        }

        let cpu = unsafe { libc::sched_getcpu() };
        let msr = MsrIntel::from(cpu)?;

        if !msr.probe(MSR_RAPL_POWER_UNIT)? ||
            !msr.probe(MSR_PKG_ENERGY_STATUS)? ||
            !msr.probe(MSR_PP1_ENERGY_STATUS)? {
            debug!("ERR: can't read power unit, pkg and gpu MSRs, aborting.");
            return Ok(None);
        }

        let pu = msr.read(MSR_RAPL_POWER_UNIT)?;
        let scale = 1.0 / (1 << ((pu >> 8) & 0x1F)) as f64;

        Ok(Some((msr, scale, scale)))
    }

    pub fn new() -> Result<Option<Box<dyn GpuPowerIntel>>>
    {
        let mut pf_evt: Option<PerfEvent> = None;
        let mut msr: Option<MsrIntel> = None;
        let gpu_scale: f64;
        let pkg_scale: f64;

        if let Some(tup_res) = IGpuPowerIntel::new_rapl_perf_event()? {
            let pf_evt_obj: PerfEvent;
            (pf_evt_obj, gpu_scale, pkg_scale) = tup_res;
            pf_evt = Some(pf_evt_obj);
        } else {
            // fallback to MSR, if possible
            let tup_res = IGpuPowerIntel::new_rapl_msr()?;
            if tup_res.is_none() {
                return Ok(None);
            }

            let msr_obj: MsrIntel;
            (msr_obj, gpu_scale, pkg_scale) = tup_res.unwrap();
            msr = Some(msr_obj);
        }

        Ok(Some(Box::new(IGpuPowerIntel {
            pf_evt,
            msr,
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
