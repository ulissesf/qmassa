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
use std::alloc;
use std::mem;
use std::io;

use anyhow::{bail, Result};
use libc::{self, Ioctl};
use log::{debug, info, warn};

use crate::perf_event::{perf_event_attr, PERF_FORMAT_GROUP, PerfEvent};
use crate::hwmon::Hwmon;
use crate::drm_drivers::{
    DrmDriver, helpers::{drm_iowr, drm_ioctl, __IncompleteArrayField},
    intel_power::{GpuPowerIntel, IGpuPowerIntel, DGpuPowerIntel},
};
use crate::drm_devices::{
    VirtFn, DrmDeviceType, DrmDeviceFreqLimits, DrmDeviceFreqs,
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

const DRM_XE_DEVICE_QUERY: Ioctl = 0x00;
const DRM_IOCTL_XE_DEVICE_QUERY: Ioctl = drm_iowr!(DRM_XE_DEVICE_QUERY,
    mem::size_of::<drm_xe_device_query>());

//
// SR-IOV and PMU helpers
//

fn xe_sriov_pf_dev_from(pci_dev: &str, dev_path: &PathBuf) -> Result<String>
{
    let pf_path = dev_path.join("physfn");
    if !pf_path.is_symlink() {
        return Ok(pci_dev.to_string());
    }

    Ok(fs::read_link(pf_path)?
        .file_name().unwrap()
        .to_str().unwrap()
        .to_string())
}

fn xe_pmu_source_from(pci_dev: &str, dev_path: &PathBuf) -> Result<String>
{
    if !PerfEvent::is_capable() {
        bail!("No PMU support");
    }

    let mut src = String::from("xe_");
    src.push_str(&xe_sriov_pf_dev_from(pci_dev, dev_path)?);
    let src = src.replace(":", "_");

    if !PerfEvent::has_source(&src) {
        bail!("No PMU source {:?}", &src);
    }

    Ok(src)
}

fn xe_sriov_fn_from(pci_dev: &str, dev_path: &PathBuf) -> Result<u64>
{
    let pf_path = dev_path.join("physfn");
    if !pf_path.is_symlink() {
        // PF fn is 0
        return Ok(0);
    }

    // find VF fn number > 0
    for nr in 0.. {
        let virt_path = pf_path.join(format!("virtfn{}", nr));
        if !virt_path.is_symlink() {
            bail!("Couldn't find SR-IOV VF fn for {:?}", pci_dev);
        }

        let dpath = fs::read_link(virt_path)?
            .file_name().unwrap()
            .to_str().unwrap()
            .to_string();
        if dpath == pci_dev {
            return Ok(nr as u64 + 1);
        }
    }

    bail!("Couldn't find SR-IOV fn for {:?}", pci_dev);
}

#[derive(Debug)]
struct XeEngine
{
    gt_id: u64,
    class: u64,
    instance: u64,
}

impl XeEngine
{
    fn engines_from(dn_fd: RawFd) -> Result<(usize, Vec<XeEngine>)>
    {
        let mut dq = drm_xe_device_query {
            extensions: 0,
            query: DRM_XE_DEVICE_QUERY_ENGINES,
            size: 0,
            data: 0,
            reserved: [0, 0],
        };

        let res = drm_ioctl!(dn_fd, DRM_IOCTL_XE_DEVICE_QUERY, &mut dq);
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

        let res = drm_ioctl!(dn_fd, DRM_IOCTL_XE_DEVICE_QUERY, &mut dq);
        if res < 0 {
            unsafe { alloc::dealloc(qengs as *mut u8, layout); }
            return Err(io::Error::last_os_error().into());
        }
        let engs = unsafe {
            (*qengs).engines.as_slice((*qengs).num_engines as usize) };

        let mut ret = Vec::new();
        let mut nr_engs = 0;
        for e in engs {
            nr_engs = max(nr_engs, e.instance.engine_class as usize + 1);
            let ne = XeEngine {
                gt_id: e.instance.gt_id as u64,
                class: e.instance.engine_class as u64,
                instance: e.instance.engine_instance as u64,
            };
            ret.push(ne);
        }

        unsafe { alloc::dealloc(qengs as *mut u8, layout); }

        Ok((nr_engs, ret))
    }
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
    nr_engs: usize,
    engs_data: Vec<Vec<XeEnginePmuData>>,
    nr_updates: u64,
}

impl XeEnginesPmu
{
    fn engs_utilization(&mut self) -> Result<HashMap<String, f64>>
    {
        let mut engs_ut = HashMap::new();

        let data = self.pf_evt.read(1 + self.nr_evts)?;
        self.nr_updates += 1;

        for cn in 0..self.nr_engs {
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
                warn!("Engine {:?} utilization at {:?}%, \
                    clamped to 100%.", xe_engine_class_name[cn], eut);
                eut = 100.0;
            }
            engs_ut.insert(xe_engine_class_name[cn].to_string(), eut);
        }

        Ok(engs_ut)
    }

    fn from(pci_dev: &str, dev_path: &PathBuf,
        dn_fd: RawFd, src: &str) -> Result<XeEnginesPmu>
    {
        let sriov_fn = xe_sriov_fn_from(pci_dev, dev_path);
        if let Err(err) = sriov_fn {
            bail!("ERR: failed getting SR-IOV fn from {:?}: {}",
                pci_dev, err);
        }
        let sriov_fn = sriov_fn.unwrap();

        let mut pf_evt = PerfEvent::from_pmu(src)?;
        let mut pf_attr = perf_event_attr::new();
        pf_attr.type_ = pf_evt.source_type()?;
        pf_attr.size = mem::size_of::<perf_event_attr>() as u32;
        pf_attr.read_format = PERF_FORMAT_GROUP;

        let cpu: i32 = unsafe { libc::sched_getcpu() };
        let act_cfg = pf_evt.event_config("engine-active-ticks")?;
        let tot_cfg = pf_evt.event_config("engine-total-ticks")?;
        let has_sriov = pf_evt.has_format_param("function");

        let (nr_engs, engs_info) = XeEngine::engines_from(dn_fd)?;
        let mut engs_data = Vec::new();
        for _ in 0..nr_engs {
            let nvec: Vec<XeEnginePmuData> = Vec::new();
            engs_data.push(nvec);
        }
        let mut idx = 0;

        for eng in engs_info.iter() {
            let mut params = vec![
                ("gt", eng.gt_id),
                ("engine_class", eng.class),
                ("engine_instance", eng.instance),
            ];
            if has_sriov {
                params.push(("function", sriov_fn));
            }

            pf_attr.config = pf_evt.format_config(&params, act_cfg)?;
            pf_evt.group_open(&pf_attr, -1, cpu, 0)?;

            pf_attr.config = pf_evt.format_config(&params, tot_cfg)?;
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

        Ok(
            XeEnginesPmu {
                pf_evt,
                nr_evts: idx,
                nr_engs,
                engs_data,
                nr_updates: 0,
            }
        )
    }
}

#[derive(Debug)]
struct XeGTFreqsPmuData
{
    last_cur: u64,
    last_act: u64,
}

#[derive(Debug)]
struct XeFreqsPmu
{
    pf_evt: PerfEvent,
    gts_data: Vec<XeGTFreqsPmuData>,
    nr_updates: u64,
}

impl XeFreqsPmu
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

        Ok((delta_cur, delta_act))
    }

    fn read_all(&mut self) -> Result<Vec<u64>>
    {
        let data = self.pf_evt.read(1 + 2 * self.gts_data.len())?;
        self.nr_updates += 1;

        Ok(data)
    }

    fn from(base_gts_dir: &PathBuf, src: &str) -> Result<XeFreqsPmu>
    {
        let mut pf_evt = PerfEvent::from_pmu(src)?;
        let mut gts_data = Vec::new();

        let mut pf_attr = perf_event_attr::new();
        pf_attr.type_ = pf_evt.source_type()?;
        pf_attr.size = mem::size_of::<perf_event_attr>() as u32;
        pf_attr.read_format = PERF_FORMAT_GROUP;

        let cpu: i32 = unsafe { libc::sched_getcpu() };
        let cur_cfg = pf_evt.event_config("gt-requested-frequency")?;
        let act_cfg = pf_evt.event_config("gt-actual-frequency")?;

        let cur_unit = pf_evt.event_unit("gt-requested-frequency")?;
        let act_unit = pf_evt.event_unit("gt-actual-frequency")?;
        if cur_unit != "MHz" || act_unit != "MHz" {
            bail!("Requested and actual freqs not in MHz, got {:?} and {:?}.",
                cur_unit, act_unit);
        }

        for nr in 0.. {
            let gt_dir = base_gts_dir.join(format!("gt{}", nr));
            if !gt_dir.is_dir() {
                break;
            }

            let params = vec![("gt", nr),];

            pf_attr.config = pf_evt.format_config(&params, cur_cfg)?;
            pf_evt.group_open(&pf_attr, -1, cpu, 0)?;

            pf_attr.config = pf_evt.format_config(&params, act_cfg)?;
            pf_evt.group_open(&pf_attr, -1, cpu, 0)?;

            gts_data.push(
                XeGTFreqsPmuData {
                    last_cur: 0,
                    last_act: 0,
                }
            );
         }

        Ok(
            XeFreqsPmu {
                pf_evt,
                gts_data,
                nr_updates: 0,
            }
        )
    }
}

fn xe_dev_type_from(dn_fd: RawFd, dev_path: &PathBuf) -> Result<DrmDeviceType>
{
    // find virtualization fn, if any
    let is_vfio = dev_path.join("vfio-dev").is_dir();

    let virt_fn = if is_vfio {
        VirtFn::VFIO
    } else if dev_path.join("physfn").is_symlink() {
        VirtFn::SriovVF
    } else if dev_path.join("virtfn0").is_symlink() {
        VirtFn::SriovPF
    } else {
        VirtFn::NoVirt
    };

    // find discrete vs integrated type
    let mut dq = drm_xe_device_query {
        extensions: 0,
        query: DRM_XE_DEVICE_QUERY_CONFIG,
        size: 0,
        data: 0,
        reserved: [0, 0],
    };

    let res = drm_ioctl!(dn_fd, DRM_IOCTL_XE_DEVICE_QUERY, &mut dq);
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

    let res = drm_ioctl!(dn_fd, DRM_IOCTL_XE_DEVICE_QUERY, &mut dq);
    if res < 0 {
        unsafe { alloc::dealloc(qcfg as *mut u8, layout); }
        return Err(io::Error::last_os_error().into());
    }
    let cfg = unsafe { (*qcfg).info.as_slice((*qcfg).num_params as usize) };
    let flags = cfg[DRM_XE_QUERY_CONFIG_FLAGS];

    let qmdt = if flags & DRM_XE_QUERY_CONFIG_FLAG_HAS_VRAM > 0 {
        DrmDeviceType::Discrete(virt_fn)
    } else {
        DrmDeviceType::Integrated(virt_fn)
    };

    unsafe { alloc::dealloc(qcfg as *mut u8, layout); }

    Ok(qmdt)
}

#[derive(Debug)]
pub struct DrmDriverXeVfio
{
    dev_type: DrmDeviceType,
    engs_pmu: Option<XeEnginesPmu>,
}

impl DrmDriver for DrmDriverXeVfio
{
    fn name(&self) -> &str
    {
        "xe-vfio-pci"
    }

    fn dev_type(&mut self) -> Result<DrmDeviceType>
    {
        Ok(self.dev_type.clone())
    }

    fn engs_utilization(&mut self) -> Result<HashMap<String, f64>>
    {
        if self.engs_pmu.is_none() {
            return Ok(HashMap::new());
        }

        self.engs_pmu.as_mut().unwrap().engs_utilization()
    }
}

impl DrmDriverXeVfio
{
    fn find_card_dir(path: &PathBuf) -> Option<String>
    {
        for entry in fs::read_dir(path).ok()? {
            let entry = entry.ok()?;
            let epath = entry.path();

            if epath.is_dir() {
                if let Some(nm) = epath.file_name().and_then(|n| n.to_str()) {
                    if nm.starts_with("card") {
                        return Some(nm.to_string());
                    }
                }
            }
        }

        None
    }

    pub fn new(qmd: &DrmDeviceInfo,
        _opts: Option<&Vec<&str>>) -> Result<Rc<RefCell<dyn DrmDriver>>>
    {
        let mut vpath = String::from("/sys/class/vfio-dev/");
        let vfio = Path::new(&qmd.dev_nodes[0].devnode)
            .file_name().unwrap().to_str().unwrap();
        vpath.push_str(vfio);
        let dev_path = Path::new(&vpath).join("device");

        let cname_res = DrmDriverXeVfio::find_card_dir(
            &dev_path.join("physfn").join("drm"));
        if cname_res.is_none() {
            bail!("{}: no DRM card for VFIO physfn, aborting.", &qmd.pci_dev);
        }
        let cname = cname_res.unwrap();

        let file = File::open(Path::new("/dev/dri").join(&cname))?;
        let fd = file.as_raw_fd();

        let dev_type = xe_dev_type_from(fd, &dev_path)?;
        let mut engs_pmu = None;

        let pmu_src_res = xe_pmu_source_from(&qmd.pci_dev, &dev_path);
        if pmu_src_res.is_err() {
            debug!("{}: ERR: failed to find PMU source: {:?}",
                &qmd.pci_dev, pmu_src_res);
        } else {
            let pmu_src = pmu_src_res.unwrap();

            let res = XeEnginesPmu::from(
                &qmd.pci_dev, &dev_path, fd, &pmu_src);
            info!("{}: engines PMU init: {}",
                &qmd.pci_dev, if res.is_ok() { "OK" } else { "FAILED" });
            if res.is_err() {
                debug!("{}: ERR: failed to enable engines PMU: {:?}",
                    &qmd.pci_dev, res);
            } else {
                engs_pmu = Some(res.unwrap());
            }
        }

        let xe_vfio = DrmDriverXeVfio {
            dev_type,
            engs_pmu,
        };

        Ok(Rc::new(RefCell::new(xe_vfio)))
    }
}

#[derive(Debug)]
pub struct DrmDriverXe
{
    _dn_file: File,
    dn_fd: RawFd,
    base_gts_dir: PathBuf,
    dev_type: DrmDeviceType,
    freq_limits: Option<Vec<DrmDeviceFreqLimits>>,
    power: Option<Box<dyn GpuPowerIntel>>,
    hwmon: Option<Hwmon>,
    engs_pmu: Option<XeEnginesPmu>,
    freqs_pmu: Option<XeFreqsPmu>,
}

impl DrmDriver for DrmDriverXe
{
    fn name(&self) -> &str
    {
        "xe"
    }

    fn dev_type(&mut self) -> Result<DrmDeviceType>
    {
        Ok(self.dev_type.clone())
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
        let mut freqs_data = None;
        if let Some(fp) = &mut self.freqs_pmu {
            freqs_data = Some(fp.read_all()?);
        }

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

            let fpath = freqs_dir.join("max_freq");
            let fstr = fs::read_to_string(&fpath)?;
            let max_val: u64 = fstr.trim_end().parse()?;

            let (cur_val, act_val) = if self.freqs_pmu.is_none() {
                let fpath = freqs_dir.join("cur_freq");
                let fstr = fs::read_to_string(&fpath)?;
                let c_val: u64 = fstr.trim_end().parse()?;

                let fpath = freqs_dir.join("act_freq");
                let fstr = fs::read_to_string(&fpath)?;
                let a_val: u64 = fstr.trim_end().parse()?;

                (c_val, a_val)
            } else {
                self.freqs_pmu
                    .as_mut().unwrap()
                    .freqs(nr, freqs_data.as_ref().unwrap())?
            };

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

impl DrmDriverXe
{
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
        let file = File::open(&qmd.dev_nodes[0].devnode)?;
        let fd = file.as_raw_fd();

        let mut cpath = String::from("/sys/class/drm/");
        let card = Path::new(&qmd.dev_nodes[0].devnode)
            .file_name().unwrap().to_str().unwrap();
        cpath.push_str(card);
        let dev_path = Path::new(&cpath).join("device");

        let dev_type = xe_dev_type_from(fd, &dev_path)?;

        // TODO: handle more than one tile
        let mut xe = DrmDriverXe {
            _dn_file: file,
            dn_fd: fd,
            base_gts_dir: dev_path.join("tile0"),
            dev_type,
            freq_limits: None,
            power: None,
            hwmon: None,
            engs_pmu: None,
            freqs_pmu: None,
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
                debug!("{}: ERR: no Hwmon support on dGPU: {:?}",
                    &qmd.pci_dev, hwmon_res);
            }
            info!("{}: Hwmon power reporting: {}", &qmd.pci_dev,
                if xe.power.is_some() { "OK" } else { "FAILED" });
        }

        if let Some(opts_vec) = opts {
            let (mut use_eng_pmu, mut use_freq_pmu) =
                DrmDriverXe::parse_pmu_opts(&qmd.pci_dev, opts_vec);

            let pmu_src_res = xe_pmu_source_from(&qmd.pci_dev, &dev_path);
            if (use_eng_pmu || use_freq_pmu) && pmu_src_res.is_err() {
                use_eng_pmu = false;
                use_freq_pmu = false;
                debug!("{}: ERR: failed to find PMU source: {:?}",
                    &qmd.pci_dev, pmu_src_res);
            }
            let pmu_src = pmu_src_res.unwrap_or(String::new());

            if use_eng_pmu {
                let res = XeEnginesPmu::from(
                    &qmd.pci_dev, &dev_path, xe.dn_fd, &pmu_src);
                info!("{}: engines PMU init: {}",
                    &qmd.pci_dev, if res.is_ok() { "OK" } else { "FAILED" });
                if res.is_err() {
                    debug!("{}: ERR: failed to enable engines PMU: {:?}",
                        &qmd.pci_dev, res);
                } else {
                    xe.engs_pmu = Some(res.unwrap());
                }
            }
            if use_freq_pmu {
                let res = XeFreqsPmu::from(&xe.base_gts_dir, &pmu_src);
                info!("{}: freqs PMU init: {}",
                    &qmd.pci_dev, if res.is_ok() { "OK" } else { "FAILED" });
                if res.is_err() {
                    debug!("{}: ERR: failed to enable freqs PMU: {:?}",
                        &qmd.pci_dev, res);
                } else {
                    xe.freqs_pmu = Some(res.unwrap());
                }
            }
        }

        Ok(Rc::new(RefCell::new(xe)))
    }
}
