use core::fmt::Debug;
use std::collections::HashMap;
use std::cell::RefCell;
use std::rc::Rc;

use anyhow::Result;

use crate::qmdrmdevices::{
    QmDrmDeviceType, QmDrmDeviceFreqs,
    QmDrmDeviceMemInfo, QmDrmDeviceInfo
};
use crate::qmdrmfdinfo::QmDrmMemRegion;
use crate::qmdrmclients::QmDrmClientMemInfo;

mod xe;
use xe::QmDrmDriverXe;
mod i915;
use i915::QmDrmDriveri915;


pub trait QmDrmDriver
{
    fn name(&self) -> &str
    {
        "(not implemented)"
    }

    fn dev_type(&mut self) -> Result<QmDrmDeviceType>
    {
        Ok(QmDrmDeviceType::Integrated)
    }

    fn freqs(&mut self) -> Result<QmDrmDeviceFreqs>
    {
        Ok(QmDrmDeviceFreqs::new())
    }

    fn mem_info(&mut self) -> Result<QmDrmDeviceMemInfo>
    {
        Ok(QmDrmDeviceMemInfo::new())
    }

    fn client_mem_info(&mut self,
        _mem_regs: &HashMap<String, QmDrmMemRegion>) -> Result<QmDrmClientMemInfo>
    {
        Ok(QmDrmClientMemInfo::new())
    }
}

impl Debug for dyn QmDrmDriver
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "QmDrmDriver({:?})", self.name())
    }
}

pub fn driver_from(
    qmd: &QmDrmDeviceInfo) -> Result<Option<Rc<RefCell<dyn QmDrmDriver>>>>
{
    let drvs: &[(&str,
        fn(&QmDrmDeviceInfo) -> Result<Rc<RefCell<dyn QmDrmDriver>>>)] = &[
        ("xe", QmDrmDriverXe::new),
        ("i915", QmDrmDriveri915::new),
    ];

    for (dn, drv_newfunc) in drvs {
        if *dn == qmd.drv_name {
            let drv = drv_newfunc(qmd)?;
            return Ok(Some(drv));
        }
    }

    Ok(None)
}
