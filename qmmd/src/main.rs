use core::net::{IpAddr, SocketAddr};
use std::collections::HashMap;
use std::env;
use std::fs::File;
use std::path::Path;
use std::thread;
use std::time;

use anyhow::{bail, Context, Result};
use clap::{ArgAction, Parser};
use env_logger;
use log::info;
use metrics_exporter_prometheus::PrometheusBuilder;

use qmlib::drm_devices::DrmDevices;

mod stats_ctrl;
use stats_ctrl::StatsCtrl;


/// qmmd! - qmassa metrics daemon
#[derive(Parser, Clone, Debug)]
#[command(version, about, long_about = None)]
struct CliArgs {
    /// Select specific devices (comma-separated list) [default: all devices]
    #[arg(short, long)]
    dev_slots: Option<String>,

    /// Interval between updates in ms
    #[arg(short, long, default_value = "1500")]
    ms_interval: u64,

    /// Number of stats updates/iterations
    #[arg(short, long, default_value = "-1")]
    nr_iterations: i32,

    /// IP to register endpoint's HTTP listener
    #[arg(short, long, default_value = "0.0.0.0")]
    ip: IpAddr,

    /// Port to register endpoint's HTTP listener
    #[arg(short, long, default_value = "9000")]
    port: u16,

    /// Use DRM fdinfo for engines and memory usage [default: no fdinfo]
    #[arg(short = 'f', long, action = ArgAction::SetTrue)]
    use_fdinfo: bool,

    /// File to log to when RUST_LOG is used [default: stderr]
    #[arg(short, long)]
    log_file: Option<String>,

    /// Options for DrmDriver in qmmd (can be passed multiple times) [default: no options]
    #[arg(short = 'o', long)]
    drv_options: Option<Vec<String>>,
}

fn export_stats_loop(args: CliArgs, qmds: DrmDevices) -> Result<()>
{
    let ival = time::Duration::from_millis(args.ms_interval);
    let max_iterations = args.nr_iterations;
    let skaddr = SocketAddr::new(args.ip, args.port);

    let _pmb = PrometheusBuilder::new();
    let _pmb = _pmb.with_http_listener(skaddr);
    _pmb.install().expect("failed to install recorder/exporter");

    let mut ctl = StatsCtrl::from(qmds);

    info!("Prometheus HTTP endpoint listener on IP {}, Port {}, \
        updated metrics every {} ms", args.ip, args.port, args.ms_interval);

    let mut nr = 0;
    loop {
        if max_iterations >= 0 && nr == max_iterations {
            break;
        }

        // refresh and publish stats
        ctl.iterate()?;
        nr += 1;

        // sleep till next iteration
        thread::sleep(ival);
    }

    Ok(())
}

fn main() -> Result<()>
{
    // parse command-line args
    let args = CliArgs::parse();

    // set up logging
    if env::var_os(env_logger::DEFAULT_FILTER_ENV).is_some() {
        let mut logger = env_logger::Builder::from_default_env();

        if let Some(log_file) = &args.log_file {
            let fname = Path::new(log_file);
            let logtarget = Box::new(File::create(fname)
                .expect("Can't create log file"));
            logger.target(env_logger::Target::Pipe(logtarget));
        }

        logger.init();
    }

    info!("Starting: v{}, {:?}", env!("CARGO_PKG_VERSION"), &args);

    // process devslots and driver options
    let slots_str: &str;
    let mut slots_lst: Vec<&str> = Vec::new();
    if args.dev_slots.is_some() {
        slots_str = args.dev_slots.as_ref().unwrap();
        slots_lst = slots_str.split(',').collect();
    }

    let mut drv_opts: HashMap<&str, Vec<&str>> = HashMap::new();
    if !args.use_fdinfo {
        // xe and i915 need to get engines usage from perf PMU
        drv_opts.insert("xe", vec!["engines=pmu",]);
        drv_opts.insert("i915", vec!["engines=pmu",]);
        // amdgpu needs to get engines usage from sysfs
        drv_opts.insert("amdgpu", vec!["engines=sysfs",]);
    }

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

    // if use_fdinfo get DRM clients from whole system
    if args.use_fdinfo {
        qmds.set_clients_pid_tree("1")
            .context("Failed to set DRM clients pid tree (base pid: 1)")?;
    }

    // export stats!
    export_stats_loop(args, qmds)
}
