use core::fmt::Debug;

use anyhow::Result;
use libc;

use crate::msr::{
    Msr, MSR_IA32_TEMPERATURE_TARGET, MSR_IA32_PACKAGE_THERM_STATUS
};


#[derive(Debug)]
pub struct IntelDriverOpts
{
    opts: u32,
}

const INTEL_DRV_OPT_ENGS_PMU: u32 = 1 << 0;
const INTEL_DRV_OPT_FREQS_PMU: u32 = 1 << 1;
const INTEL_DRV_OPT_POWER_MSR: u32 = 1 << 2;

const INTEL_DRV_OPTS: &[(&str, u32)] = &[
    ("engines=pmu", INTEL_DRV_OPT_ENGS_PMU),
    ("freqs=pmu", INTEL_DRV_OPT_FREQS_PMU),
    ("power=msr", INTEL_DRV_OPT_POWER_MSR),
];

impl IntelDriverOpts
{
    pub fn has_engs_pmu(&self) -> bool
    {
        self.opts & INTEL_DRV_OPT_ENGS_PMU != 0
    }

    pub fn has_freqs_pmu(&self) -> bool
    {
        self.opts & INTEL_DRV_OPT_FREQS_PMU != 0
    }

    pub fn has_power_msr(&self) -> bool
    {
        self.opts & INTEL_DRV_OPT_POWER_MSR != 0
    }

    fn set_bit_for(&mut self, opt: &str)
    {
        for (iopt_str, iopt_mask) in INTEL_DRV_OPTS.iter() {
            if opt == *iopt_str {
                self.opts = self.opts | iopt_mask;
            }
        }
    }

    fn set_bits_from(&mut self, other: &IntelDriverOpts)
    {
        self.opts = self.opts | other.opts;
    }

    pub fn from(pci_dev: &str,
        opts_vec: Option<&Vec<&str>>) -> IntelDriverOpts
    {
        let mut ret = IntelDriverOpts { opts: 0, };
        if opts_vec.is_none() {
            return ret;
        }
        let opts_vec = opts_vec.unwrap();

        for &opts_str in opts_vec.iter() {
            let sep_opts: Vec<&str> = opts_str.split(',').collect();
            let mut devslot = "all";
            let mut want_opts = IntelDriverOpts { opts: 0, };

            for opt in sep_opts.iter() {
                if opt.starts_with("devslot=") {
                    devslot = &opt["devslot=".len()..];
                } else {
                    want_opts.set_bit_for(opt);
                }
            }

            if devslot == "all" || devslot == pci_dev {
                ret.set_bits_from(&want_opts);
            }
        }

        ret
    }
}

#[derive(Debug)]
pub struct IGpuTempIntel
{
    msr: Msr,
    tjmax: u32,
}

const TJMAX_DEFAULT: u32 = 100;

impl IGpuTempIntel
{
    // Reads TjMax from MSR_IA32_TEMPERATURE_TARGET bits [23:16].
    // Falls back to TJMAX_DEFAULT if the MSR is unavailable or returns zero.
    fn read_tjmax(msr: &Msr) -> u32
    {
        match msr.read(MSR_IA32_TEMPERATURE_TARGET) {
            Ok(val) => {
                let tcc = ((val >> 16) & 0xFF) as u32;
                if tcc == 0 { TJMAX_DEFAULT } else { tcc }
            },
            Err(_) => TJMAX_DEFAULT
        }
    }

    // Returns package temp in Celsius using MSR_IA32_PACKAGE_THERM_STATUS.
    // Formula: tjmax - DTS, where DTS is bits [22:16] (degrees below TjMax).
    pub fn pkg_temp_c(&self) -> Result<f64>
    {
        let val = self.msr.read(MSR_IA32_PACKAGE_THERM_STATUS)?;
        let dts = ((val >> 16) & 0xFF) as u32;
        Ok(self.tjmax.saturating_sub(dts) as f64)
    }

    pub fn new() -> Result<IGpuTempIntel>
    {
        let cpu: i32 = unsafe { libc::sched_getcpu() };
        let msr = Msr::from(cpu)?;
        let tjmax = IGpuTempIntel::read_tjmax(&msr);

        Ok(IGpuTempIntel {
            msr,
            tjmax,
        })
    }
}
