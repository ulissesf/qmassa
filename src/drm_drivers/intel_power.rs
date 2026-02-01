use core::fmt::Debug;
use std::collections::HashMap;
use std::path::Path;
use std::fs::File;
use std::os::fd::{RawFd, AsRawFd};
use std::time;
use std::mem;
use std::io;

use anyhow::{bail, Result};
use libc;
use log::debug;

use crate::perf_event::{
    perf_event_attr, PERF_SAMPLE_IDENTIFIER, PERF_FORMAT_GROUP, PerfEvent
};
use crate::hwmon::Hwmon;
use crate::drm_devices::DrmDevicePower;


pub trait GpuPowerIntel
{
    fn name(&self) -> &str;

    fn power_usage(&mut self, hwmon: &Option<Hwmon>) -> Result<DrmDevicePower>;
}

impl Debug for dyn GpuPowerIntel
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "GpuPowerIntel({:?})", self.name())
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

#[derive(Debug)]
pub struct DGpuPowerIntel
{
    pwr_func: fn(&mut DGpuPowerIntel, hwmon: &Hwmon) -> Result<DrmDevicePower>,
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
    fn name(&self) -> &str
    {
        "dGPU:hwmon"
    }

    fn power_usage(&mut self, hwmon: &Option<Hwmon>) -> Result<DrmDevicePower>
    {
        (self.pwr_func)(self, hwmon.as_ref().unwrap())
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

    fn find_power_method(
        hwmon: &Hwmon
    ) -> Option<(
        fn(&mut DGpuPowerIntel, hwmon: &Hwmon) -> Result<DrmDevicePower>,
        SensorSet
    )>
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
            let ss = SensorSet {
                gpu_sensor: gpu_sensor.to_string(),
                pkg_sensor: pkg_sensor.to_string(),
                gpu_item: gpu_item.to_string(),
                pkg_item: pkg_item.to_string(),
            };

            return Some((DGpuPowerIntel::read_from_power, ss));
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
            let ss = SensorSet {
                gpu_sensor: gpu_sensor.to_string(),
                pkg_sensor: pkg_sensor.to_string(),
                gpu_item: gpu_item.to_string(),
                pkg_item: pkg_item.to_string(),
            };

            return Some((DGpuPowerIntel::read_from_energy, ss));
        }

        None
    }

    pub fn from(hwmon: &Hwmon) -> Result<Option<Box<dyn GpuPowerIntel>>>
    {
        let res = DGpuPowerIntel::find_power_method(hwmon);
        if res.is_none() {
            debug!("No method to get power via Hwmon, aborting.");
            return Ok(None);
        }
        let (pwr_func, pwr_sensors) = res.unwrap();

        let pwr = DGpuPowerIntel {
            pwr_func,
            pwr_sensors,
            last_gpu_val: 0,
            last_pkg_val: 0,
            delta_gpu_val: 0,
            delta_pkg_val: 0,
            nr_updates: 0,
            last_update: time::Instant::now(),
        };

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
    fn read(&self, offset: i64) -> Result<u64>
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
        if ret as usize != mem::size_of::<u64>() {
            bail!("Read wrong # bytes {:?} (expected {:?}) from MSR {:?}.",
                ret, mem::size_of::<u64>(), offset);
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

    fn probe(&self, offset: i64) -> bool
    {
        match self.read(offset) {
            Err(_) => false,
            _ => true
        }
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
    fn name(&self) -> &str
    {
        if self.pf_evt.is_some() { "iGPU:perf" } else { "iGPU:MSR" }
    }

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
    fn new_rapl_perf_event() -> Result<(PerfEvent, f64, f64)>
    {
        if !PerfEvent::is_capable() {
            bail!("No perf event support, no rapl power reporting");
        }

        let mut pf_evt = PerfEvent::from_pmu("power")?;
        let type_: u32 = pf_evt.source_type()?;
        let cpu: i32 = unsafe { libc::sched_getcpu() };

        let gpu_unit = pf_evt.event_unit("energy-gpu")?;
        let pkg_unit = pf_evt.event_unit("energy-pkg")?;
        if gpu_unit != "Joules" || pkg_unit != "Joules" {
            bail!("Both gpu {:?} and pkg {:?} units need to be Joules",
                gpu_unit, pkg_unit);
        }

        let gpu_scale: f64 = pf_evt.event_scale("energy-gpu")?;
        let pkg_scale: f64 = pf_evt.event_scale("energy-pkg")?;
        if gpu_scale == 0.0 || pkg_scale == 0.0 {
            bail!("Both gpu {:?} and pkg {:?} scales need to be > 0.0",
                gpu_scale, pkg_scale);
        }

        let gpu_cfg = pf_evt.event_config("energy-gpu")?;
        let pkg_cfg = pf_evt.event_config("energy-pkg")?;

        let mut pf_attr = perf_event_attr::new();
        pf_attr.type_ = type_;
        pf_attr.size = mem::size_of::<perf_event_attr>() as u32;
        pf_attr.sample_type = PERF_SAMPLE_IDENTIFIER;
        pf_attr.read_format = PERF_FORMAT_GROUP;

        pf_attr.config = gpu_cfg;
        pf_evt.group_open(&pf_attr, -1, cpu, 0)?;

        pf_attr.config = pkg_cfg;
        pf_evt.group_open(&pf_attr, -1, cpu, 0)?;

        Ok((pf_evt, gpu_scale, pkg_scale))
    }

    fn new_rapl_msr() -> Result<(MsrIntel, f64, f64)>
    {
        if !MsrIntel::is_capable() {
            bail!("Not capable of reading rapl power from MSR");
        }

        let cpu = unsafe { libc::sched_getcpu() };
        let msr = MsrIntel::from(cpu)?;

        if !msr.probe(MSR_RAPL_POWER_UNIT) ||
            !msr.probe(MSR_PKG_ENERGY_STATUS) ||
            !msr.probe(MSR_PP1_ENERGY_STATUS) {
            bail!("Can't read power unit, pkg and gpu MSRs");
        }

        let pu = msr.read(MSR_RAPL_POWER_UNIT)?;
        let scale = 1.0 / (1 << ((pu >> 8) & 0x1F)) as f64;

        Ok((msr, scale, scale))
    }

    pub fn new(mut use_msr: bool) -> Result<Option<Box<dyn GpuPowerIntel>>>
    {
        let mut pf_evt: Option<PerfEvent> = None;
        let mut msr: Option<MsrIntel> = None;
        let mut gpu_scale: f64 = 0.0;
        let mut pkg_scale: f64 = 0.0;

        if !use_msr {
            let pf_res = IGpuPowerIntel::new_rapl_perf_event();
            if let Ok(tup_res) = pf_res {
                let pf_evt_obj: PerfEvent;
                (pf_evt_obj, gpu_scale, pkg_scale) = tup_res;
                pf_evt = Some(pf_evt_obj);
            } else {
                debug!("ERR: couldn't get rapl power from perf: {:?}", pf_res);
                // fallback to MSR, if possible
                use_msr = true;
            }
        }

        if use_msr {
            let msr_res = IGpuPowerIntel::new_rapl_msr();
            if msr_res.is_err() {
                debug!("ERR: couldn't get rapl power from MSR: {:?}", msr_res);
                return Ok(None);
            }

            let msr_obj: MsrIntel;
            (msr_obj, gpu_scale, pkg_scale) = msr_res.unwrap();
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
