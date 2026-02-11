use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Result;
use metrics::{counter, gauge, Counter, Gauge};

use qmlib::drm_devices::{DrmDevices, DrmDeviceInfo};


#[derive(Debug)]
pub struct StatsCtrl
{
    qmds: DrmDevices,
    infos: HashMap<String, Counter>,
    gauges: HashMap<String, HashMap<String, Gauge>>,
}

impl StatsCtrl
{
    fn update_meminfo(dn: &String,
        di: &DrmDeviceInfo, gs: &mut HashMap<String, Gauge>)
    {
        let mi = di.mem_info.as_ref().unwrap();

        let smem = String::from("smem");
        if !gs.contains_key(&smem) {
            let labels = vec![
                ("device", dn.clone()),
                ("mem_type", smem.clone()),
                ("total", mi.smem_total.to_string())
            ];
            gs.insert(smem.clone(),
                gauge!("qmd_gpu_memory_utilization", &labels));
        }

        let gg = gs.get_mut(&smem).unwrap();
        gg.set(mi.smem_used as f64);

        if di.dev_type.is_discrete() {
            let vram = String::from("vram");
            if !gs.contains_key(&vram) {
                let labels = vec![
                    ("device", dn.clone()),
                    ("mem_type", vram.clone()),
                    ("total", mi.vram_total.to_string())
                ];
                gs.insert(vram.clone(),
                    gauge!("qmd_gpu_memory_utilization", &labels));
            }

            let gg = gs.get_mut(&vram).unwrap();
            gg.set(mi.vram_used as f64);
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
                    gauge!("qmd_gpu_engine_utilization", &labels));
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
                    gauge!("qmd_gpu_frequency", &labels));
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
                ("domain", String::from("gpu"))
            ];
            gs.insert(gpu_key.clone(),
                gauge!("qmd_gpu_power", &labels));
        }

        let gg = gs.get_mut(&gpu_key).unwrap();
        gg.set(pwr.gpu_cur_power);

        let pkg_key = String::from("power-pkg");
        if !gs.contains_key(&pkg_key) {
            let labels = vec![
                ("device", dn.clone()),
                ("domain", String::from(
                    if di.dev_type.is_discrete() { "card" } else { "package" }
                ))
            ];
            gs.insert(pkg_key.clone(),
                gauge!("qmd_gpu_power", &labels));
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
                    gauge!("qmd_gpu_temperature", &labels));
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
                    gauge!("qmd_gpu_fan_speed", &labels));
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

            let cnt = self.infos.get_mut(dn).unwrap();
            let tstamp = SystemTime::now()
                .duration_since(UNIX_EPOCH).expect("Time went backwards")
                .as_millis() as u64;
            cnt.absolute(tstamp);

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
        let mut infos = HashMap::new();
        let mut gauges = HashMap::new();

        for dn in qmds.devices().iter() {
            let di = qmds.device_info(dn).unwrap();

            let labels = vec![
                ("device", di.pci_dev.clone()),
                ("pci_id", format!("{}:{}", &di.vendor_id, &di.device_id)),
                ("vendor_name", di.vendor.clone()),
                ("device_name", di.device.clone()),
                ("revision", di.revision.clone()),
                ("driver_name", di.drv_name.clone()),
                ("dev_type", di.dev_type.to_string()),
            ];
            let cnt = counter!("qmd_gpu_info", &labels);

            infos.insert(dn.to_string(), cnt);
            gauges.insert(dn.to_string(), HashMap::new());
        }

        StatsCtrl {
            qmds,
            infos,
            gauges,
        }
    }
}
