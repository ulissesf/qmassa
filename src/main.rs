use anyhow::{Context, Result};
use env_logger;
use clap::Parser;

mod qmdevice;
mod qmprocinfo;
mod qmdrmfdinfo;
mod qmdrmclients;

use qmdevice::QmDevice;
use qmdrmclients::QmDrmClients;


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

    // TODO: if base_pid == 1 && not root, scan all current user processes

    let qmds = QmDevice::find_devices().context("Failed to find DRM devices")?;
    println!("{:#?}", qmds);

    // TODO: make sure qmds.len > 0

    let mut clis = QmDrmClients::from_pid_tree(base_pid.as_str());
    let infos = clis.refresh();
    println!("{:#?}", infos);

    // TODO: add text-based UI

    Ok(())
}
