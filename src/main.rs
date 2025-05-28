use std::cell::RefCell;
use std::collections::HashMap;
use std::env;
use std::fs::File;
use std::io::{self, IsTerminal};
use std::path::Path;
use std::process;
use std::rc::Rc;
use std::thread;
use std::time;

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
use app_data::{AppData, AppDataLive, AppDataJson};
use app::App;
use plotter::Plotter;


/// qmassa! - Display GPUs usage stats
#[derive(Parser, Clone, Debug, Deserialize, Serialize)]
#[command(version, about, long_about = None)]
pub struct CliArgs {
    /// Show only specific PCI devices (comma-separated list) [default: all devices]
    #[arg(short, long)]
    dev_slots: Option<String>,

    /// Base for process tree [default: all accessible pids' info]
    #[arg(short, long)]
    pid: Option<String>,

    /// Interval between updates in ms
    #[arg(short, long, default_value = "1500")]
    ms_interval: u64,

    /// Number of stats updates/iterations
    #[arg(short, long, default_value = "-1")]
    nr_iterations: i32,

    /// Show all DRM clients [default: only active]
    #[arg(short, long, action = ArgAction::SetTrue)]
    all_clients: bool,

    /// Group DRM client stats by PID [default: split by DRM minor & client ID]
    #[arg(short, long, action = ArgAction::SetTrue)]
    group_by_pid: bool,

    /// Save stats to a JSON file
    #[arg(short, long)]
    to_json: Option<String>,

    /// File to log to when RUST_LOG is used [default: stderr (if not tty) or qmassa-<pid>.log]
    #[arg(short, long)]
    log_file: Option<String>,

    /// Run with no TUI rendering [default: render TUI]
    #[arg(short = 'x', long, action = ArgAction::SetTrue)]
    no_tui: bool,

    /// Options for DrmDriver in qmassa (can be passed multiple times) [default: no options]
    #[arg(short = 'o', long)]
    drv_options: Option<Vec<String>>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand, Clone, Debug, Deserialize, Serialize)]
enum Command
{
    /// Replay from a JSON file
    Replay(ReplayArgs),

    /// Plot charts from a JSON file
    Plot(PlotArgs)
}

#[derive(Args, Clone, Debug, Deserialize, Serialize)]
struct ReplayArgs
{
    /// Input JSON file
    #[arg(short, long)]
    json_file: String,
}

#[derive(Args, Clone, Debug, Deserialize, Serialize)]
struct PlotArgs
{
    /// Input JSON file
    #[arg(short, long)]
    json_file: String,

    /// Prefix for output SVG files
    #[arg(short, long)]
    out_prefix: String,

    /// Plot only specific PCI devices (comma-separated list) [default: all devices]
    #[arg(short, long)]
    dev_slots: Option<String>,

    /// Charts to be plotted (comma-separated, possible values: meminfo,
    ///  engines, freqs, power, temps, fans) [default: all charts]
    #[arg(short, long)]
    charts: Option<String>,
}

fn run_replay_cmd(args: ReplayArgs) -> Result<()>
{
    // get app data from JSON file
    let jsondata = AppDataJson::from(&args.json_file)
        .context("Failed to load data from JSON file")?;
    if jsondata.is_empty() {
        bail!("JSON file is empty!");
    }

    // create tui app and run the mainloop
    let mut app = App::from(Rc::new(RefCell::new(jsondata)));
    app.run()?;

    Ok(())
}

fn run_plot_cmd(args: PlotArgs) -> Result<()>
{
    println!("qmassa: Plotting charts from {:?}", args.json_file);

    // get app data from JSON file
    let jsondata = AppDataJson::from(&args.json_file)
        .context("Failed to load data from JSON file")?;
    if jsondata.is_empty() {
        bail!("JSON file is empty!");
    }

    // create plotter and plot the charts
    let plotter = Plotter::from(jsondata,
        args.out_prefix, args.dev_slots, args.charts)?;
    plotter.plot()?;

    Ok(())
}

fn run_notui(mut appdata: AppDataLive) -> Result<()>
{
    if appdata.args().to_json.is_none() && appdata.args().log_file.is_none() {
        println!("qmassa: WARNING: No TUI being rendered but neither \
            logging nor saving JSON stats are enabled!");
    }

    let ival = time::Duration::from_millis(appdata.args().ms_interval);
    let max_iterations = appdata.args().nr_iterations;

    // start saving to JSON file (if requested)
    appdata.start_json_file()?;

    println!("qmassa: Entering no TUI loop, press Ctrl-C to stop.");
    let mut nr = 0;
    loop {
        if max_iterations >= 0 && nr == max_iterations {
            break;
        }

        // refresh stats
        if !appdata.refresh()? {
            break;
        }
        nr += 1;

        // write new state to JSON file (if needed)
        appdata.update_json_file()?;

        // sleep till next iteration
        thread::sleep(ival);
    }

    Ok(())
}

fn run_default_cmd(args: CliArgs) -> Result<()>
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
    let no_tui = args.no_tui;

    let slots_str: &str;
    let mut slots_lst: Vec<&str> = Vec::new();
    if args.dev_slots.is_some() {
        slots_str = args.dev_slots.as_ref().unwrap();
        slots_lst = slots_str.split(',').collect();
    }

    let mut drv_opts: HashMap<&str, Vec<&str>> = HashMap::new();
    if args.drv_options.is_some() {
        for dopt in args.drv_options.as_ref().unwrap().iter() {
            if let Some((drv, opts)) = dopt.split_once('=') {
                drv_opts.entry(drv)
                    .and_modify(|vo| vo.push(opts))
                    .or_insert(vec![opts,]);
            }
        }
    }

    // find all DRM subsystem devices
    let mut qmds = DrmDevices::find_devices(&slots_lst, &drv_opts)
        .context("Failed finding DRM devices")?;
    if qmds.is_empty() {
        bail!("No DRM devices found");
    }
    // get DRM clients from pid process tree starting at base_pid
    qmds.set_clients_pid_tree(base_pid.as_str())
        .context("Failed to set DRM clients pid tree")?;

    // get app data from live system info
    let appdata = AppDataLive::from(args, qmds);

    if no_tui {
        run_notui(appdata)?;
    } else {
        // create tui app and run its mainloop
        let mut app = App::from(Rc::new(RefCell::new(appdata)));
        app.run()?;
    }

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
        run_default_cmd(args)
    }
}
