use anyhow::{bail, Context, Result};
use env_logger;
use log::debug;
use clap::{Parser, ArgAction};

mod qmdrmdevices;
mod qmdrmfdinfo;
mod qmprocinfo;
mod qmdrmclients;
mod app;

use qmdrmdevices::QmDrmDevices;
use qmdrmclients::QmDrmClients;
use app::App;


/// qmassa! - display DRM clients usage stats
#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
pub struct Args {
    /// base for process tree [default: root:1, user:oldest parent PID]
    #[arg(short, long)]
    pid: Option<String>,

    /// ms interval between updates
    #[arg(short, long, default_value = "500")]
    ms_interval: Option<u64>,

    /// show all DRM clients [default: only active]
    #[arg(short, long, action = ArgAction::SetTrue)]
    all_clients: bool,
}

fn main() -> Result<()>
{
    env_logger::init();

    let args = Args::parse();

    let base_pid: String;
    if args.pid == None {
        base_pid = String::from("1");
    } else {
        base_pid = args.pid.clone().unwrap();
    }
    // TODO: if base_pid == 1 && not root, scan all current user processes

    let qmds = QmDrmDevices::find_devices()
        .context("Failed to find DRM devices")?;
    if qmds.is_empty() {
        bail!("No DRM devices found");
    }
    debug!("{:#?}", qmds);

    let mut qmclis = QmDrmClients::from_pid_tree(base_pid.as_str());
    let mut app = App::new(&qmds, &mut qmclis, &args);
    app.run()?;

    Ok(())
}
