use std::path::{Path, PathBuf};
use std::os::linux::fs::MetadataExt;
use std::fs;

use anyhow::Result;
use libc;

use crate::qmdevice::QmDevice;


#[derive(Debug)]
pub struct QmDrmEngine
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

impl Default for QmDrmEngine
{
    fn default() -> QmDrmEngine
    {
        QmDrmEngine {
            name: String::from(""),
            capacity: 1,
            time: 0,
            cycles: 0,
            total_cycles: 0,
        }
    }
}

impl QmDrmEngine
{
    pub fn new(eng_name: &str) -> QmDrmEngine
    {
        QmDrmEngine {
            name: eng_name.to_string(),
            ..Default::default()
        }
    }
}

#[derive(Debug)]
pub struct QmDrmMemRegion
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

impl Default for QmDrmMemRegion
{
    fn default() -> QmDrmMemRegion
    {
        QmDrmMemRegion {
            name: String::from(""),
            total: 0,
            shared: 0,
            resident: 0,
            purgeable: 0,
            active: 0,
        }
    }
}

impl QmDrmMemRegion
{
    pub fn new(memreg_name: &str) -> QmDrmMemRegion
    {
        QmDrmMemRegion {
            name: memreg_name.to_string(),
            ..Default::default()
        }
    }
}

#[derive(Debug)]
pub struct QmDrmFdinfo<'b>
{
    pub qmdev: Option<&'b QmDevice>,
    pub path: PathBuf,
    pub id: u32,
    pub engines: Vec<QmDrmEngine>,
    pub mem_regions: Vec<QmDrmMemRegion>,
}

impl Default for QmDrmFdinfo<'_>
{
    fn default() -> QmDrmFdinfo<'static>
    {
        QmDrmFdinfo {
            qmdev: None,
            path: PathBuf::new(),
            id: 0,
            engines: Vec::new(),
            mem_regions: Vec::new(),
        }
    }
}

impl QmDrmFdinfo<'_>
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

    fn find_engine(&mut self, eng_name: &str) -> Option<&mut QmDrmEngine>
    {
        for eng in &mut self.engines {
            if eng.name == eng_name {
                return Some(eng);
            }
        }
        None
    }

    fn update_engine(&mut self, kv_type: EngKvType, eng_name: &str, val: &str) -> Result<()>
    {
        let eng: &mut QmDrmEngine;
        if let Some(res) = self.find_engine(eng_name) {
            eng = res;
        } else {
            self.engines.push(QmDrmEngine::new(eng_name));
            let last = self.engines.len()-1;
            eng = &mut self.engines[last];
        }

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

    fn find_mem_region(&mut self, mr_name: &str) -> Option<&mut QmDrmMemRegion>
    {
        for mr in &mut self.mem_regions {
            if mr.name == mr_name {
                return Some(mr);
            }
        }
        None
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
        let mrg: &mut QmDrmMemRegion;
        if let Some(res) = self.find_mem_region(mr_name) {
            mrg = res;
        } else {
            self.mem_regions.push(QmDrmMemRegion::new(mr_name));
            let last = self.mem_regions.len()-1;
            mrg = &mut self.mem_regions[last];
        }

        let dt: Vec<&str> = val.split_whitespace().collect();
        let nr: u64 = dt[0].parse()?;
        let mut mul: u64 = 1;
        if dt.len() == 2  {
            mul = QmDrmFdinfo::mul_from_unit(dt[1]);
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

    pub fn from_drm_fdinfo<'a,'b>(fdinfo: &'a PathBuf, qmd: &'b QmDevice) -> Result<QmDrmFdinfo<'b>>
    {
        let lines: Vec<_> = fs::read_to_string(fdinfo)?
            .lines()
            .map(String::from)
            .collect();

        let mut info = QmDrmFdinfo {
            qmdev: Some(qmd),
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

            // TODO ?: check if pdev and driver match ones in qmd

            if k.starts_with("drm-client-id") {
                info.id = v.parse()?;
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
