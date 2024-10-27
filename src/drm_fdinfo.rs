use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::os::linux::fs::MetadataExt;
use std::fs;

use anyhow::Result;
use libc;


#[derive(Debug)]
#[allow(dead_code)]
pub struct DrmEngine
{
    pub name: String,
    pub capacity: u32,
    pub time: u64,
    pub cycles: u64,
    pub total_cycles: u64,
}

enum EngKvType
{
    KvTime,
    KvCapacity,
    KvCycles,
    KvTotCycles,
}

impl Default for DrmEngine
{
    fn default() -> DrmEngine
    {
        DrmEngine {
            name: String::from(""),
            capacity: 1,
            time: 0,
            cycles: 0,
            total_cycles: 0,
        }
    }
}

impl DrmEngine
{
    pub fn new(eng_name: &str) -> DrmEngine
    {
        DrmEngine {
            name: eng_name.to_string(),
            ..Default::default()
        }
    }
}

#[derive(Debug, Clone)]
pub struct DrmMemRegion
{
    pub name: String,
    pub total: u64,
    pub shared: u64,
    pub resident: u64,
    pub purgeable: u64,
    pub active: u64,
}

enum MemRegKvType
{
    KvTotal,
    KvShared,
    KvResident,
    KvPurgeable,
    KvActive,
}

impl Default for DrmMemRegion
{
    fn default() -> DrmMemRegion
    {
        DrmMemRegion {
            name: String::from(""),
            total: 0,
            shared: 0,
            resident: 0,
            purgeable: 0,
            active: 0,
        }
    }
}

impl DrmMemRegion
{
    pub fn new(memreg_name: &str) -> DrmMemRegion
    {
        DrmMemRegion {
            name: memreg_name.to_string(),
            ..Default::default()
        }
    }
}

#[derive(Debug)]
pub struct DrmFdinfo
{
    pub pci_dev: String,
    pub drm_minor: u32,
    pub client_id: u32,
    pub path: PathBuf,
    pub engines: HashMap<String, DrmEngine>,
    pub mem_regions: HashMap<String, DrmMemRegion>,
}

impl Default for DrmFdinfo
{
    fn default() -> DrmFdinfo
    {
        DrmFdinfo {
            pci_dev: String::from(""),
            drm_minor: 0,
            client_id: 0,
            path: PathBuf::new(),
            engines: HashMap::new(),
            mem_regions: HashMap::new(),
        }
    }
}

impl DrmFdinfo
{
    pub fn is_drm_fd(file: &Path, minor: &mut u32) -> Result<bool>
    {
        let met = fs::metadata(file)?;
        let st_mode = met.st_mode();
        let st_rdev = met.st_rdev();

        // check it's char device and major 226 for DRM device
        let mj: u32;
        let mn: u32;
        unsafe {
            mj = libc::major(st_rdev);
            mn = libc::minor(st_rdev);
        }

        if st_mode & libc::S_IFMT == libc::S_IFCHR && mj == 226 {
            *minor = mn;
            return Ok(true);
        }

        Ok(false)
    }

    fn update_engine(&mut self, kv_type: EngKvType, eng_name: &str, val: &str) -> Result<()>
    {
        let eng: &mut DrmEngine;

        if !self.engines.contains_key(eng_name) {
            self.engines.insert(eng_name.to_string(), DrmEngine::new(eng_name));
        }
        eng = self.engines.get_mut(eng_name).unwrap();

        match kv_type {
            EngKvType::KvCapacity => {
                eng.capacity = val.parse()?;
            },
            EngKvType::KvTime => {
                let dt: Vec<&str> = val.split_whitespace().collect();
                eng.time = dt[0].parse()?;  // ignore dt[1] == "ns"
            },
            EngKvType::KvCycles => {
                eng.cycles = val.parse()?;
            },
            EngKvType::KvTotCycles => {
                eng.total_cycles = val.parse()?;
            },
        }

        Ok(())
    }

    fn mul_from_unit(unit: &str) -> u64
    {
        match unit {
            "KiB" => 1024,
            "MiB" => 1024 * 1024,
            "GiB" => 1024 * 1024 * 1024,
            &_ => 1,
        }
    }

    fn update_mem_region(&mut self, kv_type: MemRegKvType, mr_name: &str, val: &str) -> Result<()>
    {
        let mrg: &mut DrmMemRegion;

        if !self.mem_regions.contains_key(mr_name) {
            self.mem_regions.insert(mr_name.to_string(), DrmMemRegion::new(mr_name));
        }
        mrg = self.mem_regions.get_mut(mr_name).unwrap();

        let dt: Vec<&str> = val.split_whitespace().collect();
        let nr: u64 = dt[0].parse()?;
        let mut mul: u64 = 1;
        if dt.len() == 2  {
            mul = DrmFdinfo::mul_from_unit(dt[1]);
        }

        match kv_type {
            MemRegKvType::KvTotal => {
               mrg.total = nr * mul;
            },
            MemRegKvType::KvShared => {
               mrg.shared = nr * mul;
            },
            MemRegKvType::KvResident => {
               mrg.resident = nr * mul;
            },
            MemRegKvType::KvPurgeable => {
               mrg.purgeable = nr * mul;
            },
            MemRegKvType::KvActive => {
               mrg.active = nr * mul;
            },
        }

        Ok(())
    }

    pub fn from(fdinfo: &PathBuf, d_minor: u32) -> Result<DrmFdinfo>
    {
        let lines: Vec<_> = fs::read_to_string(fdinfo)?
            .lines()
            .map(String::from)
            .collect();

        let mut info = DrmFdinfo {
            drm_minor: d_minor,
            path: PathBuf::from(fdinfo),
            ..Default::default()
        };
        for l in lines {
            let kv: Vec<&str> = l.split(":\t").collect();
            let k = kv[0];
            let v = kv[1];

            if !k.starts_with("drm-") {
                continue;
            }

            if k.starts_with("drm-pdev") {
                info.pci_dev.push_str(v);
            } else if k.starts_with("drm-client-id") {
                info.client_id = v.parse()?;
            } else if k.starts_with("drm-engine-capacity-") {
                let en = &k["drm-engine-capacity-".len()..];
                info.update_engine(EngKvType::KvCapacity, en, v)?;
            } else if k.starts_with("drm-engine-") {
                let en = &k["drm-engine-".len()..];
                info.update_engine(EngKvType::KvTime, en, v)?;
            } else if k.starts_with("drm-cycles-") {
                let en = &k["drm-cycles-".len()..];
                info.update_engine(EngKvType::KvCycles, en, v)?;
            } else if k.starts_with("drm-total-cycles-") {
                let en = &k["drm-total-cycles-".len()..];
                info.update_engine(EngKvType::KvTotCycles, en, v)?;
            } else if k.starts_with("drm-total-") {
                let mrn = &k["drm-total-".len()..];
                info.update_mem_region(MemRegKvType::KvTotal, mrn, v)?;
            } else if k.starts_with("drm-shared-") {
                let mrn = &k["drm-shared-".len()..];
                info.update_mem_region(MemRegKvType::KvShared, mrn, v)?;
            } else if k.starts_with("drm-resident-") {
                let mrn = &k["drm-resident-".len()..];
                info.update_mem_region(MemRegKvType::KvResident, mrn, v)?;
            } else if k.starts_with("drm-purgeable-") {
                let mrn = &k["drm-purgeable-".len()..];
                info.update_mem_region(MemRegKvType::KvPurgeable, mrn, v)?;
            } else if k.starts_with("drm-active-") {
                let mrn = &k["drm-active-".len()..];
                info.update_mem_region(MemRegKvType::KvActive, mrn, v)?;
            }
        }

        Ok(info)
    }
}
