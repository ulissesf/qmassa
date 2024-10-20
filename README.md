# qmassa!

qmassa is a top-like Rust-based tool for displaying GPU devices usage stats.

## General description and driver support

qmassa tries to display as much device and DRM clients (processes using the
GPU) information as possible. Command-line options and which user is running
the tool control how much can be displayed.

Most of the information is gathered through a GPU vendor and driver agnostic
interface such as standard files in /proc and /sys or by using udev. For some
of the stats, though, a driver-specific way is needed (e.g. specific query
ioctls). The table below shows the current driver support for getting device
stats in qmassa:

| Driver | Support                                                          |
| ------ | ---------------------------------------------------------------- |
| Xe     | Device type & frequencies, Device & DRM clients' resident memory |
| i915   | Device frequencies, DRM client's resident memory                 |

## How to use it

Running it as non-root user and wihout any command-line options will display
limited device usage information, and will show the DRM clients stats from
processes that user has access to in /proc.

```shell
qmassa
```

Running it as the root user and wihout any commnad-line options will display
all the device avaiable stats along with all active DRM clients in the system.

```shell
sudo qmassa
```

Only show a specific GPU device and DRM clients using it. The GPU device
is specified by its PCI device slot name.

```shell
sudo qmassa -d 0000:03:00.0
```

Only show DRM clients from the process tree starting at a specific PID.

```shell
sudo qmassa -p 2876
```

Running only for 5 iterations (UI updates).

```shell
sudo qmassa -n 5
```

Changing the interval between updates to 1s (1000 ms).

```shell
sudo qmassa -m 1000
```

Showing all DRM clients including the inactive ones (no memory allocated or engines being used).

```shell
sudo qmassa -a
```

Saving the stats to a JSON file.

```shell
sudo qmassa -t data.json
```

## Fields description

### Per device

| Field        | Description                                    |
| ------------ | ---------------------------------------------- |
| DRIVER       | Kernel driver being used                       |
| TYPE         | Integrated, Discrete or Unknown                |
| DEVICE NODES | Character device nodes in /dev/dri             |
| SMEM         | System memory used / Total system memory       |
| VRAM         | Device memory used / Total device memory       |

The frequency graph ranges from min to max values and plots the
instant driver-requested and actual device frequency for each iteration.

### Per DRM client

| Field        | Description                                       |
| ------------ | ------------------------------------------------- |
| PID          | Process ID                                        |
| SMEM         | Resident amount of system memory                  |
| VRAM         | Resident amount of device memory                  |
| MIN          | Minor number of /dev/dri/<devnode> used           |
| [Engines]    | Engine usage in the last iteration                |
| CPU          | Process' overall CPUs usage in the last iteration |
| COMMAND      | [/proc/PID/comm] /proc/PID/cmdline                |

The engines reported are driver and vendor specific, and are read directly from
DRM fdinfo files in /proc.

The CPU usage is measured by how much CPU time that process used versus the
total available CPU time across all online CPUs in the system for that
iteration. The total available CPU time is the time between two samples
times the number of online CPUs. This allows this value to stay between 0%
and 100%.

## Acknowledgements

qmassa uses <a href="https://ratatui.rs/">Ratatui</a> for displaying a nice text-based UI and leverages [many](https://github.com/ulissesf/qmassa/blob/main/Cargo.toml) other Rust crates.

## License

This project is distributed under the terms of the Apache License (Version 2.0). See [LICENSE](LICENSE) for details.
