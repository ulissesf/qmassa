use anyhow::Result;
use udev;


#[derive(Debug)]
pub struct QmDevice
{
    drm_card: String,
    drm_render: String,
    subsystem: String,
    sysname: String,            // same as PCI_SLOT_NAME
    syspath: String,
    vendor_id: String,
    device_id: String,
    driver: String,
}

impl QmDevice
{
    pub fn get_devices() -> Result<Vec<QmDevice>>
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
                drm_card: String::from(d.devnode().unwrap().to_str().unwrap()),
                drm_render: String::from(""),
                subsystem: String::from(d.subsystem().unwrap().to_str().unwrap()),
                sysname: String::from(pdev.sysname().to_str().unwrap()),
                syspath: String::from(pdev.syspath().to_str().unwrap()),
                vendor_id: String::from(&pciid[0..4]),
                device_id: String::from(&pciid[5..9]),
                driver: String::from(pdev.driver().unwrap().to_str().unwrap()),
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
                    break;
                }
            }
        }

        Ok(devs)
    }
}
