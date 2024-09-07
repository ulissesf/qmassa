use anyhow::{Context, Result};
use env_logger;
use clap::Parser;

mod qmdevice;
use qmdevice::QmDevice;

mod qmprocinfo;


#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    #[arg(short, long, default_value = "1")]
    pid: Option<String>,
}

fn main() -> Result<()>
{
    env_logger::init();

    let args = Args::parse();
    let base_pid = args.pid.unwrap();

    for qmd in QmDevice::get_devices().context("Failed to find DRM devices")? {
        println!("{:#?}", qmd);
    }

    for fd in qmprocinfo::find_drm_fds_for_pid_tree_at(&base_pid)
        .with_context(|| format!("Failed to find DRM fds for pid tree from {}", base_pid))? {
        println!("DRM fd: {:#?}", fd);
    }

    Ok(())
}
