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

use crate::qmdrmdrivers::DrmDriver;
use crate::qmdrmdrivers::qmhelpers::__IncompleteArrayField;
use crate::qmdrmdevices::{
    DrmDeviceType, DrmDeviceFreqs, DrmDeviceThrottleReasons,
    DrmDeviceMemInfo, DrmDeviceInfo
};
use crate::qmdrmfdinfo::DrmMemRegion;
use crate::qmdrmclients::DrmClientMemInfo;


//
// code modified from rust-bindgen 0.69.4 run on part of kernel's i915_drm.h
//
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

// generated manually (use nix crate if more are needed)
const DRM_IOCTL_I915_QUERY: u64 = 3222299769;

#[derive(Debug)]
pub struct DrmDriveri915
{
    dn_file: File,
    dn_fd: RawFd,
    freqs_dir: PathBuf,
    gt_dir: PathBuf,
    dev_type: Option<DrmDeviceType>,
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

        let res = unsafe {
            libc::ioctl(self.dn_fd, DRM_IOCTL_I915_QUERY, &mut dq) };
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

        let res = unsafe {
            libc::ioctl(self.dn_fd, DRM_IOCTL_I915_QUERY, &mut dq) };
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

    fn freqs(&mut self) -> Result<DrmDeviceFreqs>
    {
        let fpath = self.freqs_dir.join("gt_min_freq_mhz");
        let fstr = fs::read_to_string(&fpath)?;
        let min_val: u64 = fstr.trim_end().parse()?;

        let fpath = self.freqs_dir.join("gt_cur_freq_mhz");
        let fstr = fs::read_to_string(&fpath)?;
        let cur_val: u64 = fstr.trim_end().parse()?;

        let fpath = self.freqs_dir.join("gt_act_freq_mhz");
        let fstr = fs::read_to_string(&fpath)?;
        let act_val: u64 = fstr.trim_end().parse()?;

        let fpath = self.freqs_dir.join("gt_max_freq_mhz");
        let fstr = fs::read_to_string(&fpath)?;
        let max_val: u64 = fstr.trim_end().parse()?;

        let fpath = self.gt_dir.join("throttle_reason_pl1");
        let pl1 = fs::read_to_string(&fpath)?.trim() == "1";

        let fpath = self.gt_dir.join("throttle_reason_pl2");
        let pl2 = fs::read_to_string(&fpath)?.trim() == "1";

        let fpath = self.gt_dir.join("throttle_reason_pl4");
        let pl4 = fs::read_to_string(&fpath)?.trim() == "1";

        let fpath = self.gt_dir.join("throttle_reason_prochot");
        let prochot = fs::read_to_string(&fpath)?.trim() == "1";

        let fpath = self.gt_dir.join("throttle_reason_ratl");
        let ratl = fs::read_to_string(&fpath)?.trim() == "1";

        let fpath = self.gt_dir.join("throttle_reason_thermal");
        let thermal = fs::read_to_string(&fpath)?.trim() == "1";

        let fpath = self.gt_dir.join("throttle_reason_vr_tdc");
        let vr_tdc = fs::read_to_string(&fpath)?.trim() == "1";

        let fpath = self.gt_dir.join("throttle_reason_vr_thermalert");
        let vr_thermalert = fs::read_to_string(&fpath)?.trim() == "1";

        let fpath = self.gt_dir.join("throttle_reason_status");
        let status = fs::read_to_string(&fpath)?.trim() == "1";

        let throttle = DrmDeviceThrottleReasons {
            pl1: pl1,
            pl2: pl2,
            pl4: pl4,
            prochot: prochot,
            ratl: ratl,
            thermal: thermal,
            vr_tdc: vr_tdc,
            vr_thermalert: vr_thermalert,
            status: status,
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
}

impl DrmDriveri915
{
    pub fn new(qmd: &DrmDeviceInfo) -> Result<Rc<RefCell<dyn DrmDriver>>>
    {
        let file = File::open(qmd.drm_minors[0].devnode.clone())?;
        let fd = file.as_raw_fd();

        let mut cpath = String::from("/sys/class/drm/");
        let card = Path::new(&qmd.drm_minors[0].devnode)
            .file_name().unwrap().to_str().unwrap();
        cpath.push_str(card);

        Ok(Rc::new(RefCell::new(DrmDriveri915 {
            dn_file: file,
            dn_fd: fd,
            freqs_dir: PathBuf::from(&cpath),
            gt_dir: Path::new(&cpath).join("gt/gt0"),
            dev_type: None,
        })))
    }
}
