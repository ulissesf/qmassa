use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::fs;
use std::cell::RefCell;
use std::rc::Rc;

use anyhow::Result;
use log::warn;

use crate::qmdrmdrivers::QmDrmDriver;
use crate::qmdrmdevices::{QmDrmDeviceFreqs, QmDrmDeviceInfo, QmDrmDeviceThrottleReasons};
use crate::qmdrmfdinfo::QmDrmMemRegion;
use crate::qmdrmclients::QmDrmClientMemInfo;


#[derive(Debug)]
pub struct QmDrmDriveri915
{
    freqs_dir: PathBuf,
    gt_dir: PathBuf,
}

impl QmDrmDriver for QmDrmDriveri915
{
    fn name(&self) -> &str
    {
        "i915"
    }

    fn freqs(&mut self) -> Result<QmDrmDeviceFreqs>
    {
        let fpath = self.freqs_dir.join("gt_min_freq_mhz");
        let fstr = fs::read_to_string(&fpath)?;
        let min_val: u64 = fstr.trim_end().parse()?;

        let fpath = self.freqs_dir.join("gt_cur_freq_mhz");
        let fstr = fs::read_to_string(&fpath)?;
        let cur_val: u64 = fstr.trim_end().parse()?;

        let fpath = self.freqs_dir.join("gt_act_freq_mhz");
        let fstr = fs::read_to_string(&fpath)?;
        let act_val: u64 = fstr.trim_end().parse()?;

        let fpath = self.freqs_dir.join("gt_max_freq_mhz");
        let fstr = fs::read_to_string(&fpath)?;
        let max_val: u64 = fstr.trim_end().parse()?;

        let fpath = self.gt_dir.join("throttle_reason_pl1");
        let pl1 = fs::read_to_string(&fpath)?.trim() == "1";

        let fpath = self.gt_dir.join("throttle_reason_pl2");
        let pl2 = fs::read_to_string(&fpath)?.trim() == "1";

        let fpath = self.gt_dir.join("throttle_reason_pl4");
        let pl4 = fs::read_to_string(&fpath)?.trim() == "1";

        let fpath = self.gt_dir.join("throttle_reason_prochot");
        let prochot = fs::read_to_string(&fpath)?.trim() == "1";

        let fpath = self.gt_dir.join("throttle_reason_ratl");
        let ratl = fs::read_to_string(&fpath)?.trim() == "1";

        let fpath = self.gt_dir.join("throttle_reason_thermal");
        let thermal = fs::read_to_string(&fpath)?.trim() == "1";

        let fpath = self.gt_dir.join("throttle_reason_vr_tdc");
        let vr_tdc = fs::read_to_string(&fpath)?.trim() == "1";

        let fpath = self.gt_dir.join("throttle_reason_vr_thermalert");
        let vr_thermalert = fs::read_to_string(&fpath)?.trim() == "1";

        let fpath = self.gt_dir.join("throttle_reason_status");
        let status = fs::read_to_string(&fpath)?.trim() == "1";

        let throttle = QmDrmDeviceThrottleReasons {
            pl1: pl1,
            pl2: pl2,
            pl4: pl4,
            prochot: prochot,
            ratl: ratl,
            thermal: thermal,
            vr_tdc: vr_tdc,
            vr_thermalert: vr_thermalert,
            status: status,
        };

        Ok(QmDrmDeviceFreqs {
            min_freq: min_val,
            cur_freq: cur_val,
            act_freq: act_val,
            max_freq: max_val,
            throttle_reasons: throttle,
        })
    }

    fn client_mem_info(&mut self,
        mem_regs: &HashMap<String, QmDrmMemRegion>) -> Result<QmDrmClientMemInfo>
    {
        let mut cmi = QmDrmClientMemInfo::new();

        for mr in mem_regs.values() {
            if mr.name.starts_with("system") ||
                mr.name.starts_with("stolen-system") {
                cmi.smem_used += mr.total;
                cmi.smem_rss += mr.resident;
            } else if mr.name.starts_with("local") ||
                mr.name.starts_with("stolen-local") {
                cmi.vram_used += mr.total;
                cmi.vram_rss += mr.resident;
            } else {
                warn!("Unknown i915 memory region: {:?}, skpping it.", mr.name);
                continue;
            }
        }

        Ok(cmi)
    }
}

impl QmDrmDriveri915
{
    pub fn new(qmd: &QmDrmDeviceInfo) -> Result<Rc<RefCell<dyn QmDrmDriver>>>
    {
        let mut cpath = String::from("/sys/class/drm/");
        let card = Path::new(&qmd.drm_minors[0].devnode)
            .file_name().unwrap().to_str().unwrap();
        cpath.push_str(card);

        Ok(Rc::new(RefCell::new(QmDrmDriveri915 {
            freqs_dir: PathBuf::from(&cpath),
            gt_dir: Path::new(&cpath).join("gt/gt0"),
        })))
    }
}
