use std::collections::HashMap;
use std::path::Path;
use std::fs::File;
use std::os::fd::{RawFd, AsRawFd};
use std::mem;
use std::io;

use anyhow::{bail, Result};
use libc;
use log::debug;


// from kernel's msr-index.h
pub const MSR_RAPL_POWER_UNIT: i64 = 0x00000606;
pub const MSR_PKG_ENERGY_STATUS: i64 = 0x00000611;  // "energy-pkg"
pub const MSR_PP1_ENERGY_STATUS: i64 = 0x00000641;  // "energy-gpu"
pub const MSR_IA32_TEMPERATURE_TARGET: i64 = 0x000001a2;
pub const MSR_IA32_PACKAGE_THERM_STATUS: i64 = 0x000001b1;

#[derive(Debug)]
struct MsrSum
{
    sum: u64,
    last: u64,
}

#[derive(Debug)]
pub struct Msr
{
    _dn_file: File,
    dn_fd: RawFd,
    sums: HashMap<i64, MsrSum>,
}

impl Msr
{
    pub fn read(&self, offset: i64) -> Result<u64>
    {
        let mut val: u64 = 0;
        let val_ptr: *mut u64 = &mut val;
        let val_vptr = val_ptr as *mut libc::c_void;
        let size = mem::size_of::<u64>();

        let ret = unsafe {
            libc::pread(self.dn_fd, val_vptr, size, offset) };
        if ret < 0 {
            return Err(io::Error::last_os_error().into());
        }
        if ret as usize != mem::size_of::<u64>() {
            bail!("Read wrong # bytes {:?} (expected {:?}) from MSR {:?}.",
                ret, mem::size_of::<u64>(), offset);
        }

        Ok(val)
    }

    pub fn read_sum(&mut self, offset: i64) -> Result<u64>
    {
        if !self.sums.contains_key(&offset) {
            self.sums.insert(offset, MsrSum { sum: 0, last: 0, });
        }

        let val = self.read(offset)?;
        let msrsum = self.sums.get_mut(&offset).unwrap();

        let cur = val & 0xffffffff;
        let delta_val = cur.wrapping_sub(msrsum.last);
        msrsum.last = cur;
        msrsum.sum += delta_val;

        Ok(msrsum.sum)
    }

    pub fn probe(&self, offset: i64) -> bool
    {
        self.read(offset).is_ok()
    }

    pub fn from(cpu: i32) -> Result<Msr>
    {
        let fname = format!("/dev/cpu/{}/msr", cpu);
        let file = File::open(fname)?;
        let fd = file.as_raw_fd();

        Ok(Msr {
            _dn_file: file,
            dn_fd: fd,
            sums: HashMap::new(),
        })
    }

    pub fn is_capable() -> bool
    {
        if !Path::new("/dev/cpu/0/msr").exists() {
            debug!("INF: couldn't find MSR device node.");
            return false;
        }

        if unsafe { libc::geteuid() } != 0 {
            debug!("INF: non-root user, no MSR device node access.");
            return false;
        }

        true
    }
}
