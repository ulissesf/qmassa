use std::collections::{HashMap, HashSet};
use std::cell::RefCell;
use std::rc::{Rc, Weak};

use anyhow::{bail, Result};
use libc;
use log::{debug, warn};
use serde::{Deserialize, Serialize};
use udev;

use crate::hwmon::Hwmon;
use crate::drm_clients::{DrmClients, DrmClientInfo};
use crate::drm_drivers::{self, DrmDriver};


#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DrmDeviceType
{
    Unknown,
    Integrated,
    Discrete,
}

impl DrmDeviceType
{
    pub fn is_discrete(&self) -> bool
    {
        *self == DrmDeviceType::Discrete
    }

    pub fn is_integrated(&self) -> bool
    {
        *self == DrmDeviceType::Integrated
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
        for (nr, sensor) in slist.iter().enumerate() {
            if !sensor.has_item("input") {
                continue;
            }

            let name = if sensor.label.is_empty() {
                format!("{}", nr)
            } else {
                sensor.label.clone()
            };
            let temp_u64 = hwmon.read_sensor(&sensor.stype, "input")?;

            temps.push(DrmDeviceTemperature {
                name,
                temp: temp_u64 as f64 / 1000.0,
            });
        }

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
        for (nr, sensor) in slist.iter().enumerate() {
            if !sensor.has_item("input") {
                continue;
            }

            let name = if sensor.label.is_empty() {
                format!("{}", nr)
            } else {
                sensor.label.clone()
            };
            let speed = hwmon.read_sensor(&sensor.stype, "input")?;

            fans.push(DrmDeviceFan { name, speed, });
        }

        Ok(fans)
    }
}

#[derive(Debug)]
#[allow(dead_code)]
pub struct DrmMinorInfo
{
    pub devnode: String,
    pub drm_minor: u32,
}

impl DrmMinorInfo
{
    pub fn from(devnode: &String, devnum: u64) -> Result<DrmMinorInfo>
    {
        let mj = libc::major(devnum);
        let mn = libc::minor(devnum);

        if mj != 226 {
            bail!("Expected DRM major 226 but found {:?} for {:?}",
                mj, devnode);
        }

        Ok(DrmMinorInfo {
            devnode: devnode.clone(),
            drm_minor: mn,
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
    pub drm_minors: Vec<DrmMinorInfo>,
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
            drm_minors: Vec::new(),
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
    // tries to get DrmDriver's info, falls back to DRM clients list
    pub fn eng_utilization(&self, eng: &String) -> f64
    {
        if !self.engs_utilization.is_empty() {
            if self.engs_utilization.contains_key(eng) {
                return self.engs_utilization[eng];
            }
            return 0.0;
        } else if let Some(vref) = &self.drm_clis {
            let clis_b = vref.borrow();

            let mut res: f64 = 0.0;
            for cli in clis_b.iter() {
                res += cli.eng_utilization(eng);
            }

            if res > 100.0 {
                warn!("Engine {:?} utilization at {:?}, clamped to 100%.",
                    eng, res);
                res = 100.0;
            }
            return res;
        }

        0.0
    }

    // tries to get DrmDriver's info, falls back to DRM clients list
    pub fn engines(&self) -> Vec<String>
    {
        let mut engs = Vec::new();

        if !self.engs_utilization.is_empty() {
            engs = self.engs_utilization.keys()
                .map(|nm| nm.clone())
                .collect();
        } else if let Some(vref) = &self.drm_clis {
            let clis_b = vref.borrow();

            let mut tst: HashSet<&str> = HashSet::new();
            for cli in clis_b.iter() {
                for en in cli.engines() {
                    tst.insert(en);
                }
            }

            for en in tst.iter() {
                engs.push(en.to_string());
            }
        }

        if !engs.is_empty() {
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

        Ok(())
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

    pub fn find_devices(dev_slots: &Vec<&str>,
        drv_opts: &HashMap<&str, &str>) -> Result<DrmDevices>
    {
        let mut qmds = DrmDevices::new();

        let mut enumerator = udev::Enumerator::new()?;
        enumerator.match_subsystem("drm")?;
        enumerator.match_property("DEVNAME", "/dev/dri/*")?;

        for d in enumerator.scan_devices()? {
            let pdev = d.parent().unwrap();
            let sysname = String::from(pdev.sysname().to_str().unwrap());

            if !dev_slots.is_empty() &&
                !dev_slots.iter().any(|&ds| ds == sysname) {
                continue;
            }

            if !qmds.infos.contains_key(&sysname) {
                let pciid = if let Some(pciid) = pdev.property_value("PCI_ID") {
                    pciid.to_str().unwrap()
                } else {
                    debug!("INF: Ignoring device without PCI_ID: {:?}",
                        pdev.syspath());
                    continue;
                };

                let vendor_id = String::from(&pciid[0..4]);
                let vendor = DrmDevices::find_vendor(&vendor_id);
                let device_id = String::from(&pciid[5..9]);
                let device = DrmDevices::find_device(&vendor_id, &device_id);
                let revision = pdev.attribute_value("revision")
                    .unwrap().to_str().unwrap();
                let revision = if revision.starts_with("0x") {
                    String::from(&revision[2..])
                } else {
                    String::from(revision)
                };
                let drv_name = String::from(pdev.driver()
                    .unwrap().to_str().unwrap());

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

            let devnode = String::from(d.devnode().unwrap().to_str().unwrap());
            let devnum = d.devnum().unwrap();
            let minf = DrmMinorInfo::from(&devnode, devnum)?;

            let dinf = qmds.infos.get_mut(&sysname).unwrap();
            dinf.drm_minors.push(minf);
        }

        for dinf in qmds.infos.values_mut() {
            let dopts = drv_opts.get(dinf.drv_name.as_str()).copied();
            if let Some(drv_ref) = drm_drivers::driver_from(dinf, dopts)? {
                let dref = drv_ref.clone();
                let mut drv_b = dref.borrow_mut();

                dinf.dev_type = drv_b.dev_type()?;
                dinf.freq_limits = drv_b.freq_limits()?;
                dinf.driver = Some(drv_ref);
            }
        }

        Ok(qmds)
    }
}
