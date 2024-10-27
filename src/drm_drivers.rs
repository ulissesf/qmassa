use core::fmt::Debug;
use std::collections::HashMap;
use std::cell::RefCell;
use std::rc::Rc;

use anyhow::Result;

use crate::drm_devices::{
    DrmDeviceType, DrmDeviceFreqs,
    DrmDeviceMemInfo, DrmDeviceInfo
};
use crate::drm_fdinfo::DrmMemRegion;
use crate::drm_clients::DrmClientMemInfo;

mod helpers;
mod xe;
use xe::DrmDriverXe;
mod i915;
use i915::DrmDriveri915;


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

    fn freqs(&mut self) -> Result<DrmDeviceFreqs>
    {
        Ok(DrmDeviceFreqs::new())
    }

    fn mem_info(&mut self) -> Result<DrmDeviceMemInfo>
    {
        Ok(DrmDeviceMemInfo::new())
    }

    fn client_mem_info(&mut self,
        _mem_regs: &HashMap<String, DrmMemRegion>) -> Result<DrmClientMemInfo>
    {
        Ok(DrmClientMemInfo::new())
    }
}

impl Debug for dyn DrmDriver
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "DrmDriver({:?})", self.name())
    }
}

pub fn driver_from(
    qmd: &DrmDeviceInfo) -> Result<Option<Rc<RefCell<dyn DrmDriver>>>>
{
    let drvs: &[(&str,
        fn(&DrmDeviceInfo) -> Result<Rc<RefCell<dyn DrmDriver>>>)] = &[
        ("xe", DrmDriverXe::new),
        ("i915", DrmDriveri915::new),
    ];

    for (dn, drv_newfunc) in drvs {
        if *dn == qmd.drv_name {
            let drv = drv_newfunc(qmd)?;
            return Ok(Some(drv));
        }
    }

    Ok(None)
}