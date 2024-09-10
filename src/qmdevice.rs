use anyhow::Result;
use libc;
use udev;


#[derive(Debug)]
pub struct QmDevice
{
    subsystem: String,
    drm_card: String,
    drm_card_devnum: (u32, u32),
    drm_render: String,
    drm_render_devnum: (u32, u32),
    sysname: String,            // same as PCI_SLOT_NAME
    syspath: String,
    vendor_id: String,
    device_id: String,
    drv_name: String,
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
                drv_name: String::from(pdev.driver().unwrap().to_str().unwrap()),
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
                    break;
                }
            }
        }

        Ok(devs)
    }
}
