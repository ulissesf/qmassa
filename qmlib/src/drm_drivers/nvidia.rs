use std::collections::HashMap;
use std::cell::RefCell;
use std::ffi::OsStr;
use std::rc::Rc;

use anyhow::Result;
use nvml_wrapper::enum_wrappers::device::{Clock, TemperatureSensor};
use nvml_wrapper::error::NvmlError;
use nvml_wrapper::Nvml;

use crate::drm_drivers::DrmDriver;
use crate::drm_devices::{
    DrmDeviceType, DrmDeviceFreqLimits, DrmDeviceFreqs, DrmDevicePower,
    DrmDeviceMemInfo, DrmDeviceTemperature, DrmDeviceFan, DrmDeviceInfo,
    VirtFn,
};


#[derive(Debug)]
pub struct DrmDriverNvidia
{
    nvml: Nvml,
    pci_dev: String,
    domains: Vec<(String, Clock)>,
}

impl DrmDriver for DrmDriverNvidia
{
    fn name(&self) -> &str
    {
        "nvidia"
    }

    fn dev_type(&mut self) -> Result<DrmDeviceType>
    {
        // all NVIDIA GPUs with their own DRM node are discrete cards;
        // vGPU/SR-IOV awareness is not implemented yet
        Ok(DrmDeviceType::Discrete(VirtFn::NoVirt))
    }

    fn freq_limits(&mut self) -> Result<Vec<DrmDeviceFreqLimits>>
    {
        let dev = self.nvml.device_by_pci_bus_id(self.pci_dev.as_str())?;

        let mut fls = Vec::new();
        for (name, clk) in self.domains.iter() {
            let max_mhz = dev.max_clock_info(*clk)?;

            fls.push(DrmDeviceFreqLimits {
                name: name.clone(),
                minimum: 0,
                efficient: 0,
                maximum: max_mhz as u64,
            });
        }

        Ok(fls)
    }

    fn freqs(&mut self) -> Result<Vec<DrmDeviceFreqs>>
    {
        let dev = self.nvml.device_by_pci_bus_id(self.pci_dev.as_str())?;

        let mut fqs = Vec::new();
        for (_, clk) in self.domains.iter() {
            let cur_mhz = dev.clock_info(*clk)? as u64;
            let max_mhz = dev.max_clock_info(*clk)? as u64;

            fqs.push(DrmDeviceFreqs {
                min_freq: 0,
                cur_freq: cur_mhz,
                // NVML doesn't distinguish requested vs actual clock
                act_freq: cur_mhz,
                max_freq: max_mhz,
                ..Default::default()
            });
        }

        Ok(fqs)
    }

    fn power(&mut self) -> Result<Option<DrmDevicePower>>
    {
        let dev = self.nvml.device_by_pci_bus_id(self.pci_dev.as_str())?;

        match dev.power_usage() {
            Ok(mw) => Ok(Some(DrmDevicePower {
                gpu_cur_power: mw as f64 / 1000.0,
                pkg_cur_power: 0.0,
            })),
            Err(NvmlError::NotSupported) => Ok(None),
            Err(err) => Err(err.into()),
        }
    }

    fn mem_info(&mut self) -> Result<Option<DrmDeviceMemInfo>>
    {
        let dev = self.nvml.device_by_pci_bus_id(self.pci_dev.as_str())?;
        let mi = dev.memory_info()?;

        Ok(Some(
            DrmDeviceMemInfo {
                smem_total: 0,
                smem_used: 0,
                vram_total: mi.total,
                vram_used: mi.used,
            }
        ))
    }

    fn engs_utilization(&mut self) -> Result<HashMap<String, f64>>
    {
        let dev = self.nvml.device_by_pci_bus_id(self.pci_dev.as_str())?;
        let mut engs = HashMap::new();

        let ut = dev.utilization_rates()?;
        engs.insert("gpu".to_string(), ut.gpu as f64);

        if let Ok(einfo) = dev.encoder_utilization() {
            engs.insert("enc".to_string(), einfo.utilization as f64);
        }
        if let Ok(dinfo) = dev.decoder_utilization() {
            engs.insert("dec".to_string(), dinfo.utilization as f64);
        }

        Ok(engs)
    }

    fn temps(&mut self) -> Result<Vec<DrmDeviceTemperature>>
    {
        let dev = self.nvml.device_by_pci_bus_id(self.pci_dev.as_str())?;

        match dev.temperature(TemperatureSensor::Gpu) {
            Ok(t) => Ok(vec![DrmDeviceTemperature {
                name: "gpu".to_string(),
                temp: t as f64,
            }]),
            Err(NvmlError::NotSupported) => Ok(Vec::new()),
            Err(err) => Err(err.into()),
        }
    }

    fn fans(&mut self) -> Result<Vec<DrmDeviceFan>>
    {
        let dev = self.nvml.device_by_pci_bus_id(self.pci_dev.as_str())?;

        let nr_fans = match dev.num_fans() {
            Ok(n) => n,
            Err(NvmlError::NotSupported) => return Ok(Vec::new()),
            Err(err) => return Err(err.into()),
        };

        let mut fans = Vec::new();
        for idx in 0..nr_fans {
            if let Ok(rpm) = dev.fan_speed_rpm(idx) {
                fans.push(DrmDeviceFan {
                    name: idx.to_string(),
                    speed: rpm as u64,
                });
            }
        }

        Ok(fans)
    }
}

impl DrmDriverNvidia
{
    pub fn from(qmd: &DrmDeviceInfo,
        opts: Option<&Vec<&str>>) -> Result<Rc<RefCell<dyn DrmDriver>>>
    {
        let mut lib_path: Option<&str> = None;
        if let Some(opts_vec) = opts {
            for opt in opts_vec.iter() {
                if let Some(path) = opt.strip_prefix("nvml_lib=") {
                    lib_path = Some(path);
                }
            }
        }

        let nvml = if let Some(path) = lib_path {
            Nvml::builder().lib_path(OsStr::new(path)).init()?
        } else {
            Nvml::init()?
        };

        // fail fast here if this specific PCI device isn't visible to NVML,
        // so find_devices() can skip it instead of using a half-set-up driver
        let dev = nvml.device_by_pci_bus_id(qmd.pci_dev.as_str())?;

        let mut domains = Vec::new();
        for (name, clk) in [("gfx", Clock::Graphics), ("mem", Clock::Memory)] {
            if dev.max_clock_info(clk).is_ok() {
                domains.push((name.to_string(), clk));
            }
        }

        Ok(Rc::new(RefCell::new(DrmDriverNvidia {
            nvml,
            pci_dev: qmd.pci_dev.clone(),
            domains,
        })))
    }
}
