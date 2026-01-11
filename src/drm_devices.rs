use std::collections::HashMap;
use std::cell::RefCell;
use std::fs;
use std::path::Path;
use std::rc::{Rc, Weak};

use anyhow::{bail, Result};
use libc;
use log::{debug, info, warn};
use serde::{Deserialize, Serialize};
use udev;

use crate::hwmon::Hwmon;
use crate::drm_clients::{DrmClients, DrmClientInfo};
use crate::drm_drivers::{self, DrmDriver};


#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum VirtFn
{
    NoVirt,
    SriovPF,
    SriovVF,
    VFIO,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DrmDeviceType
{
    Unknown,
    Integrated(VirtFn),
    Discrete(VirtFn),
}

impl DrmDeviceType
{
    pub fn is_discrete(&self) -> bool
    {
        match self {
            DrmDeviceType::Discrete(_) => true,
            _ => false
        }
    }

    pub fn is_integrated(&self) -> bool
    {
        match self {
            DrmDeviceType::Integrated(_) => true,
            _ => false
        }
    }

    pub fn to_string(&self) -> String
    {
        let mut ret = String::new();

        let sriovfn = match *self {
            DrmDeviceType::Discrete(sfn) => {
                ret.push_str("Discrete");
                sfn
            },
            DrmDeviceType::Integrated(sfn) => {
                ret.push_str("Integrated");
                sfn
            },
            DrmDeviceType::Unknown => {
                ret.push_str("Unknown");
                VirtFn::NoVirt
            }
        };

        match sriovfn {
            VirtFn::SriovPF => ret.push_str(" (PF)"),
            VirtFn::SriovVF => ret.push_str(" (VF)"),
            VirtFn::VFIO => ret.push_str(" (VFIO)"),
            _ => {}
        }

        ret
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DrmDeviceThrottleReasons
{
    pub pl1: bool,
    pub pl2: bool,
    pub pl4: bool,
    pub prochot: bool,
    pub ratl: bool,
    pub thermal: bool,
    pub vr_tdc: bool,
    pub vr_thermalert: bool,
    pub status: bool
}

impl DrmDeviceThrottleReasons
{
    pub fn new() -> DrmDeviceThrottleReasons
    {
        DrmDeviceThrottleReasons {
            pl1: false,
            pl2: false,
            pl4: false,
            prochot: false,
            ratl: false,
            thermal: false,
            vr_tdc: false,
            vr_thermalert: false,
            status: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DrmDeviceFreqLimits
{
    pub name: String,
    pub minimum: u64,
    pub efficient: u64,
    pub maximum: u64,
}

impl DrmDeviceFreqLimits
{
    pub fn new() -> DrmDeviceFreqLimits
    {
        DrmDeviceFreqLimits {
            name: String::new(),
            minimum: 0,
            efficient: 0,
            maximum: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DrmDeviceFreqs
{
    pub min_freq: u64,
    pub cur_freq: u64,
    pub act_freq: u64,
    pub max_freq: u64,
    pub throttle_reasons: DrmDeviceThrottleReasons,
}

impl DrmDeviceFreqs
{
    pub fn new() -> DrmDeviceFreqs
    {
        DrmDeviceFreqs {
            min_freq: 0,
            cur_freq: 0,
            act_freq: 0,
            max_freq: 0,
            throttle_reasons: DrmDeviceThrottleReasons::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DrmDevicePower
{
    pub gpu_cur_power: f64,
    pub pkg_cur_power: f64,
}

impl DrmDevicePower
{
    pub fn new() -> DrmDevicePower
    {
        DrmDevicePower {
            gpu_cur_power: 0.0,
            pkg_cur_power: 0.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DrmDeviceMemInfo
{
    pub smem_total: u64,
    pub smem_used: u64,
    pub vram_total: u64,
    pub vram_used: u64,
}

impl DrmDeviceMemInfo
{
    pub fn new() -> DrmDeviceMemInfo
    {
        DrmDeviceMemInfo {
            smem_total: 0,
            smem_used: 0,
            vram_total: 0,
            vram_used: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DrmDeviceTemperature
{
    pub name: String,
    pub temp: f64,
}

impl DrmDeviceTemperature
{
    pub fn from_hwmon(hwmon: &Hwmon) -> Result<Vec<DrmDeviceTemperature>>
    {
        let mut temps = Vec::new();

        let slist = hwmon.sensors("temp");
        for sensor in slist.iter() {
            if !sensor.has_item("input") {
                continue;
            }

            let name = if sensor.label.is_empty() {
                format!("{}", &sensor.stype["temp".len()..])
            } else {
                sensor.label.clone()
            };
            let temp_u64 = hwmon.read_sensor(&sensor.stype, "input")?;

            temps.push(DrmDeviceTemperature {
                name,
                temp: temp_u64 as f64 / 1000.0,
            });
        }
        temps.sort_by(|a, b| a.name.cmp(&b.name));

        Ok(temps)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DrmDeviceFan
{
    pub name: String,
    pub speed: u64,
}

impl DrmDeviceFan
{
    pub fn from_hwmon(hwmon: &Hwmon) -> Result<Vec<DrmDeviceFan>>
    {
        let mut fans = Vec::new();

        let slist = hwmon.sensors("fan");
        for sensor in slist.iter() {
            if !sensor.has_item("input") {
                continue;
            }

            let name = if sensor.label.is_empty() {
                format!("{}", &sensor.stype["fan".len()..])
            } else {
                sensor.label.clone()
            };
            let speed = hwmon.read_sensor(&sensor.stype, "input")?;

            fans.push(DrmDeviceFan { name, speed, });
        }
        fans.sort_by(|a, b| a.name.cmp(&b.name));

        Ok(fans)
    }
}

pub const DRM_DEVNODE_MAJOR: u32 = 226;

#[derive(Debug)]
#[allow(dead_code)]
pub struct DeviceNodeInfo
{
    pub devnode: String,
    pub major: u32,
    pub minor: u32,
}

impl DeviceNodeInfo
{
    fn from_drm(devnode: String, devnum: u64) -> Result<DeviceNodeInfo>
    {
        let mj = libc::major(devnum);
        let mn = libc::minor(devnum);

        if mj != DRM_DEVNODE_MAJOR {
            bail!("Expected DRM major {:?} but found {:?} for {:?}",
                DRM_DEVNODE_MAJOR, mj, devnode);
        }

        Ok(DeviceNodeInfo {
            devnode: devnode,
            major: DRM_DEVNODE_MAJOR,
            minor: mn,
        })
    }
}

#[derive(Debug)]
#[allow(dead_code)]
pub struct DrmDeviceInfo
{
    pub pci_dev: String,                // sysname or PCI_SLOT_NAME in udev
    pub vendor_id: String,
    pub vendor: String,
    pub device_id: String,
    pub device: String,
    pub revision: String,
    pub drv_name: String,
    pub dev_nodes: Vec<DeviceNodeInfo>,
    pub dev_type: DrmDeviceType,
    pub freq_limits: Vec<DrmDeviceFreqLimits>,
    pub freqs: Vec<DrmDeviceFreqs>,
    pub power: DrmDevicePower,
    pub mem_info: DrmDeviceMemInfo,
    engs_utilization: HashMap<String, f64>,
    pub temps: Vec<DrmDeviceTemperature>,
    pub fans: Vec<DrmDeviceFan>,
    driver: Option<Rc<RefCell<dyn DrmDriver>>>,
    drm_clis: Option<Rc<RefCell<Vec<DrmClientInfo>>>>,
}

impl Default for DrmDeviceInfo
{
    fn default() -> DrmDeviceInfo
    {
        DrmDeviceInfo {
            pci_dev: String::new(),
            vendor_id: String::new(),
            vendor: String::new(),
            device_id: String::new(),
            device: String::new(),
            revision: String::new(),
            drv_name: String::new(),
            dev_nodes: Vec::new(),
            dev_type: DrmDeviceType::Unknown,
            freq_limits: vec![DrmDeviceFreqLimits::new(),],
            freqs: vec![DrmDeviceFreqs::new(),],
            power: DrmDevicePower::new(),
            mem_info: DrmDeviceMemInfo::new(),
            engs_utilization: HashMap::new(),
            temps: Vec::new(),
            fans: Vec::new(),
            driver: None,
            drm_clis: None,
        }
    }
}

impl DrmDeviceInfo
{
    pub fn eng_utilization(&self, eng: &String) -> f64
    {
        if !self.engs_utilization.is_empty() {
            if self.engs_utilization.contains_key(eng) {
                return self.engs_utilization[eng];
            }
        }

        0.0
    }

    pub fn engines(&self) -> Vec<String>
    {
        let mut engs = Vec::new();

        if !self.engs_utilization.is_empty() {
            engs = self.engs_utilization.keys()
                .map(|nm| nm.clone())
                .collect();
            engs.sort();
        }

        engs
    }

    pub fn clients(&self) -> Option<Weak<RefCell<Vec<DrmClientInfo>>>>
    {
        if let Some(vref) = &self.drm_clis {
            return Some(Rc::downgrade(&vref));
        }

        None
    }

    pub fn refresh(&mut self) -> Result<()>
    {
        // reset engines usage data
        self.engs_utilization.drain();

        // update usage data from specific driver
        if let Some(drv_ref) = &self.driver {
            let mut drv_b = drv_ref.borrow_mut();

            // note: dev_type and freq_limits don't change

            self.freqs = drv_b.freqs()?;
            self.power = drv_b.power()?;
            self.mem_info = drv_b.mem_info()?;
            self.engs_utilization = drv_b.engs_utilization()?;

            if self.dev_type.is_discrete() {
                self.temps = drv_b.temps()?;
                self.fans = drv_b.fans()?;
            }
        }

        // if no driver was found or it doesn't provide engines usage data,
        // fall back to adding up utilization from DRM clients list
        if self.engs_utilization.is_empty() {
            if let Some(vref) = &self.drm_clis {
                let clis_b = vref.borrow();

                for cli in clis_b.iter() {
                    for en in cli.engines() {
                        let ut = cli.eng_utilization(en);
                        self.engs_utilization
                            .entry(en.clone())
                            .and_modify(|tot| *tot += ut)
                            .or_insert(ut);
                    }
                }

                for (en, tot) in self.engs_utilization.iter_mut() {
                    if *tot > 100.0 {
                        warn!("{}: engine {:?} utilization at {:?}, clamped to 100%.",
                            &self.pci_dev, en, tot);
                        *tot = 100.0;
                    }
                }
            }
        }

        Ok(())
    }

    pub fn has_driver(&self) -> bool
    {
        self.driver.is_some()
    }
}

#[derive(Debug)]
pub struct DrmDevices
{
    infos: HashMap<String, DrmDeviceInfo>,
    qmclis: Option<DrmClients>,
}

impl DrmDevices
{
    pub fn device_info(&self, dev: &String) -> Option<&DrmDeviceInfo>
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

        // assumes devices don't vanish, so just update their driver-specific
        // dynamic information (e.g. mem info, engines, freqs, power, ...)
        for di in self.infos.values_mut() {
            di.refresh()?;
        }

        debug!("DRM Devices: {:#?}", self.infos);

        Ok(())
    }

    pub fn set_clients_pid_tree(&mut self, at_pid: &str) -> Result<()>
    {
        self.qmclis = Some(DrmClients::from_pid_tree(at_pid)?);

        Ok(())
    }

    fn new() -> DrmDevices
    {
        DrmDevices {
            infos: HashMap::new(),
            qmclis: None,
        }
    }

    fn vendor_name(vendor_id: &String) -> String
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

    fn device_name(vendor_id: &String, device_id: &String) -> String
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

    pub fn find_devices(dev_slots: &Vec<&str>,
        drv_opts: &HashMap<&str, Vec<&str>>) -> Result<DrmDevices>
    {
        let mut qmds = DrmDevices::new();

        let mut enumerator = udev::Enumerator::new()?;
        enumerator.match_subsystem("drm")?;
        enumerator.match_property("DEVNAME", "/dev/dri/*")?;

        for d in enumerator.scan_devices()? {
            let pdev = d.parent().unwrap();
            let sysname = pdev.sysname().to_str().unwrap().to_string();

            if !dev_slots.is_empty() &&
                !dev_slots.iter().any(|&ds| ds == sysname) {
                continue;
            }

            let vendor_id: String;
            let vendor: String;
            let device_id: String;
            let device: String;
            let revision: String;

            if !qmds.infos.contains_key(&sysname) {
                if let Some(pciid) = pdev.property_value("PCI_ID") {
                    let pciid = pciid.to_str().unwrap();

                    vendor_id = String::from(&pciid[0..4]);
                    vendor = DrmDevices::vendor_name(&vendor_id);

                    device_id = String::from(&pciid[5..9]);
                    device = DrmDevices::device_name(&vendor_id, &device_id);

                    let rev_str = pdev.attribute_value("revision")
                        .unwrap().to_str().unwrap();
                    revision = if rev_str.starts_with("0x") {
                        String::from(&rev_str[2..])
                    } else {
                        String::from(rev_str)
                    };
                } else {
                    // not a PCI device
                    vendor_id = String::new();
                    vendor = String::new();
                    device_id = String::new();
                    device = String::new();
                    revision = String::new();
                };

                let drv_name = pdev.driver().unwrap()
                    .to_str().unwrap().to_string();

                let ndinf = DrmDeviceInfo {
                    pci_dev: sysname.clone(),
                    vendor_id,
                    vendor,
                    device_id,
                    device,
                    revision,
                    drv_name,
                    ..Default::default()
                };
                qmds.infos.insert(sysname.clone(), ndinf);
            }

            let devnode = d.devnode().unwrap().to_str().unwrap().to_string();
            let devnum = d.devnum().unwrap();
            let minf = DeviceNodeInfo::from_drm(devnode, devnum)?;

            let dinf = qmds.infos.get_mut(&sysname).unwrap();
            dinf.dev_nodes.push(minf);
        }

        for dinf in qmds.infos.values_mut() {
            let dopts = drv_opts.get(dinf.drv_name.as_str());
            if let Some(drv_ref) = drm_drivers::driver_from(dinf, dopts)? {
                let dref = drv_ref.clone();
                let mut drv_b = dref.borrow_mut();

                dinf.dev_type = drv_b.dev_type()?;
                dinf.freq_limits = drv_b.freq_limits()?;
                dinf.driver = Some(drv_ref);
            }
            info!(
                "New device: pci_dev={}, vendor_id={}, vendor={:?}, \
                device_id={}, device={:?}, revision={}, drv_name={}, \
                dev_type={:?}, dev_nodes={:?}",
                &dinf.pci_dev, &dinf.vendor_id, &dinf.vendor,
                &dinf.device_id, &dinf.device, &dinf.revision,
                &dinf.drv_name, &dinf.dev_type, &dinf.dev_nodes
            );
        }

        Ok(qmds)
    }
}

pub fn sysname_from_drm_minor(mn: u32) -> Result<String>
{
    let card = format!("/sys/class/drm/card{}", mn);
    let render = format!("/sys/class/drm/renderD{}", mn);

    let lnk = if Path::new(&card).is_symlink() {
        &card
    } else if Path::new(&render).is_symlink() {
        &render
    } else {
        bail!("No device for DRM minor {:?} in /sys/class/drm", mn);
    };

    let sysname = fs::read_link(Path::new(lnk).join("device"))?
        .file_name().unwrap()
        .to_str().unwrap().to_string();

    Ok(sysname)
}
