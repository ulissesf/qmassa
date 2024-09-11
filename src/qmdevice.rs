use anyhow::Result;
use libc;
use udev;

use crate::qmdriver::{self, QmDriver};


#[derive(Debug)]
pub struct QmDevice
{
    pub subsystem: String,
    pub drm_card: String,
    pub drm_card_devnum: (u32, u32),
    pub drm_render: String,
    pub drm_render_devnum: (u32, u32),
    pub sysname: String,            // same as PCI_SLOT_NAME
    pub syspath: String,
    pub vendor_id: String,
    pub device_id: String,
    pub drv_name: String,
    pub driver: Box<dyn QmDriver>,
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
        let mut rend:Vec<QmDevice> = Vec::new();

        let mut enumerator = udev::Enumerator::new()?;
        enumerator.match_subsystem("drm")?;
        enumerator.match_property("DEVNAME", "/dev/dri/*")?;

        for d in enumerator.scan_devices()? {
             let pdev = d.parent().unwrap();
             let pciid = pdev.property_value("PCI_ID").unwrap().to_str().unwrap();

             let dname = String::from(pdev.driver().unwrap().to_str().unwrap());
             let qmd = QmDevice {
                subsystem: String::from(d.subsystem().unwrap().to_str().unwrap()),
                drm_card: String::from(d.devnode().unwrap().to_str().unwrap()),
                drm_card_devnum: mj_mn_from_devnum(d.devnum().unwrap()),
                drm_render: String::from(""),
                drm_render_devnum: (0, 0),
                sysname: String::from(pdev.sysname().to_str().unwrap()),
                syspath: String::from(pdev.syspath().to_str().unwrap()),
                vendor_id: String::from(&pciid[0..4]),
                device_id: String::from(&pciid[5..9]),
                driver: qmdriver::find_driver(dname.as_str()),
                drv_name: dname,
            };

            if qmd.drm_card.starts_with("/dev/dri/render") {
                rend.push(qmd);
            } else {
                devs.push(qmd);
            }
        }

        for cd in &mut devs {
            for rd in &rend {
                if rd.syspath == cd.syspath {
                    cd.drm_render = rd.drm_card.clone();
                    cd.drm_render_devnum = rd.drm_card_devnum;
                    cd.driver.add_device(cd);
                    break;
                }
            }
        }

        Ok(devs)
    }
}
