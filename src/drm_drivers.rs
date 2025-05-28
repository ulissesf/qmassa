use core::fmt::Debug;
use std::collections::HashMap;
use std::cell::RefCell;
use std::rc::Rc;

use anyhow::Result;

use crate::drm_devices::{
    DrmDeviceType, DrmDeviceFreqLimits, DrmDeviceFreqs, DrmDevicePower,
    DrmDeviceMemInfo, DrmDeviceTemperature, DrmDeviceFan, DrmDeviceInfo
};
use crate::drm_fdinfo::DrmMemRegion;
use crate::drm_clients::DrmClientMemInfo;

mod helpers;
mod intel_power;
mod xe;
use xe::DrmDriverXe;
mod i915;
use i915::DrmDriveri915;
mod amdgpu;
use amdgpu::DrmDriverAmdgpu;


pub trait DrmDriver
{
    fn name(&self) -> &str
    {
        "(not implemented)"
    }

    fn dev_type(&mut self) -> Result<DrmDeviceType>
    {
        Ok(DrmDeviceType::Unknown)
    }

    fn freq_limits(&mut self) -> Result<Vec<DrmDeviceFreqLimits>>
    {
        Ok(vec![DrmDeviceFreqLimits::new(),])
    }

    fn freqs(&mut self) -> Result<Vec<DrmDeviceFreqs>>
    {
        Ok(vec![DrmDeviceFreqs::new(),])
    }

    fn power(&mut self) -> Result<DrmDevicePower>
    {
        Ok(DrmDevicePower::new())
    }

    fn mem_info(&mut self) -> Result<DrmDeviceMemInfo>
    {
        Ok(DrmDeviceMemInfo::new())
    }

    fn client_mem_info(&mut self,
        mem_regs: &HashMap<String, DrmMemRegion>) -> Result<DrmClientMemInfo>
    {
        if let Some(mrg) = mem_regs.get("memory") {
            let mut cmi = DrmClientMemInfo::new();
            cmi.smem_rss = mrg.resident;
            cmi.smem_used = mrg.total;
            return Ok(cmi);
        }

        Ok(DrmClientMemInfo::new())
    }

    fn engs_utilization(&mut self) -> Result<HashMap<String, f64>>
    {
        Ok(HashMap::new())
    }

    fn temps(&mut self) -> Result<Vec<DrmDeviceTemperature>>
    {
        Ok(Vec::new())
    }

    fn fans(&mut self) -> Result<Vec<DrmDeviceFan>>
    {
        Ok(Vec::new())
    }
}

impl Debug for dyn DrmDriver
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "DrmDriver({:?})", self.name())
    }
}

pub fn driver_from(qmd: &DrmDeviceInfo,
    opts: Option<&Vec<&str>>) -> Result<Option<Rc<RefCell<dyn DrmDriver>>>>
{
    let drvs: &[(&str, fn(&DrmDeviceInfo, Option<&Vec<&str>>) -> Result<Rc<RefCell<dyn DrmDriver>>>)] = &[
        ("xe", DrmDriverXe::new),
        ("i915", DrmDriveri915::new),
        ("amdgpu", DrmDriverAmdgpu::new),
    ];

    for (dn, drv_newfunc) in drvs {
        if *dn == qmd.drv_name {
            let drv = drv_newfunc(qmd, opts)?;
            return Ok(Some(drv));
        }
    }

    Ok(None)
}
