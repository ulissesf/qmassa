use anyhow::{Context, Result};
use env_logger;
use clap::Parser;

mod qmdevice;
use qmdevice::QmDevice;


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

    let qmds = QmDevice::find_devices().context("Failed to find DRM devices")?;
    for d in qmds {
        println!("{:#?}", d);
    }

    Ok(())
}
