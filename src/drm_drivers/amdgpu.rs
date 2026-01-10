#![allow(dead_code)]
#![allow(non_camel_case_types)]
#![allow(non_upper_case_globals)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::fs::{self, File};
use std::os::fd::{RawFd, AsRawFd};
use std::cell::RefCell;
use std::rc::Rc;
use std::mem;
use std::io;

use anyhow::Result;
use libc::Ioctl;
use log::{debug, warn};

use crate::drm_drivers::DrmDriver;
use crate::drm_drivers::helpers::{drm_iow, drm_ioctl};
use crate::hwmon::Hwmon;
use crate::drm_devices::{
    DrmDeviceType, DrmDeviceFreqLimits, DrmDeviceFreqs, DrmDevicePower,
    DrmDeviceMemInfo, DrmDeviceTemperature, DrmDeviceFan, DrmDeviceInfo
};
use crate::drm_fdinfo::DrmMemRegion;
use crate::drm_clients::DrmClientMemInfo;


// rust-bindgen 0.69.5 on Linux kernel v6.12 uapi amdgpu_drm.h + changes
#[repr(C)]
#[derive(Debug, Copy, Clone)]
struct drm_amdgpu_info_device {
    device_id: u32,
    chip_rev: u32,
    external_rev: u32,
    pci_rev: u32,
    family: u32,
    num_shader_engines: u32,
    num_shader_arrays_per_engine: u32,
    gpu_counter_freq: u32,
    max_engine_clock: u64,
    max_memory_clock: u64,
    cu_active_number: u32,
    cu_ao_mask: u32,
    cu_bitmap: [[u32; 4usize]; 4usize],
    enabled_rb_pipes_mask: u32,
    num_rb_pipes: u32,
    num_hw_gfx_contexts: u32,
    pcie_gen: u32,
    ids_flags: u64,
    virtual_address_offset: u64,
    virtual_address_max: u64,
    virtual_address_alignment: u32,
    pte_fragment_size: u32,
    gart_page_size: u32,
    ce_ram_size: u32,
    vram_type: u32,
    vram_bit_width: u32,
    vce_harvest_config: u32,
    gc_double_offchip_lds_buf: u32,
    prim_buf_gpu_addr: u64,
    pos_buf_gpu_addr: u64,
    cntl_sb_buf_gpu_addr: u64,
    param_buf_gpu_addr: u64,
    prim_buf_size: u32,
    pos_buf_size: u32,
    cntl_sb_buf_size: u32,
    param_buf_size: u32,
    wave_front_size: u32,
    num_shader_visible_vgprs: u32,
    num_cu_per_sh: u32,
    num_tcc_blocks: u32,
    gs_vgt_table_depth: u32,
    gs_prim_buffer_depth: u32,
    max_gs_waves_per_vgt: u32,
    pcie_num_lanes: u32,
    cu_ao_bitmap: [[u32; 4usize]; 4usize],
    high_va_offset: u64,
    high_va_max: u64,
    pa_sc_tile_steering_override: u32,
    tcc_disabled_mask: u64,
    min_engine_clock: u64,
    min_memory_clock: u64,
    tcp_cache_size: u32,
    num_sqc_per_wgp: u32,
    sqc_data_cache_size: u32,
    sqc_inst_cache_size: u32,
    gl1c_cache_size: u32,
    gl2c_cache_size: u32,
    mall_size: u64,
    enabled_rb_pipes_mask_hi: u32,
    shadow_size: u32,
    shadow_alignment: u32,
    csa_size: u32,
    csa_alignment: u32,
}

impl drm_amdgpu_info_device
{
    fn new() -> drm_amdgpu_info_device
    {
        drm_amdgpu_info_device {
            device_id: 0,
            chip_rev: 0,
            external_rev: 0,
            pci_rev: 0,
            family: 0,
            num_shader_engines: 0,
            num_shader_arrays_per_engine: 0,
            gpu_counter_freq: 0,
            max_engine_clock: 0,
            max_memory_clock: 0,
            cu_active_number: 0,
            cu_ao_mask: 0,
            cu_bitmap: [[0; 4usize]; 4usize],
            enabled_rb_pipes_mask: 0,
            num_rb_pipes: 0,
            num_hw_gfx_contexts: 0,
            pcie_gen: 0,
            ids_flags: 0,
            virtual_address_offset: 0,
            virtual_address_max: 0,
            virtual_address_alignment: 0,
            pte_fragment_size: 0,
            gart_page_size: 0,
            ce_ram_size: 0,
            vram_type: 0,
            vram_bit_width: 0,
            vce_harvest_config: 0,
            gc_double_offchip_lds_buf: 0,
            prim_buf_gpu_addr: 0,
            pos_buf_gpu_addr: 0,
            cntl_sb_buf_gpu_addr: 0,
            param_buf_gpu_addr: 0,
            prim_buf_size: 0,
            pos_buf_size: 0,
            cntl_sb_buf_size: 0,
            param_buf_size: 0,
            wave_front_size: 0,
            num_shader_visible_vgprs: 0,
            num_cu_per_sh: 0,
            num_tcc_blocks: 0,
            gs_vgt_table_depth: 0,
            gs_prim_buffer_depth: 0,
            max_gs_waves_per_vgt: 0,
            pcie_num_lanes: 0,
            cu_ao_bitmap: [[0; 4usize]; 4usize],
            high_va_offset: 0,
            high_va_max: 0,
            pa_sc_tile_steering_override: 0,
            tcc_disabled_mask: 0,
            min_engine_clock: 0,
            min_memory_clock: 0,
            tcp_cache_size: 0,
            num_sqc_per_wgp: 0,
            sqc_data_cache_size: 0,
            sqc_inst_cache_size: 0,
            gl1c_cache_size: 0,
            gl2c_cache_size: 0,
            mall_size: 0,
            enabled_rb_pipes_mask_hi: 0,
            shadow_size: 0,
            shadow_alignment: 0,
            csa_size: 0,
            csa_alignment: 0,
        }
    }
}

const AMDGPU_IDS_FLAGS_FUSION: u64 = 0x1;

const AMDGPU_INFO_DEV_INFO: u32 = 0x16;

#[repr(C)]
#[derive(Debug, Copy, Clone)]
struct drm_amdgpu_heap_info {
    total_heap_size: u64,
    usable_heap_size: u64,
    heap_usage: u64,
    max_allocation: u64,
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
struct drm_amdgpu_memory_info {
    vram: drm_amdgpu_heap_info,
    cpu_accessible_vram: drm_amdgpu_heap_info,
    gtt: drm_amdgpu_heap_info,
}

impl drm_amdgpu_memory_info
{
    fn new() -> drm_amdgpu_memory_info
    {
        let zhi = drm_amdgpu_heap_info {
            total_heap_size: 0,
            usable_heap_size: 0,
            heap_usage: 0,
            max_allocation: 0,
        };

        drm_amdgpu_memory_info {
            vram: zhi,
            cpu_accessible_vram: zhi,
            gtt: zhi,
        }
    }
}

const AMDGPU_INFO_MEMORY: u32 = 0x19;

#[repr(C)]
#[derive(Debug, Copy, Clone)]
struct drm_amdgpu_info_mode_crtc {
    id: u32,
    _pad: u32,
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
struct drm_amdgpu_info_query_hw_ip {
    type_: u32,
    ip_instance: u32,
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
struct drm_amdgpu_info_read_mmr_reg {
    dword_offset: u32,
    count: u32,
    instance: u32,
    flags: u32,
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
struct drm_amdgpu_query_fw {
   fw_type: u32,
   ip_instance: u32,
   index: u32,
   _pad: u32,
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
struct drm_amdgpu_info_vbios_info {
    type_: u32,
    offset: u32,
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
struct drm_amdgpu_info_sensor_info {
    type_: u32,
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
struct drm_amdgpu_info_video_cap {
    type_: u32,
}

#[repr(C)]
#[derive(Copy, Clone)]
union drm_amdgpu_info_extra {
    mode_crtc: drm_amdgpu_info_mode_crtc,
    query_hw_ip: drm_amdgpu_info_query_hw_ip,
    read_mmr_reg: drm_amdgpu_info_read_mmr_reg,
    query_fw: drm_amdgpu_query_fw,
    vbios_info: drm_amdgpu_info_vbios_info,
    sensor_info: drm_amdgpu_info_sensor_info,
    video_cap: drm_amdgpu_info_video_cap,
}

#[repr(C)]
#[derive(Copy, Clone)]
struct drm_amdgpu_info {
    return_pointer: u64,
    return_size: u32,
    query: u32,
    extra: drm_amdgpu_info_extra,
}

impl drm_amdgpu_info
{
    fn new() -> drm_amdgpu_info
    {
        // only init the largest union member
        let query_fw = drm_amdgpu_query_fw {
            fw_type: 0, ip_instance: 0, index: 0, _pad: 0 };
        let einfo = drm_amdgpu_info_extra {
            query_fw,
        };

        drm_amdgpu_info {
            return_pointer: 0,
            return_size: 0,
            query: 0,
            extra: einfo,
        }
    }
}

const DRM_AMDGPU_INFO: Ioctl = 0x05;
const DRM_IOCTL_AMDGPU_INFO: Ioctl = drm_iow!(DRM_AMDGPU_INFO,
    mem::size_of::<drm_amdgpu_info>());

#[derive(Debug)]
pub struct DrmDriverAmdgpu
{
    _dn_file: File,
    dn_fd: RawFd,
    freqs_dir: PathBuf,
    dev_type: Option<DrmDeviceType>,
    freq_limits: Option<DrmDeviceFreqLimits>,
    hwmon: Option<Hwmon>,
    sensor: String,
}

impl DrmDriver for DrmDriverAmdgpu
{
    fn name(&self) -> &str
    {
        "amdgpu"
    }

    fn dev_type(&mut self) -> Result<DrmDeviceType>
    {
        if let Some(dt) = &self.dev_type {
            return Ok(dt.clone());
        }

        let mut qid = drm_amdgpu_info_device::new();
        let qid_ptr: *mut drm_amdgpu_info_device = &mut qid;

        self.amdgpu_info_ioctl(AMDGPU_INFO_DEV_INFO,
            qid_ptr as u64, mem::size_of::<drm_amdgpu_info_device>() as u32)?;

        let qmdt = if qid.ids_flags & AMDGPU_IDS_FLAGS_FUSION > 0 {
            DrmDeviceType::Integrated
        } else {
            DrmDeviceType::Discrete
        };

        self.dev_type = Some(qmdt.clone());
        Ok(qmdt)
    }

    fn mem_info(&mut self) -> Result<DrmDeviceMemInfo>
    {
        let mut qim = drm_amdgpu_memory_info::new();
        let qim_ptr: *mut drm_amdgpu_memory_info = &mut qim;

        self.amdgpu_info_ioctl(
            AMDGPU_INFO_MEMORY,
            qim_ptr as u64,
            mem::size_of::<drm_amdgpu_memory_info>() as u32)?;

        Ok(DrmDeviceMemInfo {
            smem_total: qim.gtt.total_heap_size,
            smem_used: qim.gtt.heap_usage,
            vram_total: qim.vram.total_heap_size,
            vram_used: qim.vram.heap_usage,
        })
    }

    fn freq_limits(&mut self) -> Result<Vec<DrmDeviceFreqLimits>>
    {
        if let Some(fls) = &self.freq_limits {
            return Ok(vec![fls.clone(),]);
        }

        // TODO: get non-gfx freq limits
        let fpath = self.freqs_dir.join("pp_dpm_sclk");
        let sclk_str = fs::read_to_string(&fpath)?;

        let mut fls = DrmDeviceFreqLimits::new();
        for line in sclk_str.lines() {
            let kv: Vec<_> = line.split(':').map(|it| it.trim()).collect();
            if kv.len() < 2 {
                warn!("Wrong line [{:?}] from {:?}, aborting.", line, fpath);
                return Ok(vec![fls,]);
            }
            let k: u32 = kv[0].parse()?;
            if k == 1 {
                continue;
            }

            let mut v = kv[1];
            if v.ends_with(" *") {
                v = &kv[1][..kv[1].len() - 2];
            }
            if !v.ends_with("Mhz") {
                warn!("Wrong line [{:?}] from {:?}, aborting.", line, fpath);
                return Ok(vec![fls,]);
            }
            v = &v[..v.len() - 3];

            if k == 0 {
                // FIXME: actual freq can go lower then minimum in sysfs file
                fls.minimum = 0;
            } else if k == 2 {
                fls.maximum = v.parse()?;
                // FIXME: actual freq can go much higher than max in sysfs file
                fls.maximum += fls.maximum / 2;
            } else {
                fls.minimum = 0;
                fls.maximum = 0;
                warn!("Wrong line [{:?}] from {:?}, aborting.", line, fpath);
                return Ok(vec![fls,]);
            }
        }

        self.freq_limits = Some(fls.clone());
        Ok(vec![fls,])
    }

    fn freqs(&mut self) -> Result<Vec<DrmDeviceFreqs>>
    {
        // TODO: get non-gfx freqs
        let fpath = self.freqs_dir.join("pp_dpm_sclk");
        let sclk_str = fs::read_to_string(&fpath)?;

        let mut freqs = DrmDeviceFreqs::new();
        for line in sclk_str.lines() {
            let tl = line.trim();
            if !tl.ends_with("Mhz *") {
                continue;
            }
            let kv: Vec<_> = tl[..tl.len() - 5]
                .split(':').map(|it| it.trim()).collect();

            freqs.act_freq = kv[1].parse()?;
        }

        Ok(vec![freqs,])
    }

    fn power(&mut self) -> Result<DrmDevicePower>
    {
        if self.hwmon.is_none() || self.sensor.is_empty() {
            // TODO: need to add integrated support, only hwmon/discrete now
            return Ok(DrmDevicePower::new());
        }
        let hwmon = self.hwmon.as_ref().unwrap();

        let val = hwmon.read_sensor(&self.sensor, "average")?;

        Ok(DrmDevicePower {
            gpu_cur_power: val as f64 / 1000000.0,
            pkg_cur_power: 0.0,
        })
    }

    fn client_mem_info(&mut self,
        mem_regs: &HashMap<String, DrmMemRegion>) -> Result<DrmClientMemInfo>
    {
        let mut cmi = DrmClientMemInfo::new();

        for mr in mem_regs.values() {
            if mr.name.starts_with("cpu") || mr.name.starts_with("gtt") {
                cmi.smem_used += mr.total;
                cmi.smem_rss += mr.resident;
            } else if mr.name.starts_with("vram") {
                cmi.vram_used += mr.total;
                cmi.vram_rss += mr.resident;
            } else {
                warn!("Unknown amdgpu memory region: {:?}, skpping it.",
                    mr.name);
                continue;
            }
        }

        Ok(cmi)
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

impl DrmDriverAmdgpu
{
    fn amdgpu_info_ioctl(&self,
        query_id: u32, data: u64, size: u32) -> Result<()>
    {
        let mut qi = drm_amdgpu_info::new();

        qi.query = query_id;
        qi.return_pointer = data;
        qi.return_size = size;

        let res = drm_ioctl!(self.dn_fd, DRM_IOCTL_AMDGPU_INFO, &mut qi);
        if res < 0 {
            return Err(io::Error::last_os_error().into());
        }

        Ok(())
    }

    pub fn new(qmd: &DrmDeviceInfo,
        _opts: Option<&Vec<&str>>) -> Result<Rc<RefCell<dyn DrmDriver>>>
    {
        let mut dn: &str = "";
        for c in qmd.dev_nodes.iter() {
            if c.devnode.contains("render") {
                dn = &c.devnode;
                break;
            }
        }

        let file = File::open(dn)?;
        let fd = file.as_raw_fd();

        let mut cpath = String::from("/sys/class/drm/");
        let card = Path::new(dn).file_name().unwrap().to_str().unwrap();
        cpath.push_str(card);

        let mut amdgpu = DrmDriverAmdgpu {
            _dn_file: file,
            dn_fd: fd,
            freqs_dir: Path::new(&cpath).join("device"),
            dev_type: None,
            freq_limits: None,
            hwmon: None,
            sensor: String::new(),
        };

        let dtype = amdgpu.dev_type()?;
        amdgpu.freq_limits()?;

        if dtype.is_discrete() {
            let hwmon_res = Hwmon::from(
                Path::new(&cpath).join("device/hwmon"));
            if let Ok(hwmon) = hwmon_res {
                let hwmon_ref = hwmon.as_ref().unwrap();
                let plist = hwmon_ref.sensors("power");
                for s in plist.iter() {
                    if s.has_item("average") {
                        amdgpu.sensor = s.stype.clone();
                    }
                }
                amdgpu.hwmon = hwmon;
            } else {
                debug!("{}: ERR: no Hwmon support on dGPU: {:?}",
                    &qmd.pci_dev, hwmon_res);
            }
        }

        Ok(Rc::new(RefCell::new(amdgpu)))
    }
}
