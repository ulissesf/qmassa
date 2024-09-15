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
    pub device_id: String,
    pub drv_name: String,
    // TODO: add type either integrated or discrete
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

impl QmDevice
{
    pub fn find_devices() -> Result<Vec<QmDevice>>
    {
        let mut devs:Vec<QmDevice> = Vec::new();

        let mut enumerator = udev::Enumerator::new()?;
        enumerator.match_subsystem("drm")?;
        enumerator.match_property("DEVNAME", "/dev/dri/*")?;

        for d in enumerator.scan_devices()? {
             let pdev = d.parent().unwrap();
             let pciid = pdev.property_value("PCI_ID").unwrap().to_str().unwrap();

             let qmd = QmDevice {
                subsystem: String::from(d.subsystem().unwrap().to_str().unwrap()),
                devnode: String::from(d.devnode().unwrap().to_str().unwrap()),
                devnum: mj_mn_from_devnum(d.devnum().unwrap()),
                sysname: String::from(pdev.sysname().to_str().unwrap()),
                syspath: String::from(pdev.syspath().to_str().unwrap()),
                vendor_id: String::from(&pciid[0..4]),
                device_id: String::from(&pciid[5..9]),
                drv_name: String::from(pdev.driver().unwrap().to_str().unwrap())
            };

            devs.push(qmd);
        }

        Ok(devs)
    }
}
