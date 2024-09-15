use anyhow::{Context, Result};
use env_logger;
use clap::Parser;

mod qmdevice;
use qmdevice::QmDevice;

mod qmdrmfdinfo;

mod qmprocinfo;
use qmprocinfo::QmProcInfo;


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

    // TODO: if base_pid == 1 && not root, set base_pid to first pid from user

    let qmds = QmDevice::find_devices().context("Failed to find DRM devices")?;
    for d in &qmds {
        println!("{:#?}", d);
    }

    // TODO: make sure qmds.len > 0

    let pst = QmProcInfo::from_pid_tree(&base_pid, &qmds)
        .with_context(|| format!("Failed to get proc data from tree at {:?}", base_pid))?;
    println!("{:#?}", pst);

    // TODO: add text-based UI

    Ok(())
}
