use anyhow::{Context, Result};
use env_logger;
use log::debug;
use clap::Parser;

mod qmdevice;
mod qmprocinfo;
mod qmdrmfdinfo;
mod qmdrmclients;
mod app;

use qmdevice::QmDevice;
use qmdrmclients::QmDrmClients;
use app::App;


#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Args {
    #[arg(short, long, default_value = "1")]
    pid: Option<String>,

    #[arg(short, long, default_value = "500")]
    ms_interval: Option<u64>,
}

fn main() -> Result<()>
{
    env_logger::init();

    let args = Args::parse();
    let base_pid = args.pid.unwrap();
    let ms_interval = args.ms_interval.unwrap();

    // TODO: if base_pid == 1 && not root, scan all current user processes

    let qmds = QmDevice::find_devices().context("Failed to find DRM devices")?;
    if qmds.is_empty() {
        anyhow::bail!("No DRM devices found");
    }
    debug!("{:#?}", qmds);

    let mut clis = QmDrmClients::from_pid_tree(base_pid.as_str());
    let mut app = App::new(&qmds, &mut clis, ms_interval);
    app.run()?;

    Ok(())
}
