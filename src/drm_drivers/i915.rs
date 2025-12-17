#![allow(dead_code)]
#![allow(non_camel_case_types)]
#![allow(non_upper_case_globals)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::fs::{self, File};
use std::os::fd::{RawFd, AsRawFd};
use std::cell::RefCell;
use std::cmp::max;
use std::rc::Rc;
use std::time;
use std::alloc;
use std::mem;
use std::io;

use anyhow::{bail, Result};
use libc::Ioctl;
use log::{debug, info, warn};

use crate::perf_event::{perf_event_attr, PERF_FORMAT_GROUP, PerfEvent};
use crate::hwmon::Hwmon;
use crate::drm_drivers::{
    DrmDriver, helpers::{drm_iowr, drm_ioctl, __IncompleteArrayField},
    intel_power::{GpuPowerIntel, IGpuPowerIntel, DGpuPowerIntel},
};
use crate::drm_devices::{
    DrmDeviceType, DrmDeviceFreqs, DrmDeviceFreqLimits,
    DrmDeviceThrottleReasons, DrmDevicePower, DrmDeviceMemInfo,
    DrmDeviceTemperature, DrmDeviceFan, DrmDeviceInfo
};
use crate::drm_fdinfo::DrmMemRegion;
use crate::drm_clients::DrmClientMemInfo;


// based on rust-bindgen on Linux kernel v6.12+ uapi i915_drm.h
#[repr(C)]
#[derive(Debug, Copy, Clone)]
struct drm_i915_gem_memory_class_instance {
    memory_class: u16,
    memory_instance: u16,
}

const I915_MEMORY_CLASS_SYSTEM: u16 = 0;
const I915_MEMORY_CLASS_DEVICE: u16 = 1;

#[repr(C)]
#[derive(Debug, Copy, Clone)]
struct drm_i915_memory_region_info_cpu_visible_memory {
    probed_cpu_visible_size: u64,
    unallocated_cpu_visible_size: u64,
}

#[repr(C)]
#[derive(Copy, Clone)]
union drm_i915_memory_region_info_extra_info {
    rsvd1: [u64; 8usize],
    cpu: drm_i915_memory_region_info_cpu_visible_memory,
}

#[repr(C)]
#[derive(Copy, Clone)]
struct drm_i915_memory_region_info {
    region: drm_i915_gem_memory_class_instance,
    rsvd0: u32,
    probed_size: u64,
    unallocated_size: u64,
    extra_info: drm_i915_memory_region_info_extra_info,
}

#[repr(C)]
#[derive(Debug)]
struct drm_i915_query_memory_regions {
    num_regions: u32,
    rsvd: [u32; 3usize],
    regions: __IncompleteArrayField<drm_i915_memory_region_info>,
}

const DRM_I915_QUERY_MEMORY_REGIONS: u64 = 4;

#[repr(C)]
#[derive(Debug, Copy, Clone)]
struct drm_i915_query_item {
    query_id: u64,
    length: i32,
    flags: u32,
    data_ptr: u64,
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
struct drm_i915_query {
    num_items: u32,
    flags: u32,
    items_ptr: u64,
}

const DRM_I915_QUERY: Ioctl = 0x39;
const DRM_IOCTL_I915_QUERY: Ioctl = drm_iowr!(DRM_I915_QUERY,
    mem::size_of::<drm_i915_query>());

const I915_ENGINE_CLASS_RENDER: u16 = 0;
const I915_ENGINE_CLASS_COPY: u16 = 1;
const I915_ENGINE_CLASS_VIDEO: u16 = 2;
const I915_ENGINE_CLASS_VIDEO_ENHANCE: u16 = 3;
const I915_ENGINE_CLASS_COMPUTE: u16 = 4;

const QM_I915_ENGINE_CLASS_TOTAL: usize = 5;
const i915_engine_class_name: [&str; QM_I915_ENGINE_CLASS_TOTAL] = [
    "render",
    "copy",
    "video",
    "video-enhance",
    "compute",
];

#[derive(Debug)]
struct I915Engine
{
    name: String,
    class: u16,
}

#[derive(Debug)]
struct I915EnginePmuData
{
    idx: usize,
    last_active: u64,
}

#[derive(Debug)]
struct I915EnginesPmu
{
    pf_evt: PerfEvent,
    nr_evts: usize,
    nr_engs: usize,
    engs_data: Vec<Vec<I915EnginePmuData>>,
    nr_updates: u64,
    last_update: time::Instant,
}

impl I915EnginesPmu
{
    fn engs_utilization(&mut self) -> Result<HashMap<String, f64>>
    {
        let mut engs_ut = HashMap::new();

        let data = self.pf_evt.read(self.nr_evts + 1)?;
        let elapsed = self.last_update.elapsed().as_nanos() as u64;
        self.last_update = time::Instant::now();
        self.nr_updates += 1;

        for cn in 0..self.nr_engs {
            let mut acum_active = 0;
            let mut acum_total = 0;

            for epd in self.engs_data[cn].iter_mut() {
                let curr_active = data[1 + epd.idx];

                if self.nr_updates >= 2  {
                    acum_active += curr_active - epd.last_active;
                    acum_total += elapsed;
                }
                epd.last_active = curr_active;
            }

            let mut eut = if acum_active == 0 || acum_total == 0 {
                0.0
            } else {
                (acum_active as f64 / acum_total as f64) * 100.0
            };
            if eut > 100.0 {
                warn!("Engine {:?} utilization at {:.1}%, \
                    clamped to 100%.", i915_engine_class_name[cn], eut);
                eut = 100.0;
            }
            engs_ut.insert(i915_engine_class_name[cn].to_string(), eut);
        }

        Ok(engs_ut)
    }
}

#[derive(Debug)]
struct I915GTFreqsPmuData
{
    last_cur: u64,
    last_act: u64,
}

#[derive(Debug)]
struct I915FreqsPmu
{
    pf_evt: PerfEvent,
    gts_data: Vec<I915GTFreqsPmuData>,
    nr_updates: u64,
    elapsed: f64,
    last_update: time::Instant,
}

impl I915FreqsPmu
{
    // returns (requested, actual) freqs for a GT
    fn freqs(&mut self, gt_nr: usize, data: &Vec<u64>) -> Result<(u64, u64)>
    {
        if gt_nr >= self.gts_data.len() {
            bail!("No freqs PMU set up for GT {:?}", gt_nr);
        }
        let gt_data = &mut self.gts_data[gt_nr];

        let mut delta_cur = 0;
        let mut delta_act = 0;
        let base_idx = 1 + 2 * gt_nr;
        let curr_cur = data[base_idx];
        let curr_act = data[base_idx + 1];

        if self.nr_updates >= 2 {
            delta_cur = curr_cur - gt_data.last_cur;
            delta_act = curr_act - gt_data.last_act;
        }
        gt_data.last_cur = curr_cur;
        gt_data.last_act = curr_act;

        if self.elapsed == 0.0 {
            Ok((0, 0))
        } else {
            let ret_cur = delta_cur as f64 / self.elapsed;
            let ret_act = delta_act as f64 / self.elapsed;

            Ok((ret_cur as u64, ret_act as u64))
        }
    }

    fn read_all(&mut self) -> Result<Vec<u64>>
    {
        let data = self.pf_evt.read(1 + 2 * self.gts_data.len())?;
        self.elapsed = self.last_update.elapsed().as_secs_f64();
        self.last_update = time::Instant::now();
        self.nr_updates += 1;

        Ok(data)
    }
}

#[derive(Debug)]
pub struct DrmDriveri915
{
    _dn_file: File,
    dn_fd: RawFd,
    base_gts_dir: PathBuf,
    dev_type: Option<DrmDeviceType>,
    freq_limits: Option<Vec<DrmDeviceFreqLimits>>,
    power: Option<Box<dyn GpuPowerIntel>>,
    hwmon: Option<Hwmon>,
    engs_pmu: Option<I915EnginesPmu>,
    freqs_pmu: Option<I915FreqsPmu>,
}

impl DrmDriver for DrmDriveri915
{
    fn name(&self) -> &str
    {
        "i915"
    }

    fn dev_type(&mut self) -> Result<DrmDeviceType>
    {
        if let Some(dt) = &self.dev_type {
            return Ok(dt.clone());
        }

        let dmi = self.mem_info()?;
        let qmdt = if dmi.vram_total > 0 {
            DrmDeviceType::Discrete
        } else {
            DrmDeviceType::Integrated
        };

        self.dev_type = Some(qmdt.clone());
        Ok(qmdt)
    }

    fn mem_info(&mut self) -> Result<DrmDeviceMemInfo>
    {
        let mut dqi = drm_i915_query_item {
            query_id: DRM_I915_QUERY_MEMORY_REGIONS,
            length: 0,
            flags: 0,
            data_ptr: 0,
        };
        let dqi_ptr: *mut drm_i915_query_item = &mut dqi;

        let mut dq = drm_i915_query {
            num_items: 1,
            flags: 0,
            items_ptr: dqi_ptr as u64,
        };

        let res = drm_ioctl!(self.dn_fd, DRM_IOCTL_I915_QUERY, &mut dq);
        if res < 0 {
            return Err(io::Error::last_os_error().into());
        }

        if dqi.length as usize <= 0 {
            warn!("i915 memregions query ioctl() with {:?} length, skipping.",
                dqi.length as usize);
            return Ok(DrmDeviceMemInfo::new());
        }

        let layout = alloc::Layout::from_size_align(dqi.length as usize,
            mem::align_of::<u64>()).unwrap();
        let qmrg = unsafe {
            let ptr = alloc::alloc_zeroed(layout) as *mut drm_i915_query_memory_regions;
            if ptr.is_null() {
                panic!("Can't allocate memory for i915 memregions query ioctl()");
            }

            ptr
        };
        dqi.data_ptr = qmrg as u64;

        let res = drm_ioctl!(self.dn_fd, DRM_IOCTL_I915_QUERY, &mut dq);
        if res < 0 {
            unsafe { alloc::dealloc(qmrg as *mut u8, layout); }
            return Err(io::Error::last_os_error().into());
        }
        if dqi.length <= 0 {
            warn!("i915 memregions query ioctl() error: {:?}", dqi.length);
            unsafe { alloc::dealloc(qmrg as *mut u8, layout); }
            return Ok(DrmDeviceMemInfo::new());
        }
        let mrgs = unsafe {
            (*qmrg).regions.as_slice((*qmrg).num_regions as usize) };

        let mut qmdmi = DrmDeviceMemInfo::new();
        for mr in mrgs {
            match mr.region.memory_class {
                I915_MEMORY_CLASS_SYSTEM => {
                    qmdmi.smem_total += mr.probed_size;
                    qmdmi.smem_used += mr.probed_size - mr.unallocated_size;
                },
                I915_MEMORY_CLASS_DEVICE => {
                    qmdmi.vram_total += mr.probed_size;
                    qmdmi.vram_used += mr.probed_size - mr.unallocated_size;
                },
                _ => {
                    warn!("Unknown i915 memory class: {:?}, skipping mem region.",
                        mr.region.memory_class);
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
            let freqs_dir = self.base_gts_dir.join(format!("gt{}", nr));
            if !freqs_dir.is_dir() {
                break;
            }

            let fpath = freqs_dir.join("rps_RPn_freq_mhz");
            let fstr = fs::read_to_string(&fpath)?;
            let rpn_val: u64 = fstr.trim_end().parse()?;

            let fpath = freqs_dir.join("rps_RP1_freq_mhz");
            let fstr = fs::read_to_string(&fpath)?;
            let rp1_val: u64 = fstr.trim_end().parse()?;

            let fpath = freqs_dir.join("rps_RP0_freq_mhz");
            let fstr = fs::read_to_string(&fpath)?;
            let rp0_val: u64 = fstr.trim_end().parse()?;

            fls.push(DrmDeviceFreqLimits {
                name: format!("gt{}", nr),
                minimum: rpn_val,
                efficient: rp1_val,
                maximum: rp0_val,
            });
        }

        self.freq_limits = Some(fls.clone());
        Ok(fls)
    }

    fn freqs(&mut self) -> Result<Vec<DrmDeviceFreqs>>
    {
        let mut freqs_data = None;
        if let Some(fp) = &mut self.freqs_pmu {
            freqs_data = Some(fp.read_all()?);
        }

        let mut fqs = Vec::new();
        for nr in 0.. {
            let freqs_dir = self.base_gts_dir.join(format!("gt{}", nr));
            if !freqs_dir.is_dir() {
                break;
            }

            let fpath = freqs_dir.join("rps_min_freq_mhz");
            let fstr = fs::read_to_string(&fpath)?;
            let min_val: u64 = fstr.trim_end().parse()?;

            let fpath = freqs_dir.join("rps_max_freq_mhz");
            let fstr = fs::read_to_string(&fpath)?;
            let max_val: u64 = fstr.trim_end().parse()?;

            let (cur_val, act_val) = if self.freqs_pmu.is_none() {
                let fpath = freqs_dir.join("rps_cur_freq_mhz");
                let fstr = fs::read_to_string(&fpath)?;
                let c_val: u64 = fstr.trim_end().parse()?;

                let fpath = freqs_dir.join("rps_act_freq_mhz");
                let fstr = fs::read_to_string(&fpath)?;
                let a_val: u64 = fstr.trim_end().parse()?;

                (c_val, a_val)
            } else {
                self.freqs_pmu
                    .as_mut().unwrap()
                    .freqs(nr, freqs_data.as_ref().unwrap())?
            };

            let fpath = freqs_dir.join("throttle_reason_pl1");
            let pl1 = fpath.is_file() &&
                fs::read_to_string(&fpath)?.trim() == "1";

            let fpath = freqs_dir.join("throttle_reason_pl2");
            let pl2 = fpath.is_file() &&
                fs::read_to_string(&fpath)?.trim() == "1";

            let fpath = freqs_dir.join("throttle_reason_pl4");
            let pl4 = fpath.is_file() &&
                fs::read_to_string(&fpath)?.trim() == "1";

            let fpath = freqs_dir.join("throttle_reason_prochot");
            let prochot = fpath.is_file() &&
                fs::read_to_string(&fpath)?.trim() == "1";

            let fpath = freqs_dir.join("throttle_reason_ratl");
            let ratl = fpath.is_file() &&
                fs::read_to_string(&fpath)?.trim() == "1";

            let fpath = freqs_dir.join("throttle_reason_thermal");
            let thermal = fpath.is_file() &&
                fs::read_to_string(&fpath)?.trim() == "1";

            let fpath = freqs_dir.join("throttle_reason_vr_tdc");
            let vr_tdc = fpath.is_file() &&
                fs::read_to_string(&fpath)?.trim() == "1";

            let fpath = freqs_dir.join("throttle_reason_vr_thermalert");
            let vr_thermalert = fpath.is_file() &&
                fs::read_to_string(&fpath)?.trim() == "1";

            let fpath = freqs_dir.join("throttle_reason_status");
            let status = fpath.is_file() &&
                fs::read_to_string(&fpath)?.trim() == "1";

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
            if mr.name.starts_with("system") ||
                mr.name.starts_with("stolen-system") {
                cmi.smem_used += mr.total;
                cmi.smem_rss += mr.resident;
            } else if mr.name.starts_with("local") ||
                mr.name.starts_with("stolen-local") {
                cmi.vram_used += mr.total;
                cmi.vram_rss += mr.resident;
            } else {
                warn!("Unknown i915 memory region: {:?}, skpping it.", mr.name);
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
        if self.hwmon.is_none() {
            return Ok(Vec::new());
        }

        DrmDeviceTemperature::from_hwmon(self.hwmon.as_ref().unwrap())
    }

    fn fans(&mut self) -> Result<Vec<DrmDeviceFan>>
    {
        if self.hwmon.is_none() {
            return Ok(Vec::new());
        }

        DrmDeviceFan::from_hwmon(self.hwmon.as_ref().unwrap())
    }
}

impl DrmDriveri915
{
    fn engines_info(cpath: &str) -> Result<(usize, Vec<I915Engine>)>
    {
        let engs_dir = Path::new(cpath).join("engine");
        let mut engs = Vec::new();
        let mut nr_engs = 0;

        for et in fs::read_dir(&engs_dir)? {
            let path = et?.path();
            if !path.is_dir() {
                continue;
            }

            let name = fs::read_to_string(path.join("name"))?
                .trim()
                .to_string();
            let class: u16 = fs::read_to_string(path.join("class"))?
                .trim()
                .parse()?;

            nr_engs = max(nr_engs, class as usize + 1);
            engs.push(
                I915Engine {
                    name,
                    class,
                }
            );
        }

        Ok((nr_engs, engs))
    }

    fn init_engines_pmu(&mut self, src: &str, cpath: &str) -> Result<()>
    {
        let mut pf_evt = PerfEvent::new(src);
        let mut pf_attr = perf_event_attr::new();
        pf_attr.type_ = pf_evt.source_type()?;
        pf_attr.size = mem::size_of::<perf_event_attr>() as u32;
        pf_attr.read_format = PERF_FORMAT_GROUP;

        let (nr_engs, engs_info) = DrmDriveri915::engines_info(cpath)?;
        let mut engs_data = Vec::new();
        for _ in 0..nr_engs {
            let nvec: Vec<I915EnginePmuData> = Vec::new();
            engs_data.push(nvec);
        }
        let mut idx = 0;

        for eng in engs_info.iter() {
            let evt_name = format!("{}-busy", &eng.name);

            let evt_unit = pf_evt.event_unit(&evt_name)?;
            if evt_unit != "ns" {
                bail!("Event {:?} with unexpected unit {:?} vs \"ns\"",
                    &evt_name, &evt_unit);
            }

            pf_attr.config = pf_evt
                .event_keys_config(&evt_name, &vec!["config"])?["config"];
            // i915 PMUs only work with CPU 0
            pf_evt.group_open(&pf_attr, -1, 0, 0)?;

            engs_data[eng.class as usize].push(
                I915EnginePmuData {
                    idx,
                    last_active: 0,
                }
            );
            idx += 1;
        }

        self.engs_pmu = Some(
            I915EnginesPmu {
                pf_evt,
                nr_evts: idx,
                nr_engs,
                engs_data,
                nr_updates: 0,
                last_update: time::Instant::now(),
            }
        );

        Ok(())
    }

    fn init_freqs_pmu(&mut self, src: &str) -> Result<()>
    {
        let mut pf_evt = PerfEvent::new(src);
        let mut gts_data = Vec::new();

        let mut pf_attr = perf_event_attr::new();
        pf_attr.type_ = pf_evt.source_type()?;
        pf_attr.size = mem::size_of::<perf_event_attr>() as u32;
        pf_attr.read_format = PERF_FORMAT_GROUP;

        let cur_cfg = pf_evt
                .event_keys_config("requested-frequency",
                    &vec!["config"])?["config"];
        let act_cfg = pf_evt
                .event_keys_config("actual-frequency",
                    &vec!["config"])?["config"];

        let cur_unit = pf_evt.event_unit("requested-frequency")?;
        let act_unit = pf_evt.event_unit("actual-frequency")?;
        if cur_unit != "M" || act_unit != "M" {
            bail!("Requested and actual freqs not in \"M\", got {:?} and {:?}.",
                cur_unit, act_unit);
        }

        for nr in 0.. {
            let gt_dir = self.base_gts_dir.join(format!("gt{}", nr));
            if !gt_dir.is_dir() {
                break;
            }

            // i915 details:
            //  - GT nr goes in the top 4 bits of config
            //  - PMUs only work with CPU 0

            pf_attr.config = cur_cfg | ((nr as u64) << 60);
            pf_evt.group_open(&pf_attr, -1, 0, 0)?;

            pf_attr.config = act_cfg | ((nr as u64) << 60);
            pf_evt.group_open(&pf_attr, -1, 0, 0)?;

            gts_data.push(
                I915GTFreqsPmuData {
                    last_cur: 0,
                    last_act: 0,
                }
            );
        }

        self.freqs_pmu = Some(
            I915FreqsPmu {
                pf_evt,
                gts_data,
                nr_updates: 0,
                elapsed: 0.0,
                last_update: time::Instant::now(),
            }
        );

        Ok(())
    }

    fn pmu_evt_source(pci_dev: &str, dtype: &DrmDeviceType) -> Result<String>
    {
        if !PerfEvent::is_capable() {
            bail!("No PMU support");
        }

        let mut src = String::from("i915");
        if dtype.is_discrete() {
            src.push_str("_");
            src.push_str(pci_dev);
        }
        let src = src.replace(":", "_");

        if !PerfEvent::has_source(&src) {
            bail!("No PMU source {:?}", &src);
        }

        Ok(src)
    }

    fn parse_pmu_opts(pci_dev: &str, opts_vec: &Vec<&str>) -> (bool, bool)
    {
        let mut use_eng_pmu = false;
        let mut use_freq_pmu = false;

        for &opts_str in opts_vec.iter() {
            let sep_opts: Vec<&str> = opts_str.split(',').collect();
            let mut want_eng_pmu = false;
            let mut want_freq_pmu = false;
            let mut devslot = "all";

            for opt in sep_opts.iter() {
                if opt.starts_with("devslot=") {
                    devslot = &opt["devslot=".len()..];
                } else if opt == &"engines=pmu" {
                    want_eng_pmu = true;
                } else if opt == &"freqs=pmu" {
                    want_freq_pmu = true;
                }
            }

            if devslot == "all" || devslot == pci_dev {
                use_eng_pmu = use_eng_pmu || want_eng_pmu;
                use_freq_pmu = use_freq_pmu || want_freq_pmu;
            }
        }

        (use_eng_pmu, use_freq_pmu)
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

        // TODO: handle more than one tile
        let mut i915 = DrmDriveri915 {
            _dn_file: file,
            dn_fd: fd,
            base_gts_dir: Path::new(&cpath).join("gt"),
            dev_type: None,
            freq_limits: None,
            power: None,
            hwmon: None,
            engs_pmu: None,
            freqs_pmu: None,
        };

        let dtype = i915.dev_type()?;
        i915.freq_limits()?;

        if dtype.is_integrated() {
            i915.power = IGpuPowerIntel::new()?;
            if let Some(po) = &i915.power {
                info!("{}: rapl power reporting from: {}",
                    &qmd.pci_dev, po.name());
            } else {
                info!("{}: no rapl power reporting", &qmd.pci_dev);
            }
        } else if dtype.is_discrete() {
            let hwmon_res = Hwmon::from(
                Path::new(&cpath).join("device/hwmon"));
            if let Ok(hwmon) = hwmon_res {
                i915.power = DGpuPowerIntel::from(hwmon.as_ref().unwrap())?;
                i915.hwmon = hwmon;
            } else {
                debug!("{}: ERR: no Hwmon support on dGPU: {:?}",
                    &qmd.pci_dev, hwmon_res);
            }
            info!("{}: Hwmon power reporting: {}", &qmd.pci_dev,
                if i915.power.is_some() { "OK" } else { "FAILED" });
        }

        if let Some(opts_vec) = opts {
            let (mut use_eng_pmu, mut use_freq_pmu) =
                DrmDriveri915::parse_pmu_opts(&qmd.pci_dev, opts_vec);

            let pmu_src_res =
                DrmDriveri915::pmu_evt_source(&qmd.pci_dev, &dtype);
            if (use_eng_pmu || use_freq_pmu) && pmu_src_res.is_err() {
                use_eng_pmu = false;
                use_freq_pmu = false;
                debug!("{}: ERR: failed to find PMU source: {:?}",
                    &qmd.pci_dev, pmu_src_res);
            }
            let pmu_src = pmu_src_res.unwrap_or(String::new());

            if use_eng_pmu {
                let res = i915.init_engines_pmu(&pmu_src, &cpath);
                info!("{}: engines PMU init: {}",
                    &qmd.pci_dev, if res.is_ok() { "OK" } else { "FAILED" });
                if res.is_err() {
                    debug!("{}: ERR: failed to enable engines PMU: {:?}",
                        &qmd.pci_dev, res);
                }
            }
            if use_freq_pmu {
                let res = i915.init_freqs_pmu(&pmu_src);
                info!("{}: freqs PMU init: {}",
                    &qmd.pci_dev, if res.is_ok() { "OK" } else { "FAILED" });
                if res.is_err() {
                    debug!("{}: ERR: failed to enable freqs PMU: {:?}",
                        &qmd.pci_dev, res);
                }
            }
        }

        Ok(Rc::new(RefCell::new(i915)))
    }
}
