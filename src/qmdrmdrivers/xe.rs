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

use crate::qmdrmdrivers::QmDrmDriver;
use crate::qmdrmdevices::{
    QmDrmDeviceType, QmDrmDeviceFreqs,
    QmDrmDeviceMemInfo, QmDrmDeviceInfo
};
use crate::qmdrmfdinfo::QmDrmMemRegion;
use crate::qmdrmclients::QmDrmClientMemInfo;


//
// code modified from rust-bindgen 0.69.4 ran on part of kernel's xe_drm.h
//
#[repr(C)]
#[derive(Default)]
struct __IncompleteArrayField<T>(::std::marker::PhantomData<T>, [T; 0]);
impl<T> __IncompleteArrayField<T> {
    #[inline]
    const fn new() -> Self {
        __IncompleteArrayField(::std::marker::PhantomData, [])
    }
    #[inline]
    fn as_ptr(&self) -> *const T {
        self as *const _ as *const T
    }
    #[inline]
    fn as_mut_ptr(&mut self) -> *mut T {
        self as *mut _ as *mut T
    }
    #[inline]
    unsafe fn as_slice(&self, len: usize) -> &[T] {
        ::std::slice::from_raw_parts(self.as_ptr(), len)
    }
    #[inline]
    unsafe fn as_mut_slice(&mut self, len: usize) -> &mut [T] {
        ::std::slice::from_raw_parts_mut(self.as_mut_ptr(), len)
    }
}
impl<T> ::std::fmt::Debug for __IncompleteArrayField<T> {
    fn fmt(&self, fmt: &mut ::std::fmt::Formatter<'_>) -> ::std::fmt::Result {
        fmt.write_str("__IncompleteArrayField")
    }
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

// generated manually (use nix crate if more are needed)
const DRM_IOCTL_XE_DEVICE_QUERY: u64 = 3223872576;

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
const DRM_XE_QUERY_CONFIG_FLAG_HAS_VRAM: u32 = 1;
const DRM_XE_QUERY_CONFIG_MIN_ALIGNMENT: u32 = 2;
const DRM_XE_QUERY_CONFIG_VA_BITS: u32 = 3;
const DRM_XE_QUERY_CONFIG_MAX_EXEC_QUEUE_PRIORITY: u32 = 4;

const DRM_XE_DEVICE_QUERY_CONFIG: u32 = 2;

#[derive(Debug)]
pub struct QmDrmDriverXe
{
    dn_file: File,
    dn_fd: RawFd,
    freqs_dir: PathBuf,
    dev_type: Option<QmDrmDeviceType>,
}

impl QmDrmDriver for QmDrmDriverXe
{
    fn name(&self) -> &str
    {
        "xe"
    }

    fn dev_type(&mut self) -> Result<QmDrmDeviceType>
    {
        if let Some(dt) = &self.dev_type {
            return Ok(dt.clone());
        }

        let mut dq = drm_xe_device_query {
            extensions: 0,
            size: 0,
            data: 0,
            query: DRM_XE_DEVICE_QUERY_CONFIG,
            reserved: [0, 0],
        };

        let res = unsafe {
            libc::ioctl(self.dn_fd, DRM_IOCTL_XE_DEVICE_QUERY, &dq) };
        if res < 0 {
            return Err(io::Error::last_os_error().into());
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
            libc::ioctl(self.dn_fd, DRM_IOCTL_XE_DEVICE_QUERY, &dq) };
        if res < 0 {
            unsafe { alloc::dealloc(qcfg as *mut u8, layout); }
            return Err(io::Error::last_os_error().into());
        }

        let cfg = unsafe { (*qcfg).info.as_slice((*qcfg).num_params as usize) };
        let flags = cfg[DRM_XE_QUERY_CONFIG_FLAGS as usize];

        let qmdt = if flags & DRM_XE_QUERY_CONFIG_FLAG_HAS_VRAM as u64 > 0 {
            QmDrmDeviceType::Discrete
        } else {
            QmDrmDeviceType::Integrated
        };

        unsafe { alloc::dealloc(qcfg as *mut u8, layout); }

        self.dev_type = Some(qmdt.clone());
        Ok(qmdt)
    }

    fn mem_info(&mut self) -> Result<QmDrmDeviceMemInfo>
    {
        let mut dq = drm_xe_device_query {
            extensions: 0,
            size: 0,
            data: 0,
            query: DRM_XE_DEVICE_QUERY_MEM_REGIONS,
            reserved: [0, 0],
        };

        let res = unsafe {
            libc::ioctl(self.dn_fd, DRM_IOCTL_XE_DEVICE_QUERY, &dq) };
        if res < 0 {
            return Err(io::Error::last_os_error().into());
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
            libc::ioctl(self.dn_fd, DRM_IOCTL_XE_DEVICE_QUERY, &dq) };
        if res < 0 {
            unsafe { alloc::dealloc(qmrg as *mut u8, layout); }
            return Err(io::Error::last_os_error().into());
        }

        let mrgs = unsafe {
            (*qmrg).mem_regions.as_slice((*qmrg).num_mem_regions as usize) };

        let mut qmdmi = QmDrmDeviceMemInfo::new();
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

    fn freqs(&mut self) -> Result<QmDrmDeviceFreqs>
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

        Ok(QmDrmDeviceFreqs {
            min_freq: min_val,
            cur_freq: cur_val,
            act_freq: act_val,
            max_freq: max_val,
        })
    }

    fn client_mem_info(&mut self,
        mem_regs: &HashMap<String, QmDrmMemRegion>) -> Result<QmDrmClientMemInfo>
    {
        let mut cmi = QmDrmClientMemInfo::new();

        for mr in mem_regs.values() {
            if mr.name.starts_with("system") || mr.name.starts_with("gtt") {
                cmi.smem_used += mr.total;
                cmi.smem_rss += mr.resident;
            } else if mr.name.starts_with("vram") {
                cmi.vram_used += mr.total;
                cmi.vram_rss += mr.resident;
            } else if mr.name.starts_with("stolen") {
                if self.dev_type()? == QmDrmDeviceType::Discrete {
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

impl QmDrmDriverXe
{
    pub fn new(qmd: &QmDrmDeviceInfo) -> Result<Rc<RefCell<dyn QmDrmDriver>>>
    {
        let file = File::open(qmd.drm_minors[0].devnode.clone())?;
        let fd = file.as_raw_fd();

        let mut cpath = String::from("/sys/class/drm/");
        let card = Path::new(&qmd.drm_minors[0].devnode)
            .file_name().unwrap().to_str().unwrap();
        cpath.push_str(card);

        // TODO: handle more than one tile & gt
        Ok(Rc::new(RefCell::new(QmDrmDriverXe {
            dn_file: file,
            dn_fd: fd,
            freqs_dir: Path::new(&cpath).join("device/tile0/gt0/freq0"),
            dev_type: None,
        })))
    }
}
