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

use anyhow::Result;
use log::warn;
use libc;

use crate::drm_drivers::DrmDriver;
use crate::drm_drivers::helpers::{drm_iowr, __IncompleteArrayField};
use crate::drm_devices::{
    DrmDeviceType, DrmDeviceFreqLimits, DrmDeviceFreqs,
    DrmDeviceThrottleReasons, DrmDeviceMemInfo, DrmDeviceInfo
};
use crate::drm_fdinfo::DrmMemRegion;
use crate::drm_clients::DrmClientMemInfo;


// rust-bindgen 0.69.4 on Linux kernel v6.12 uapi xe_drm.h + changes
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

const DRM_XE_DEVICE_QUERY_MEM_REGIONS: u32 = 1;

#[repr(C)]
#[derive(Debug)]
struct drm_xe_query_config {
    num_params: u32,
    pad: u32,
    info: __IncompleteArrayField<u64>,
}

const DRM_XE_QUERY_CONFIG_REV_AND_DEVICE_ID: u32 = 0;
const DRM_XE_QUERY_CONFIG_FLAGS: u32 = 1;
const DRM_XE_QUERY_CONFIG_FLAG_HAS_VRAM: u64 = 1;
const DRM_XE_QUERY_CONFIG_MIN_ALIGNMENT: u64 = 2;
const DRM_XE_QUERY_CONFIG_VA_BITS: u32 = 3;
const DRM_XE_QUERY_CONFIG_MAX_EXEC_QUEUE_PRIORITY: u32 = 4;

const DRM_XE_DEVICE_QUERY_CONFIG: u32 = 2;

#[repr(C)]
#[derive(Debug, Copy, Clone)]
struct drm_xe_device_query {
    extensions: u64,
    query: u32,
    size: u32,
    data: u64,
    reserved: [u64; 2usize],
}

const DRM_XE_DEVICE_QUERY: u64 = 0x00;
const DRM_IOCTL_XE_DEVICE_QUERY: u64 = drm_iowr!(DRM_XE_DEVICE_QUERY,
    mem::size_of::<drm_xe_device_query>());

#[derive(Debug)]
pub struct DrmDriverXe
{
    dn_file: File,
    dn_fd: RawFd,
    freqs_dir: PathBuf,
    throttle_dir: PathBuf,
    dev_type: Option<DrmDeviceType>,
    freq_limits: Option<DrmDeviceFreqLimits>,
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

        let res = unsafe {
            libc::ioctl(self.dn_fd, DRM_IOCTL_XE_DEVICE_QUERY, &mut dq) };
        if res < 0 {
            return Err(io::Error::last_os_error().into());
        }

        if dq.size as usize == 0 {
            warn!("Xe config query ioctl() returned 0 size, skipping.");
            return Ok(DrmDeviceType::Unknown);
        }

        let layout = alloc::Layout::from_size_align(dq.size as usize,
            mem::size_of::<u64>())?;
        let qcfg = unsafe {
            let ptr = alloc::alloc(layout) as *mut drm_xe_query_config;
            if ptr.is_null() {
                panic!("Can't allocate memory for Xe query config ioctl()");
            }

            ptr
        };
        dq.data = qcfg as u64;

        let res = unsafe {
            libc::ioctl(self.dn_fd, DRM_IOCTL_XE_DEVICE_QUERY, &mut dq) };
        if res < 0 {
            unsafe { alloc::dealloc(qcfg as *mut u8, layout); }
            return Err(io::Error::last_os_error().into());
        }
        let cfg = unsafe { (*qcfg).info.as_slice((*qcfg).num_params as usize) };
        let flags = cfg[DRM_XE_QUERY_CONFIG_FLAGS as usize];

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

        let res = unsafe {
            libc::ioctl(self.dn_fd, DRM_IOCTL_XE_DEVICE_QUERY, &mut dq) };
        if res < 0 {
            return Err(io::Error::last_os_error().into());
        }

        if dq.size as usize == 0 {
            warn!("Xe mem regions query ioctl() returned 0 size, skipping.");
            return Ok(DrmDeviceMemInfo::new());
        }

        let layout = alloc::Layout::from_size_align(dq.size as usize,
            mem::size_of::<u64>())?;
        let qmrg = unsafe {
            let ptr = alloc::alloc(layout) as *mut drm_xe_query_mem_regions;
            if ptr.is_null() {
                panic!("Can't allocate memory for Xe query mem regions ioctl()");
            }

            ptr
        };
        dq.data = qmrg as u64;

        let res = unsafe {
            libc::ioctl(self.dn_fd, DRM_IOCTL_XE_DEVICE_QUERY, &mut dq) };
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

    fn freq_limits(&mut self) -> Result<DrmDeviceFreqLimits>
    {
        if let Some(fls) = &self.freq_limits {
            return Ok(fls.clone());
        }

        let fpath = self.freqs_dir.join("rpn_freq");
        let fstr = fs::read_to_string(&fpath)?;
        let rpn_val: u64 = fstr.trim_end().parse()?;

        let fpath = self.freqs_dir.join("rpe_freq");
        let fstr = fs::read_to_string(&fpath)?;
        let rpe_val: u64 = fstr.trim_end().parse()?;

        let fpath = self.freqs_dir.join("rp0_freq");
        let fstr = fs::read_to_string(&fpath)?;
        let rp0_val: u64 = fstr.trim_end().parse()?;

        let fls = DrmDeviceFreqLimits {
            minimum: rpn_val,
            efficient: rpe_val,
            maximum: rp0_val,
        };

        self.freq_limits = Some(fls.clone());
        Ok(fls)
    }

    fn freqs(&mut self) -> Result<DrmDeviceFreqs>
    {
        let fpath = self.freqs_dir.join("min_freq");
        let fstr = fs::read_to_string(&fpath)?;
        let min_val: u64 = fstr.trim_end().parse()?;

        let fpath = self.freqs_dir.join("cur_freq");
        let fstr = fs::read_to_string(&fpath)?;
        let cur_val: u64 = fstr.trim_end().parse()?;

        let fpath = self.freqs_dir.join("act_freq");
        let fstr = fs::read_to_string(&fpath)?;
        let act_val: u64 = fstr.trim_end().parse()?;

        let fpath = self.freqs_dir.join("max_freq");
        let fstr = fs::read_to_string(&fpath)?;
        let max_val: u64 = fstr.trim_end().parse()?;

        let fpath = self.throttle_dir.join("reason_pl1");
        let pl1 = fs::read_to_string(&fpath)?.trim() == "1";

        let fpath = self.throttle_dir.join("reason_pl2");
        let pl2 = fs::read_to_string(&fpath)?.trim() == "1";

        let fpath = self.throttle_dir.join("reason_pl4");
        let pl4 = fs::read_to_string(&fpath)?.trim() == "1";

        let fpath = self.throttle_dir.join("reason_prochot");
        let prochot = fs::read_to_string(&fpath)?.trim() == "1";

        let fpath = self.throttle_dir.join("reason_ratl");
        let ratl = fs::read_to_string(&fpath)?.trim() == "1";

        let fpath = self.throttle_dir.join("reason_thermal");
        let thermal = fs::read_to_string(&fpath)?.trim() == "1";

        let fpath = self.throttle_dir.join("reason_vr_tdc");
        let vr_tdc = fs::read_to_string(&fpath)?.trim() == "1";

        let fpath = self.throttle_dir.join("reason_vr_thermalert");
        let vr_thermalert = fs::read_to_string(&fpath)?.trim() == "1";

        let fpath = self.throttle_dir.join("status");
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

        Ok(DrmDeviceFreqs {
            min_freq: min_val,
            cur_freq: cur_val,
            act_freq: act_val,
            max_freq: max_val,
            throttle_reasons: throttle,
        })
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
}

impl DrmDriverXe
{
    pub fn new(qmd: &DrmDeviceInfo) -> Result<Rc<RefCell<dyn DrmDriver>>>
    {
        let file = File::open(qmd.drm_minors[0].devnode.clone())?;
        let fd = file.as_raw_fd();

        let mut cpath = String::from("/sys/class/drm/");
        let card = Path::new(&qmd.drm_minors[0].devnode)
            .file_name().unwrap().to_str().unwrap();
        cpath.push_str(card);

        // TODO: handle more than one tile & gt
        Ok(Rc::new(RefCell::new(DrmDriverXe {
            dn_file: file,
            dn_fd: fd,
            freqs_dir: Path::new(&cpath).join("device/tile0/gt0/freq0"),
            throttle_dir: Path::new(&cpath)
                .join("device/tile0/gt0/freq0/throttle"),
            dev_type: None,
            freq_limits: None,
        })))
    }
}
