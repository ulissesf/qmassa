#![allow(dead_code)]
#![allow(non_camel_case_types)]
#![allow(non_upper_case_globals)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::fs::{self, File};
use std::os::fd::{RawFd, AsRawFd};
use std::cell::RefCell;
use std::rc::Rc;
use std::alloc;
use std::mem;
use std::io;

use anyhow::{bail, Result};
use libc;
use log::{debug, info, warn};

use crate::perf_event::{perf_event_attr, PERF_FORMAT_GROUP, PerfEvent};
use crate::hwmon::Hwmon;
use crate::drm_drivers::{
    DrmDriver, helpers::{drm_iowr, drm_ioctl, __IncompleteArrayField},
    intel_power::{GpuPowerIntel, IGpuPowerIntel, DGpuPowerIntel},
};
use crate::drm_devices::{
    DrmDeviceType, DrmDeviceFreqLimits, DrmDeviceFreqs,
    DrmDeviceThrottleReasons, DrmDevicePower, DrmDeviceMemInfo,
    DrmDeviceTemperature, DrmDeviceFan, DrmDeviceInfo
};
use crate::drm_fdinfo::DrmMemRegion;
use crate::drm_clients::DrmClientMemInfo;


// based on rust-bindgen on Linux kernel v6.12+ uapi xe_drm.h
#[repr(C)]
#[derive(Debug, Copy, Clone)]
struct drm_xe_mem_region {
    mem_class: u16,
    instance: u16,
    min_page_size: u32,
    total_size: u64,
    used: u64,
    cpu_visible_size: u64,
    cpu_visible_used: u64,
    reserved: [u64; 6usize],
}

const DRM_XE_MEM_REGION_CLASS_SYSMEM: u16 = 0;
const DRM_XE_MEM_REGION_CLASS_VRAM: u16 = 1;

#[repr(C)]
#[derive(Debug)]
struct drm_xe_query_mem_regions {
    num_mem_regions: u32,
    pad: u32,
    mem_regions: __IncompleteArrayField<drm_xe_mem_region>,
}

#[repr(C)]
#[derive(Debug)]
struct drm_xe_query_config {
    num_params: u32,
    pad: u32,
    info: __IncompleteArrayField<u64>,
}

const DRM_XE_QUERY_CONFIG_FLAGS: usize = 1;
const DRM_XE_QUERY_CONFIG_FLAG_HAS_VRAM: u64 = 1;

#[repr(C)]
#[derive(Debug, Copy, Clone)]
struct drm_xe_engine_class_instance {
    engine_class: u16,
    engine_instance: u16,
    gt_id: u16,
    pad: u16,
}

const DRM_XE_ENGINE_CLASS_RENDER: u16 = 0;
const DRM_XE_ENGINE_CLASS_COPY: u16 = 1;
const DRM_XE_ENGINE_CLASS_VIDEO_DECODE: u16 = 2;
const DRM_XE_ENGINE_CLASS_VIDEO_ENHANCE: u16 = 3;
const DRM_XE_ENGINE_CLASS_COMPUTE: u16 = 4;

const QM_DRM_XE_ENGINE_CLASS_TOTAL: usize = 5;
const xe_engine_class_name: [&str; QM_DRM_XE_ENGINE_CLASS_TOTAL] = [
    "rcs",
    "bcs",
    "vcs",
    "vecs",
    "ccs",
];

#[repr(C)]
#[derive(Debug, Copy, Clone)]
struct drm_xe_engine {
    instance: drm_xe_engine_class_instance,
    reserved: [u64; 3usize],
}

#[repr(C)]
#[derive(Debug)]
struct drm_xe_query_engines {
    num_engines: u32,
    pad: u32,
    engines: __IncompleteArrayField<drm_xe_engine>,
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
struct drm_xe_device_query {
    extensions: u64,
    query: u32,
    size: u32,
    data: u64,
    reserved: [u64; 2usize],
}

const DRM_XE_DEVICE_QUERY_ENGINES: u32 = 0;
const DRM_XE_DEVICE_QUERY_MEM_REGIONS: u32 = 1;
const DRM_XE_DEVICE_QUERY_CONFIG: u32 = 2;

const DRM_XE_DEVICE_QUERY: u64 = 0x00;
const DRM_IOCTL_XE_DEVICE_QUERY: u64 = drm_iowr!(DRM_XE_DEVICE_QUERY,
    mem::size_of::<drm_xe_device_query>());

#[derive(Debug)]
struct XeEngine
{
    gt_id: u16,
    class: u16,
    instance: u16,
}

#[derive(Debug)]
struct XeEnginePmuData
{
    base_idx: usize,
    last_active: u64,
    last_total: u64,
}

#[derive(Debug)]
struct XeEnginesPmu
{
    pf_evt: PerfEvent,
    nr_evts: usize,
    engs_data: Vec<Vec<XeEnginePmuData>>,
    nr_updates: u64,
}

impl XeEnginesPmu
{
    fn engs_utilization(&mut self) -> Result<HashMap<String, f64>>
    {
        let mut engs_ut = HashMap::new();

        let data = self.pf_evt.read(self.nr_evts + 1)?;
        self.nr_updates += 1;

        for cn in 0..QM_DRM_XE_ENGINE_CLASS_TOTAL {
            let mut acum_active = 0;
            let mut acum_total = 0;

            for epd in self.engs_data[cn].iter_mut() {
                let curr_active = data[1 + epd.base_idx];
                let curr_total = data[1 + epd.base_idx + 1];

                if self.nr_updates >= 2  {
                    acum_active += curr_active - epd.last_active;
                    acum_total += curr_total - epd.last_total;
                }
                epd.last_active = curr_active;
                epd.last_total = curr_total;
            }

            let mut eut = if acum_active == 0 || acum_total == 0 {
                0.0
            } else {
                (acum_active as f64 / acum_total as f64) * 100.0
            };
            if eut > 100.0 {
                warn!("Engine {:?} utilization at {:.1}%, \
                    clamped to 100%.", xe_engine_class_name[cn], eut);
                eut = 100.0;
            }
            engs_ut.insert(xe_engine_class_name[cn].to_string(), eut);
        }

        Ok(engs_ut)
    }
}

#[derive(Debug)]
pub struct DrmDriverXe
{
    _dn_file: File,
    dn_fd: RawFd,
    base_gts_dir: PathBuf,
    dev_type: Option<DrmDeviceType>,
    freq_limits: Option<Vec<DrmDeviceFreqLimits>>,
    power: Option<Box<dyn GpuPowerIntel>>,
    hwmon: Option<Hwmon>,
    engs_pmu: Option<XeEnginesPmu>,
}

impl DrmDriver for DrmDriverXe
{
    fn name(&self) -> &str
    {
        "xe"
    }

    fn dev_type(&mut self) -> Result<DrmDeviceType>
    {
        if let Some(dt) = &self.dev_type {
            return Ok(dt.clone());
        }

        let mut dq = drm_xe_device_query {
            extensions: 0,
            query: DRM_XE_DEVICE_QUERY_CONFIG,
            size: 0,
            data: 0,
            reserved: [0, 0],
        };

        let res = drm_ioctl!(self.dn_fd, DRM_IOCTL_XE_DEVICE_QUERY, &mut dq);
        if res < 0 {
            return Err(io::Error::last_os_error().into());
        }

        if dq.size as usize == 0 {
            warn!("Xe config query ioctl() returned 0 size, skipping.");
            return Ok(DrmDeviceType::Unknown);
        }

        let layout = alloc::Layout::from_size_align(dq.size as usize,
            mem::align_of::<u64>())?;
        let qcfg = unsafe {
            let ptr = alloc::alloc(layout) as *mut drm_xe_query_config;
            if ptr.is_null() {
                panic!("Can't allocate memory for Xe query config ioctl()");
            }

            ptr
        };
        dq.data = qcfg as u64;

        let res = drm_ioctl!(self.dn_fd, DRM_IOCTL_XE_DEVICE_QUERY, &mut dq);
        if res < 0 {
            unsafe { alloc::dealloc(qcfg as *mut u8, layout); }
            return Err(io::Error::last_os_error().into());
        }
        let cfg = unsafe { (*qcfg).info.as_slice((*qcfg).num_params as usize) };
        let flags = cfg[DRM_XE_QUERY_CONFIG_FLAGS];

        let qmdt = if flags & DRM_XE_QUERY_CONFIG_FLAG_HAS_VRAM > 0 {
            DrmDeviceType::Discrete
        } else {
            DrmDeviceType::Integrated
        };

        unsafe { alloc::dealloc(qcfg as *mut u8, layout); }

        self.dev_type = Some(qmdt.clone());
        Ok(qmdt)
    }

    fn mem_info(&mut self) -> Result<DrmDeviceMemInfo>
    {
        let mut dq = drm_xe_device_query {
            extensions: 0,
            query: DRM_XE_DEVICE_QUERY_MEM_REGIONS,
            size: 0,
            data: 0,
            reserved: [0, 0],
        };

        let res = drm_ioctl!(self.dn_fd, DRM_IOCTL_XE_DEVICE_QUERY, &mut dq);
        if res < 0 {
            return Err(io::Error::last_os_error().into());
        }

        if dq.size as usize == 0 {
            warn!("Xe mem regions query ioctl() returned 0 size, skipping.");
            return Ok(DrmDeviceMemInfo::new());
        }

        let layout = alloc::Layout::from_size_align(dq.size as usize,
            mem::align_of::<u64>())?;
        let qmrg = unsafe {
            let ptr = alloc::alloc(layout) as *mut drm_xe_query_mem_regions;
            if ptr.is_null() {
                panic!("Can't allocate memory for Xe query mem regions ioctl()");
            }

            ptr
        };
        dq.data = qmrg as u64;

        let res = drm_ioctl!(self.dn_fd, DRM_IOCTL_XE_DEVICE_QUERY, &mut dq);
        if res < 0 {
            unsafe { alloc::dealloc(qmrg as *mut u8, layout); }
            return Err(io::Error::last_os_error().into());
        }
        let mrgs = unsafe {
            (*qmrg).mem_regions.as_slice((*qmrg).num_mem_regions as usize) };

        let mut qmdmi = DrmDeviceMemInfo::new();
        for mr in mrgs {
            match mr.mem_class {
                DRM_XE_MEM_REGION_CLASS_SYSMEM => {
                    qmdmi.smem_total += mr.total_size;
                    qmdmi.smem_used += mr.used;
                },
                DRM_XE_MEM_REGION_CLASS_VRAM => {
                    qmdmi.vram_total += mr.total_size;
                    qmdmi.vram_used += mr.used;
                },
                _ => {
                    warn!("Unknown Xe memory class: {:?}, skipping mem region.",
                        mr.mem_class);
                    continue;
                }
            }
        }

        unsafe { alloc::dealloc(qmrg as *mut u8, layout); }

        Ok(qmdmi)
    }

    fn freq_limits(&mut self) -> Result<Vec<DrmDeviceFreqLimits>>
    {
        if let Some(fls) = &self.freq_limits {
            return Ok(fls.clone());
        }

        let mut fls = Vec::new();
        for nr in 0.. {
            let freqs_dir = self.base_gts_dir.join(format!("gt{}/freq0", nr));
            if !freqs_dir.is_dir() {
                break;
            }

            let fpath = freqs_dir.join("rpn_freq");
            let fstr = fs::read_to_string(&fpath)?;
            let rpn_val: u64 = fstr.trim_end().parse()?;

            let fpath = freqs_dir.join("rpe_freq");
            let fstr = fs::read_to_string(&fpath)?;
            let rpe_val: u64 = fstr.trim_end().parse()?;

            let fpath = freqs_dir.join("rp0_freq");
            let fstr = fs::read_to_string(&fpath)?;
            let rp0_val: u64 = fstr.trim_end().parse()?;

            fls.push(DrmDeviceFreqLimits {
                name: format!("gt{}", nr),
                minimum: rpn_val,
                efficient: rpe_val,
                maximum: rp0_val,
            });
        }

        self.freq_limits = Some(fls.clone());
        Ok(fls)
    }

    fn freqs(&mut self) -> Result<Vec<DrmDeviceFreqs>>
    {
        let mut fqs = Vec::new();
        for nr in 0.. {
            let freqs_dir = self.base_gts_dir.join(format!("gt{}/freq0", nr));
            if !freqs_dir.is_dir() {
                break;
            }
            let throttle_dir = freqs_dir.join("throttle");

            let fpath = freqs_dir.join("min_freq");
            let fstr = fs::read_to_string(&fpath)?;
            let min_val: u64 = fstr.trim_end().parse()?;

            let fpath = freqs_dir.join("cur_freq");
            let fstr = fs::read_to_string(&fpath)?;
            let cur_val: u64 = fstr.trim_end().parse()?;

            let fpath = freqs_dir.join("act_freq");
            let fstr = fs::read_to_string(&fpath)?;
            let act_val: u64 = fstr.trim_end().parse()?;

            let fpath = freqs_dir.join("max_freq");
            let fstr = fs::read_to_string(&fpath)?;
            let max_val: u64 = fstr.trim_end().parse()?;

            let fpath = throttle_dir.join("reason_pl1");
            let pl1 = fs::read_to_string(&fpath)?.trim() == "1";

            let fpath = throttle_dir.join("reason_pl2");
            let pl2 = fs::read_to_string(&fpath)?.trim() == "1";

            let fpath = throttle_dir.join("reason_pl4");
            let pl4 = fs::read_to_string(&fpath)?.trim() == "1";

            let fpath = throttle_dir.join("reason_prochot");
            let prochot = fs::read_to_string(&fpath)?.trim() == "1";

            let fpath = throttle_dir.join("reason_ratl");
            let ratl = fs::read_to_string(&fpath)?.trim() == "1";

            let fpath = throttle_dir.join("reason_thermal");
            let thermal = fs::read_to_string(&fpath)?.trim() == "1";

            let fpath = throttle_dir.join("reason_vr_tdc");
            let vr_tdc = fs::read_to_string(&fpath)?.trim() == "1";

            let fpath = throttle_dir.join("reason_vr_thermalert");
            let vr_thermalert = fs::read_to_string(&fpath)?.trim() == "1";

            let fpath = throttle_dir.join("status");
            let status = fs::read_to_string(&fpath)?.trim() == "1";

            let throttle = DrmDeviceThrottleReasons {
                pl1,
                pl2,
                pl4,
                prochot,
                ratl,
                thermal,
                vr_tdc,
                vr_thermalert,
                status,
            };

            fqs.push(DrmDeviceFreqs {
                min_freq: min_val,
                cur_freq: cur_val,
                act_freq: act_val,
                max_freq: max_val,
                throttle_reasons: throttle,
            });
        }

        Ok(fqs)
    }

    fn power(&mut self) -> Result<DrmDevicePower>
    {
        if self.power.is_none() {
            return Ok(DrmDevicePower::new());
        }

        self.power.as_mut().unwrap().power_usage(&self.hwmon)
    }

    fn client_mem_info(&mut self,
        mem_regs: &HashMap<String, DrmMemRegion>) -> Result<DrmClientMemInfo>
    {
        let mut cmi = DrmClientMemInfo::new();

        for mr in mem_regs.values() {
            if mr.name.starts_with("system") || mr.name.starts_with("gtt") {
                cmi.smem_used += mr.total;
                cmi.smem_rss += mr.resident;
            } else if mr.name.starts_with("vram") {
                cmi.vram_used += mr.total;
                cmi.vram_rss += mr.resident;
            } else if mr.name.starts_with("stolen") {
                if self.dev_type()?.is_discrete() {
                    cmi.vram_used += mr.total;
                    cmi.vram_rss += mr.resident;
                } else {
                    cmi.smem_used += mr.total;
                    cmi.smem_rss += mr.resident;
                }
            } else {
                warn!("Unknown Xe memory region: {:?}, skpping it.", mr.name);
                continue;
            }
        }

        Ok(cmi)
    }

    fn engs_utilization(&mut self) -> Result<HashMap<String, f64>>
    {
        if self.engs_pmu.is_none() {
            return Ok(HashMap::new());
        }

        self.engs_pmu.as_mut().unwrap().engs_utilization()
    }

    fn temps(&mut self) -> Result<Vec<DrmDeviceTemperature>>
    {
        if self.hwmon.is_some() {
            DrmDeviceTemperature::from_hwmon(self.hwmon.as_ref().unwrap())
        } else {
            Ok(Vec::new())
        }
    }

    fn fans(&mut self) -> Result<Vec<DrmDeviceFan>>
    {
        if self.hwmon.is_some() {
            DrmDeviceFan::from_hwmon(self.hwmon.as_ref().unwrap())
        } else {
            Ok(Vec::new())
        }
    }
}

impl DrmDriverXe
{
    fn engines_info(&self) -> Result<Vec<XeEngine>>
    {
        let mut dq = drm_xe_device_query {
            extensions: 0,
            query: DRM_XE_DEVICE_QUERY_ENGINES,
            size: 0,
            data: 0,
            reserved: [0, 0],
        };

        let res = drm_ioctl!(self.dn_fd, DRM_IOCTL_XE_DEVICE_QUERY, &mut dq);
        if res < 0 {
            return Err(io::Error::last_os_error().into());
        }

        if dq.size as usize == 0 {
            warn!("Xe query engines ioctl returned 0 size, aborting.");
            bail!("Xe query engines ioctl returned 0 size");
        }

        let layout = alloc::Layout::from_size_align(dq.size as usize,
            mem::align_of::<u64>())?;
        let qengs = unsafe {
            let ptr = alloc::alloc(layout) as *mut drm_xe_query_engines;
            if ptr.is_null() {
                panic!("Can't allocate memory for Xe query engines ioctl()");
            }

            ptr
        };
        dq.data = qengs as u64;

        let res = drm_ioctl!(self.dn_fd, DRM_IOCTL_XE_DEVICE_QUERY, &mut dq);
        if res < 0 {
            unsafe { alloc::dealloc(qengs as *mut u8, layout); }
            return Err(io::Error::last_os_error().into());
        }
        let engs = unsafe {
            (*qengs).engines.as_slice((*qengs).num_engines as usize) };

        let mut ret = Vec::new();
        for e in engs {
            let ne = XeEngine {
                gt_id: e.instance.gt_id,
                class: e.instance.engine_class,
                instance: e.instance.engine_instance,
            };
            ret.push(ne);
        }

        unsafe { alloc::dealloc(qengs as *mut u8, layout); }

        Ok(ret)
    }

    fn init_engines_pmu(&mut self, qmd: &DrmDeviceInfo) -> Result<()>
    {
        if !PerfEvent::is_capable() {
            bail!("No PMU support");
        }

        let mut src = String::from("xe_");
        src.push_str(&qmd.pci_dev);
        let src = src.replace(":", "_");

        if !PerfEvent::has_source(&src) {
            bail!("No PMU source {:?}", &src);
        }

        let mut pf_evt = PerfEvent::new(&src);
        let mut pf_attr = perf_event_attr::new();
        pf_attr.type_ = pf_evt.source_type()?;
        pf_attr.size = mem::size_of::<perf_event_attr>() as u32;
        pf_attr.read_format = PERF_FORMAT_GROUP;

        let cpu: i32 = unsafe { libc::sched_getcpu() };
        let act_cfg = pf_evt.event_config("engine-active-ticks")?;
        let tot_cfg = pf_evt.event_config("engine-total-ticks")?;

        let engs_info = self.engines_info()?;
        let mut engs_data = Vec::new();
        for _ in 0..QM_DRM_XE_ENGINE_CLASS_TOTAL {
            let nvec: Vec<XeEnginePmuData> = Vec::new();
            engs_data.push(nvec);
        }
        let mut idx = 0;

        for eng in engs_info.iter() {
            let eng_act_cfg = pf_evt.format_config(
                vec![
                    ("gt", eng.gt_id as u64),
                    ("engine_class", eng.class as u64),
                    ("engine_instance", eng.instance as u64)],
                act_cfg)?;
            let eng_tot_cfg = pf_evt.format_config(
                vec![
                    ("gt", eng.gt_id as u64),
                    ("engine_class", eng.class as u64),
                    ("engine_instance", eng.instance as u64)],
                tot_cfg)?;

            pf_attr.config = eng_act_cfg;
            pf_evt.group_open(&pf_attr, -1, cpu, 0)?;

            pf_attr.config = eng_tot_cfg;
            pf_evt.group_open(&pf_attr, -1, cpu, 0)?;

            engs_data[eng.class as usize].push(
                XeEnginePmuData {
                    base_idx: idx,
                    last_active: 0,
                    last_total: 0,
                }
            );
            idx += 2;
        }

        self.engs_pmu = Some(
            XeEnginesPmu {
                pf_evt,
                nr_evts: idx,
                engs_data,
                nr_updates: 0,
            }
        );

        Ok(())
    }

    pub fn new(qmd: &DrmDeviceInfo,
        opts: Option<&Vec<&str>>) -> Result<Rc<RefCell<dyn DrmDriver>>>
    {
        let file = File::open(qmd.drm_minors[0].devnode.clone())?;
        let fd = file.as_raw_fd();

        let mut cpath = String::from("/sys/class/drm/");
        let card = Path::new(&qmd.drm_minors[0].devnode)
            .file_name().unwrap().to_str().unwrap();
        cpath.push_str(card);
        let dev_path = Path::new(&cpath).join("device");

        // TODO: handle more than one tile
        let mut xe = DrmDriverXe {
            _dn_file: file,
            dn_fd: fd,
            base_gts_dir: dev_path.join("tile0"),
            dev_type: None,
            freq_limits: None,
            power: None,
            hwmon: None,
            engs_pmu: None,
        };

        let dtype = xe.dev_type()?;
        xe.freq_limits()?;

        if dtype.is_integrated() {
            xe.power = IGpuPowerIntel::new()?;
            if let Some(po) = &xe.power {
                info!("{}: rapl power reporting from: {}",
                    &qmd.pci_dev, po.name());
            } else {
                info!("{}: no rapl power reporting", &qmd.pci_dev);
            }
        } else if dtype.is_discrete() {
            let hwmon_res = Hwmon::from(dev_path.join("hwmon"));
            if let Ok(hwmon) = hwmon_res {
                xe.power = DGpuPowerIntel::from(hwmon.as_ref().unwrap())?;
                xe.hwmon = hwmon;
            } else {
                debug!("ERR: no Hwmon support on dGPU: {:?}", hwmon_res);
            }
            info!("{}: Hwmon power reporting: {}", &qmd.pci_dev,
                if xe.power.is_some() { "OK" } else { "FAILED" });
        }

        if let Some(opts_vec) = opts {
            let mut use_eng_pmu = false;

            for &opts_str in opts_vec.iter() {
                let sep_opts: Vec<&str> = opts_str.split(',').collect();
                let mut want_eng_pmu = false;
                let mut devslot = "all";

                for opt in sep_opts.iter() {
                    if opt.starts_with("devslot=") {
                        devslot = &opt["devslot=".len()..];
                    } else if opt == &"engines=pmu" {
                        want_eng_pmu = true;
                    }
                }

                if want_eng_pmu &&
                    (devslot == "all" || devslot == qmd.pci_dev) {
                    use_eng_pmu = true;
                }
            }

            if use_eng_pmu {
                let res = xe.init_engines_pmu(qmd);
                info!("{}: engines PMU init: {}",
                    &qmd.pci_dev, if res.is_ok() { "OK" } else { "FAILED" });
                if res.is_err() {
                    debug!("ERR: failed to enable engines PMU: {:?}", res);
                }
            }
        }

        Ok(Rc::new(RefCell::new(xe)))
    }
}
