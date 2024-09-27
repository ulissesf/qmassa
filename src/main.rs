use std::io::{self, IsTerminal};
use std::path::Path;
use std::fs::File;
use std::process;
use std::env;

use anyhow::{bail, Context, Result};
use env_logger;
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
    ms_interval: u64,

    /// show all DRM clients [default: only active]
    #[arg(short, long, action = ArgAction::SetTrue)]
    all_clients: bool,

    /// number of stats updates/iterations
    #[arg(short, long, default_value = "-1")]
    nr_iterations: i32,

    /// dump stats into text file
    #[arg(short, long)]
    to_txt: Option<String>,

    /// file to log to when RUST_LOG is used [default: stderr (if not tty) or qmassa-<pid>.log]
    #[arg(short, long)]
    log_file: Option<String>,
}

fn main() -> Result<()>
{
    // parse command-line args
    let args = Args::parse();

    // set up logging, if needed
    if env::var_os(env_logger::DEFAULT_FILTER_ENV) != None {
        let mut logger = env_logger::Builder::from_default_env();
        let fname: &Path;

        if args.log_file == None && !io::stderr().is_terminal() {
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

    // if base_pid not set, pick PID depending on user
    //   root       => PID 1
    //   non-root   => oldest parent PID of current process (likely a
    //                 systemd user session or an sshd process)
    // TODO: add PID finding logic here
    let base_pid: String;
    if args.pid == None {
        base_pid = String::from("1");
    } else {
        base_pid = args.pid.clone().unwrap();
    }

    // find all DRM subsystem devices
    let qmds = QmDrmDevices::find_devices()
        .context("Failed to find DRM devices")?;
    if qmds.is_empty() {
        bail!("No DRM devices found");
    }

    // get DRM clients from pid process tree starting at base_pid
    let mut qmclis = QmDrmClients::from_pid_tree(base_pid.as_str());

    // create tui app and run its mainloop
    let mut app = App::new(&qmds, &mut qmclis, &args);
    app.run()?;

    Ok(())
}
