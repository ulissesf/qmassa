use std::collections::HashMap;

use anyhow::Result;
use libc;
use udev;


#[derive(Debug)]
pub struct QmDevice
{
    pub subsystem: String,
    pub devnode: String,
    pub devnum: (u32, u32),
    pub sysname: String,            // same as PCI_SLOT_NAME
    pub syspath: String,
    pub vendor_id: String,
    pub vendor: String,
    pub device_id: String,
    pub device: String,
    pub drv_name: String,
    // TODO: add type either integrated or discrete
}

impl QmDevice
{
    pub fn major(&self) -> u32
    {
        self.devnum.0
    }

    pub fn minor(&self) -> u32
    {
        self.devnum.1
    }

    fn mj_mn_from_devnum(dnum: u64) -> (u32, u32)
    {
        let mj: u32;
        let mn: u32;

        unsafe {
            mj = libc::major(dnum);
            mn = libc::minor(dnum);
        }

        (mj, mn)
    }

    fn find_vendor(qmd: &QmDevice) -> String
    {
        if let Ok(hwdb) = udev::Hwdb::new() {
            let id = u32::from_str_radix(&qmd.vendor_id, 16).unwrap();
            let modalias = format!("pci:v{:08X}*", id);

            if let Some(res) = hwdb.query_one(modalias,
                "ID_VENDOR_FROM_DATABASE".to_string()) {
                return res.to_str().unwrap().to_string();
            }
        }

        qmd.vendor_id.clone()
    }

    fn find_device(qmd: &QmDevice) -> String
    {
        if let Ok(hwdb) = udev::Hwdb::new() {
            let vid = u32::from_str_radix(&qmd.vendor_id, 16).unwrap();
            let did = u32::from_str_radix(&qmd.device_id, 16).unwrap();
            let modalias = format!("pci:v{:08X}d{:08X}*", vid, did);

            if let Some(res) = hwdb.query_one(modalias,
                "ID_MODEL_FROM_DATABASE".to_string()) {
                return res.to_str().unwrap().to_string();
            }
        }

        format!("{}:{}", qmd.vendor_id, qmd.device_id)
    }

    pub fn find_devices() -> Result<HashMap<u32, QmDevice>>
    {
        let mut devs:HashMap<u32, QmDevice> = HashMap::new();

        let mut enumerator = udev::Enumerator::new()?;
        enumerator.match_subsystem("drm")?;
        enumerator.match_property("DEVNAME", "/dev/dri/*")?;

        for d in enumerator.scan_devices()? {
            let pdev = d.parent().unwrap();
            let pciid = pdev.property_value("PCI_ID").unwrap().to_str().unwrap();

            let mut qmd = QmDevice {
                subsystem: String::from(d.subsystem().unwrap().to_str().unwrap()),
                devnode: String::from(d.devnode().unwrap().to_str().unwrap()),
                devnum: QmDevice::mj_mn_from_devnum(d.devnum().unwrap()),
                sysname: String::from(pdev.sysname().to_str().unwrap()),
                syspath: String::from(pdev.syspath().to_str().unwrap()),
                vendor_id: String::from(&pciid[0..4]),
                device_id: String::from(&pciid[5..9]),
                vendor: String::from(""),
                device: String::from(""),
                drv_name: String::from(pdev.driver().unwrap().to_str().unwrap())
            };

            qmd.vendor = QmDevice::find_vendor(&qmd);
            qmd.device = QmDevice::find_device(&qmd);

            devs.insert(qmd.devnum.1, qmd);
        }

        Ok(devs)
    }
}
