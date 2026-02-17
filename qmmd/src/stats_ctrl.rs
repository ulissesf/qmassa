use std::collections::HashMap;

use anyhow::Result;
use metrics::{counter, gauge, Gauge};

use qmlib::drm_devices::{DrmDevices, DrmDeviceInfo};


#[derive(Debug)]
pub struct StatsCtrl
{
    qmds: DrmDevices,
    gauges: HashMap<String, HashMap<String, Gauge>>,
}

impl StatsCtrl
{
    fn update_meminfo(dn: &String,
        di: &DrmDeviceInfo, gs: &mut HashMap<String, Gauge>)
    {
        let mi = di.mem_info.as_ref().unwrap();

        let smem_used = String::from("smem-used");
        let smem_tot = String::from("smem-total");
        if !gs.contains_key(&smem_used) {
            let labels = vec![
                ("device", dn.clone()),
            ];
            gs.insert(smem_used.clone(),
                gauge!("qmmd_gpu_smem_used_bytes", &labels));
            gs.insert(smem_tot.clone(),
                gauge!("qmmd_gpu_smem_total_bytes", &labels));
        }

        let gg = gs.get_mut(&smem_used).unwrap();
        gg.set(mi.smem_used as f64);
        let gg = gs.get_mut(&smem_tot).unwrap();
        gg.set(mi.smem_total as f64);

        if di.dev_type.is_discrete() {
            let vram_used = String::from("vram-used");
            let vram_tot = String::from("vram-total");
            if !gs.contains_key(&vram_used) {
                let labels = vec![
                    ("device", dn.clone()),
                ];
                gs.insert(vram_used.clone(),
                    gauge!("qmmd_gpu_vram_used_bytes", &labels));
                gs.insert(vram_tot.clone(),
                    gauge!("qmmd_gpu_vram_total_bytes", &labels));
            }

            let gg = gs.get_mut(&vram_used).unwrap();
            gg.set(mi.vram_used as f64);
            let gg = gs.get_mut(&vram_tot).unwrap();
            gg.set(mi.vram_total as f64);
        }
    }

    fn update_engines(dn: &String,
        di: &DrmDeviceInfo, gs: &mut HashMap<String, Gauge>)
    {
        for en in di.engines().iter() {
            let eng_key = format!("engine-{}", en);
            if !gs.contains_key(&eng_key) {
                let labels = vec![
                    ("device", dn.clone()),
                    ("name", en.clone())
                ];
                gs.insert(eng_key.clone(),
                    gauge!("qmmd_gpu_engine_utilization_pct", &labels));
            }

            let gg = gs.get_mut(&eng_key).unwrap();
            gg.set(di.eng_utilization(en));
        }
    }

    fn update_freqs(dn: &String,
        di: &DrmDeviceInfo, gs: &mut HashMap<String, Gauge>)
    {
        for (fql, freq) in di.freq_limits.iter().zip(di.freqs.iter()) {
            let freq_key = format!("freq-{}", fql.name);
            if !gs.contains_key(&freq_key) {
                let labels = vec![
                    ("device", dn.clone()),
                    ("name", fql.name.clone())
                ];
                gs.insert(freq_key.clone(),
                    gauge!("qmmd_gpu_frequency_mhz", &labels));
            }

            let gg = gs.get_mut(&freq_key).unwrap();
            gg.set(freq.act_freq as f64);
        }
    }

    fn update_power(dn: &String,
        di: &DrmDeviceInfo, gs: &mut HashMap<String, Gauge>)
    {
        let pwr = di.power.as_ref().unwrap();

        let gpu_key = String::from("power-gpu");
        if !gs.contains_key(&gpu_key) {
            let labels = vec![
                ("device", dn.clone()),
            ];
            gs.insert(gpu_key.clone(),
                gauge!("qmmd_gpu_power_watts", &labels));
        }

        let gg = gs.get_mut(&gpu_key).unwrap();
        gg.set(pwr.gpu_cur_power);

        let pkg_key = String::from("power-pkg");
        if !gs.contains_key(&pkg_key) {
            let labels = vec![
                ("device", dn.clone()),
            ];
            gs.insert(pkg_key.clone(),
                gauge!("qmmd_gpu_package_power_watts", &labels));
        }

        let gg = gs.get_mut(&pkg_key).unwrap();
        gg.set(pwr.pkg_cur_power);
    }

    fn update_temps(dn: &String,
        di: &DrmDeviceInfo, gs: &mut HashMap<String, Gauge>)
    {
        for tmp in di.temps.iter() {
            let tmp_key = format!("temp-{}", tmp.name);
            if !gs.contains_key(&tmp_key) {
                let labels = vec![
                    ("device", dn.clone()),
                    ("name", tmp.name.clone())
                ];
                gs.insert(tmp_key.clone(),
                    gauge!("qmmd_gpu_temperature_celsius", &labels));
            }

            let gg = gs.get_mut(&tmp_key).unwrap();
            gg.set(tmp.temp);
        }
    }

    fn update_fans(dn: &String,
        di: &DrmDeviceInfo, gs: &mut HashMap<String, Gauge>)
    {
        for fan in di.fans.iter() {
            let fan_key = format!("fan-{}", fan.name);
            if !gs.contains_key(&fan_key) {
                let labels = vec![
                    ("device", dn.clone()),
                    ("name", fan.name.clone())
                ];
                gs.insert(fan_key.clone(),
                    gauge!("qmmd_gpu_fan_speed_rpm", &labels));
            }

            let gg = gs.get_mut(&fan_key).unwrap();
            gg.set(fan.speed as f64);
        }
    }

    pub fn iterate(&mut self) -> Result<()>
    {
        // refresh GPUs' stats
        self.qmds.refresh()?;

        // create/update info & gauges
        for (dn, gs) in self.gauges.iter_mut() {
            let di = self.qmds.device_info(dn).unwrap();
            if !di.has_driver() {
                continue;
            }

            if di.mem_info.is_some() {
                StatsCtrl::update_meminfo(dn, di, gs);
            }
            if !di.engines().is_empty() {
                StatsCtrl::update_engines(dn, di, gs);
            }
            if !di.freqs.is_empty() {
                StatsCtrl::update_freqs(dn, di, gs);
            }
            if di.power.is_some() {
                StatsCtrl::update_power(dn, di, gs);
            }
            if !di.temps.is_empty() {
                StatsCtrl::update_temps(dn, di, gs);
            }
            if !di.fans.is_empty() {
                StatsCtrl::update_fans(dn, di, gs);
            }
        }

        Ok(())
    }

    pub fn from(qmds: DrmDevices) -> StatsCtrl
    {
        let mut gauges = HashMap::new();

        for dn in qmds.devices().iter() {
            let di = qmds.device_info(dn).unwrap();

            let mut devnodes = String::new();
            for node in di.dev_nodes.iter() {
                if !devnodes.is_empty() {
                    devnodes.push_str(",");
                }
                devnodes.push_str(&node.devnode);
            }

            let labels = vec![
                ("device", di.pci_dev.clone()),
                ("pci_id", format!("{}:{}", &di.vendor_id, &di.device_id)),
                ("vendor_name", di.vendor.clone()),
                ("device_name", di.device.clone()),
                ("revision", di.revision.clone()),
                ("driver_name", di.drv_name.clone()),
                ("dev_type", di.dev_type.to_string()),
                ("dev_nodes", devnodes),
            ];
            let cnt = counter!("qmmd_gpu_info", &labels);
            cnt.absolute(1);

            gauges.insert(dn.to_string(), HashMap::new());
        }

        StatsCtrl {
            qmds,
            gauges,
        }
    }
}
