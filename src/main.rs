use std::cell::RefCell;
use std::env;
use std::fs::File;
use std::io::{self, IsTerminal};
use std::path::Path;
use std::process;
use std::rc::Rc;

use anyhow::{bail, Context, Result};
use env_logger;
use clap::{ArgAction, Args, Parser, Subcommand};
use libc;
use serde::{Deserialize, Serialize};

mod perf_event;
mod hwmon;
mod drm_devices;
mod drm_drivers;
mod drm_fdinfo;
mod proc_info;
mod drm_clients;
mod app_data;
mod app;
mod plotter;

use drm_devices::DrmDevices;
use app_data::{AppDataLive, AppDataJson};
use app::App;
use plotter::Plotter;


/// qmassa! - display DRM clients usage stats
#[derive(Parser, Clone, Debug, Deserialize, Serialize)]
#[command(version, about, long_about = None)]
pub struct CliArgs {
    /// show only specific PCI device (default: all devices)
    #[arg(short, long)]
    dev_slot: Option<String>,

    /// base for process tree [default: all accessible pids' info]
    #[arg(short, long)]
    pid: Option<String>,

    /// ms interval between updates
    #[arg(short, long, default_value = "1500")]
    ms_interval: u64,

    /// show all DRM clients [default: only active]
    #[arg(short, long, action = ArgAction::SetTrue)]
    all_clients: bool,

    /// number of stats updates/iterations
    #[arg(short, long, default_value = "-1")]
    nr_iterations: i32,

    /// save stats to a JSON file
    #[arg(short, long)]
    to_json: Option<String>,

    /// file to log to when RUST_LOG is used [default: stderr (if not tty) or qmassa-<pid>.log]
    #[arg(short, long)]
    log_file: Option<String>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand, Clone, Debug, Deserialize, Serialize)]
enum Command
{
    /// replay from a JSON file
    Replay(ReplayArgs),

    /// Plots charts from JSON data
    Plot(PlotArgs)
}

#[derive(Args, Clone, Debug, Deserialize, Serialize)]
struct ReplayArgs
{
    json_file: String,
}

#[derive(Args, Clone, Debug, Deserialize, Serialize)]
struct PlotArgs
{
    /// Input JSON file
    #[arg(short, long)]
    json_file: String,

    /// Output PNG file
    #[arg(short, long)]
    png_file: String,

    /// Optional comma-separated list of charts to be plotted
    #[arg(short, long)]
    charts: Option<String>,
}

fn run_replay_cmd(args: ReplayArgs) -> Result<()>
{
    // get app data from JSON file
    let jsondata = AppDataJson::from(&args.json_file)
        .context("Failed to load data from JSON file")?;

    // create tui app and run the mainloop
    let mut app = App::from(Rc::new(RefCell::new(jsondata)));
    app.run()?;

    Ok(())
}

fn run_plot_cmd(args: PlotArgs) -> Result<()>
{
    println!("Plotting charts from {} to {}", &args.json_file, &args.png_file);

    // get app data from JSON file
    let jsondata = AppDataJson::from(&args.json_file)
        .context("Failed to load data from JSON file")?;

    let charts_filter = args.charts.clone()
        .map(|s| s.split(',')
        .map(|s| s.to_string())
        .collect());

    // create plotter and plot the charts
    let plotter = Plotter::new(
        jsondata,
        args.png_file.to_string(),
        charts_filter,
    );
    plotter.plot()?;

    Ok(())
}

fn run_tui_cmd(args: CliArgs) -> Result<()>
{
    let base_pid: String;
    if args.pid.is_some() {
        base_pid = args.pid.clone().unwrap();
    } else {
        // base_pid is not set, pick value depending on user:
        //   root       => "1", to scan process tree for whole system
        //   non-root   => "", all processes with accessible info are scanned
        let euid: u32 = unsafe { libc::geteuid() };
        base_pid = if euid == 0 { String::from("1") } else { String::from("") };
    }

    // find all DRM subsystem devices
    let mut qmds = DrmDevices::find_devices()
        .context("Failed finding DRM devices")?;
    if qmds.is_empty() {
        bail!("No DRM devices found");
    }
    // get DRM clients from pid process tree starting at base_pid
    qmds.set_clients_pid_tree(base_pid.as_str())
        .context("Failed to set DRM clients pid tree")?;

    // get app data from live system info
    let appdata = AppDataLive::from(args, qmds);

    // create tui app and run its mainloop
    let mut app = App::from(Rc::new(RefCell::new(appdata)));
    app.run()?;

    Ok(())
}

fn main() -> Result<()>
{
    // parse command-line args
    let args = CliArgs::parse();

    // set up logging for all subcommands (if needed)
    if env::var_os(env_logger::DEFAULT_FILTER_ENV).is_some() {
        let mut logger = env_logger::Builder::from_default_env();
        let fname: &Path;

        if args.log_file.is_none() && !io::stderr().is_terminal() {
            logger.init();
        } else {
            let mut fnstr: String;

            if let Some(log_file) = &args.log_file {
                fname = Path::new(log_file);
            } else {
                // stderr is a tty/terminal
                fnstr = env::current_exe()
                    .expect("Failed to get current process name")
                    .file_name().unwrap().to_str().unwrap().to_string();
                fnstr.push_str("-");
                fnstr.push_str(&process::id().to_string());
                fnstr.push_str(".log");

                fname = Path::new(&fnstr);
            }

            let logtarget = Box::new(File::create(fname)
                .expect("Can't create log file"));
            logger.target(env_logger::Target::Pipe(logtarget));
            logger.init();
        }
    }

    if let Some(cmd) = args.command {
        match cmd {
            Command::Replay(cmd_args) => {
                run_replay_cmd(cmd_args)
            },
            Command::Plot(cmd_args) => {
                run_plot_cmd(cmd_args)
            },
        }
    } else {
        run_tui_cmd(args)
    }
}
