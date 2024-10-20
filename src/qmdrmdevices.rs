use std::collections::HashMap;
use std::cell::RefCell;
use std::rc::{Rc, Weak};

use anyhow::{bail, Result};
use libc;
use log::debug;
use serde::{Deserialize, Serialize};
use udev;

use crate::qmdrmclients::{QmDrmClients, QmDrmClientInfo};
use crate::qmdrmdrivers::{self, QmDrmDriver};


#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QmDrmDeviceType
{
    Unknown,
    Integrated,
    Discrete,
}

impl QmDrmDeviceType
{
    pub fn is_discrete(&self) -> bool
    {
        *self == QmDrmDeviceType::Discrete
    }

    pub fn is_integrated(&self) -> bool
    {
        *self == QmDrmDeviceType::Integrated
    }

    pub fn to_string(&self) -> String
    {
        if self.is_discrete() {
            String::from("Discrete")
        } else if self.is_integrated() {
            String::from("Integrated")
        } else {
            String::from("Unknown")
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QmDrmDeviceFreqs
{
    pub min_freq: u64,
    pub cur_freq: u64,
    pub act_freq: u64,
    pub max_freq: u64,
}

impl QmDrmDeviceFreqs
{
    pub fn new() -> QmDrmDeviceFreqs
    {
        QmDrmDeviceFreqs {
            min_freq: 0,
            cur_freq: 0,
            act_freq: 0,
            max_freq: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QmDrmDeviceMemInfo
{
    pub smem_total: u64,
    pub smem_used: u64,
    pub vram_total: u64,
    pub vram_used: u64,
}

impl QmDrmDeviceMemInfo
{
    pub fn new() -> QmDrmDeviceMemInfo
    {
        QmDrmDeviceMemInfo {
            smem_total: 0,
            smem_used: 0,
            vram_total: 0,
            vram_used: 0,
        }
    }
}

#[derive(Debug)]
pub struct QmDrmMinorInfo
{
    pub devnode: String,
    pub drm_minor: u32,
}

impl QmDrmMinorInfo
{
    pub fn from(devnode: &String, devnum: u64) -> Result<QmDrmMinorInfo>
    {
        let mj: u32;
        let mn: u32;

        unsafe {
            mj = libc::major(devnum);
            mn = libc::minor(devnum);
        }

        if mj != 226 {
            bail!("Expected DRM major 226 but found {:?} for {:?}",
                mj, devnode);
        }

        Ok(QmDrmMinorInfo {
            devnode: devnode.clone(),
            drm_minor: mn,
        })
    }
}

#[derive(Debug)]
pub struct QmDrmDeviceInfo
{
    pub pci_dev: String,                // sysname or PCI_SLOT_NAME in udev
    pub dev_type: QmDrmDeviceType,
    pub freqs: QmDrmDeviceFreqs,
    pub mem_info: QmDrmDeviceMemInfo,
    pub vendor_id: String,
    pub vendor: String,
    pub device_id: String,
    pub device: String,
    pub revision: String,
    pub drv_name: String,
    pub drm_minors: Vec<QmDrmMinorInfo>,
    driver: Option<Rc<RefCell<dyn QmDrmDriver>>>,
    drm_clis: Option<Rc<RefCell<Vec<QmDrmClientInfo>>>>,
}

impl Default for QmDrmDeviceInfo
{
    fn default() -> QmDrmDeviceInfo
    {
        QmDrmDeviceInfo {
            pci_dev: String::from(""),
            dev_type: QmDrmDeviceType::Unknown,
            freqs: QmDrmDeviceFreqs::new(),
            mem_info: QmDrmDeviceMemInfo::new(),
            vendor_id: String::from(""),
            vendor: String::from(""),
            device_id: String::from(""),
            device: String::from(""),
            revision: String::from(""),
            drv_name: String::from(""),
            drm_minors: Vec::new(),
            driver: None,
            drm_clis: None,
        }
    }
}

impl QmDrmDeviceInfo
{
    pub fn clients(&self) -> Option<Weak<RefCell<Vec<QmDrmClientInfo>>>>
    {
        if let Some(vref) = &self.drm_clis {
            return Some(Rc::downgrade(&vref));
        }

        None
    }

    pub fn refresh(&mut self) -> Result<()>
    {
        if let Some(drv_ref) = &self.driver {
            let mut drv_b = drv_ref.borrow_mut();

            // note: dev_type doesn't change
            self.freqs = drv_b.freqs()?;
            self.mem_info = drv_b.mem_info()?;
        }

        Ok(())
    }
}

#[derive(Debug)]
pub struct QmDrmDevices
{
    infos: HashMap<String, QmDrmDeviceInfo>,
    qmclis: Option<QmDrmClients>,
}

impl QmDrmDevices
{
    pub fn device_info(&self, dev: &String) -> Option<&QmDrmDeviceInfo>
    {
        self.infos.get(dev)
    }

    pub fn devices(&self) -> Vec<&String>
    {
        let mut res: Vec<&String> = self.infos.keys().collect::<Vec<&_>>();
        res.sort();

        res
    }

    pub fn is_empty(&self) -> bool
    {
        self.infos.is_empty()
    }

    pub fn refresh(&mut self) -> Result<()>
    {
        // assumes devices don't vanish, so just update their
        // driver-specific dynamic information (e.g. freqs)
        for di in self.infos.values_mut() {
            di.refresh()?;
        }

        // update DRM clients information (if possible)
        if let Some(clis) = &mut self.qmclis {
            clis.refresh()?;

            for di in self.infos.values_mut() {
                di.drm_clis = clis.device_clients(&di.pci_dev);
                if let Some(drv_ref) = &di.driver {
                    let drv_wref = Rc::downgrade(drv_ref);
                    clis.set_dev_clients_driver(&di.pci_dev, drv_wref);
                }
            }
        }

        debug!("DRM Devices: {:#?}", self.infos);

        Ok(())
    }

    pub fn clients_pid_tree(&mut self, at_pid: &str)
    {
        self.qmclis = Some(QmDrmClients::from_pid_tree(at_pid));
    }

    fn new() -> QmDrmDevices
    {
        QmDrmDevices {
            infos: HashMap::new(),
            qmclis: None,
        }
    }

    fn find_vendor(vendor_id: &String) -> String
    {
        if let Ok(hwdb) = udev::Hwdb::new() {
            let id = u32::from_str_radix(vendor_id, 16).unwrap();
            let modalias = format!("pci:v{:08X}*", id);

            if let Some(res) = hwdb.query_one(modalias,
                "ID_VENDOR_FROM_DATABASE".to_string()) {
                return res.to_str().unwrap().to_string();
            }
        }

        vendor_id.clone()
    }

    fn find_device(vendor_id: &String, device_id: &String) -> String
    {
        if let Ok(hwdb) = udev::Hwdb::new() {
            let vid = u32::from_str_radix(vendor_id, 16).unwrap();
            let did = u32::from_str_radix(device_id, 16).unwrap();
            let modalias = format!("pci:v{:08X}d{:08X}*", vid, did);

            if let Some(res) = hwdb.query_one(modalias,
                "ID_MODEL_FROM_DATABASE".to_string()) {
                return res.to_str().unwrap().to_string();
            }
        }

        device_id.clone()
    }

    pub fn find_devices() -> Result<QmDrmDevices>
    {
        let mut qmds = QmDrmDevices::new();

        let mut enumerator = udev::Enumerator::new()?;
        enumerator.match_subsystem("drm")?;
        enumerator.match_property("DEVNAME", "/dev/dri/*")?;

        for d in enumerator.scan_devices()? {
            let pdev = d.parent().unwrap();
            let sysname = String::from(pdev.sysname().to_str().unwrap());

            if !qmds.infos.contains_key(&sysname) {
                let pciid = if let Some(pciid) = pdev.property_value("PCI_ID") {
                    pciid.to_str().unwrap()
                } else {
                    debug!("INF: Ignoring device without PCI_ID: {:?}",
                        pdev.syspath());
                    continue;
                };

                let vendor_id = String::from(&pciid[0..4]);
                let vendor = QmDrmDevices::find_vendor(&vendor_id);
                let device_id = String::from(&pciid[5..9]);
                let device = QmDrmDevices::find_device(&vendor_id, &device_id);
                let revision = pdev.attribute_value("revision")
                    .unwrap().to_str().unwrap();
                let revision = if revision.starts_with("0x") {
                    String::from(&revision[2..])
                } else {
                    String::from(revision)
                };
                let drv_name = String::from(pdev.driver()
                    .unwrap().to_str().unwrap());

                let ndinf = QmDrmDeviceInfo {
                    pci_dev: sysname.clone(),
                    vendor_id: vendor_id,
                    vendor: vendor,
                    device_id: device_id,
                    device: device,
                    revision: revision,
                    drv_name: drv_name,
                    ..Default::default()
                };
                qmds.infos.insert(sysname.clone(), ndinf);
            };

            let devnode = String::from(d.devnode().unwrap().to_str().unwrap());
            let devnum = d.devnum().unwrap();
            let minf = QmDrmMinorInfo::from(&devnode, devnum)?;

            let dinf = qmds.infos.get_mut(&sysname).unwrap();
            dinf.drm_minors.push(minf);

            if dinf.drm_minors.len() == 1 {
                if let Some(drv_ref) = qmdrmdrivers::driver_from(dinf)? {
                    let dref = drv_ref.clone();
                    let mut drv_b = dref.borrow_mut();

                    dinf.dev_type = drv_b.dev_type()?;
                    dinf.freqs = drv_b.freqs()?;
                    dinf.mem_info = drv_b.mem_info()?;

                    dinf.driver = Some(drv_ref);
                }
            }
        }

        Ok(qmds)
    }
}
