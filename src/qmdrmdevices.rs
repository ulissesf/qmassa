use std::collections::HashMap;

use anyhow::{bail, Result};
use libc;
use log::debug;
use udev;


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
    pub syspath: String,
    pub vendor_id: String,
    pub vendor: String,
    pub device_id: String,
    pub device: String,
    pub drv_name: String,
    pub drm_minors: Vec<QmDrmMinorInfo>,
    // TODO: add type either integrated or discrete
}

#[derive(Debug)]
pub struct QmDrmDevices
{
    infos: HashMap<String, QmDrmDeviceInfo>,
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

    fn new() -> QmDrmDevices
    {
        QmDrmDevices {
            infos: HashMap::new(),
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
                    debug!("INF: Ignoring device without PCI_ID: {:?}", pdev.syspath());
                    continue;
                };

                let syspath = String::from(pdev.syspath().to_str().unwrap());
                let vendor_id = String::from(&pciid[0..4]);
                let vendor = QmDrmDevices::find_vendor(&vendor_id);
                let device_id = String::from(&pciid[5..9]);
                let device = QmDrmDevices::find_device(&vendor_id, &device_id);
                let drv_name = String::from(pdev.driver()
                    .unwrap().to_str().unwrap());

                let ndinf = QmDrmDeviceInfo {
                    pci_dev: sysname.clone(),
                    syspath: syspath,
                    vendor_id: vendor_id,
                    vendor: vendor,
                    device_id: device_id,
                    device: device,
                    drv_name: drv_name,
                    drm_minors: Vec::new(),
                };
                qmds.infos.insert(sysname.clone(), ndinf);
            };

            let devnode = String::from(d.devnode().unwrap().to_str().unwrap());
            let devnum = d.devnum().unwrap();
            let minf = QmDrmMinorInfo::from(&devnode, devnum)?;

            let dinf = qmds.infos.get_mut(&sysname).unwrap();
            dinf.drm_minors.push(minf);
        }

        Ok(qmds)
    }
}
