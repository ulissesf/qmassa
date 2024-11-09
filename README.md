# qmassa!

<div align="center">
[!https://img.shields.io/crates/v/qmassa?logo=rust&style=flat-square&logoColor=E05D44&color=E05D44][https://crates.io/crates/qmassa]
</div>

qmassa is a Rust terminal-based tool for displaying GPUs usage stats on Linux.

![qmassa](http://honeh.4kim.org/qmassa/qmassa-v0.2.2.gif)

## General description

qmassa tries to display as much device and DRM clients (processes using the
GPU) information as possible. Command-line options and which user is running
the tool control how much can be displayed.

Most of the information is gathered through a GPU vendor and driver agnostic
interface such as standard files in /proc and /sys or by using udev. For some
of the stats, though, a driver-specific way is needed, and qmassa then
leverages what the kernel drivers expose in their uAPI (e.g. specific query
ioctls), specific sysfs files/directories or through perf events.

## How to install it

The recommendation is to install qmassa using cargo. If you want to install
the latest release on crates.io and using qmassa's lock file:

```shell
cargo install --locked qmassa
```

If you want to install the latest development version using qmassa's lock file:

```shell
cargo install --locked --git https://github.com/ulissesf/qmassa
```

## How to use it

> [!IMPORTANT]
> If you want to run qmassa as a non-root user, it needs to be added to at
> least the video, render, and power groups (or equivalent ones in your Linux
> distribution). That is needed so qmassa can open the DRM device nodes to
> collect information from ioctls. If your user is not in the right groups
> you'll likely get "Permission denied" errors.

Running it as non-root user and wihout any command-line options will display
limited device usage information and the DRM clients stats from processes that
user has access to in /proc.

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

Running for only 5 iterations (UI updates).

```shell
sudo qmassa -n 5
```

Changing the interval between updates to 1s (1000 ms).

```shell
sudo qmassa -m 1000
```

Showing all DRM clients including the inactive ones (no memory allocated or
engines being used).

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
| [Engines]    | Overall engine usage in the last iteration     |
| POWER        | GPU power usage / Package power usage          |

The memory usage values are either in bytes (no letter), or in KiB
(using "K" letter), or in MiB (using "M" letter), or in GiB (using "G"
letter). The values are rounded to be easily displayed in a small space,
but if you save the stats to a JSON file you can get them all in bytes.

The overall engines usage depends on the DRM clients that the user has
access to. In order to have a system view, please run qmassa as root.

The intention of the power reporting is to have values that are the
closest possible to the power usage from both the GPU and the larger package
(or card) containing it. It's good to remember that larger package is
different on integrated vs discrete GPUs, and there are limitations on what
drivers expose and what they have visibility on so expect the information
to vary a lot across GPUs and vendors. All the power usage values are in
watt (W).

The frequency graph ranges from min to max values and plots the instant
driver-requested (if supported) and actual device frequency for each
iteration. The graph legend shows the latest value for those frequencies.
The graph also indicates the overall status and PL1 throttle reason (for
now only valiid on i915 and Xe drivers). All the frequency values are in
MHz.

#### Driver support

The table below shows the current drivers and features supported in qmassa
to get device information.

| Driver | Dev Type | Mem Info | Engines | Freqs   | Power   | Client Mem Info |
| ------ | -------- | -------- | ------- | ------- | ------- | --------------- |
| xe     | :white_check_mark: | :white_check_mark: | :white_check_mark: | :white_check_mark: | :white_check_mark: | :white_check_mark: |
| i915   | :white_check_mark: | :white_check_mark: | :white_check_mark: | :white_check_mark: | :white_check_mark: | :white_check_mark: |
| amdgpu | :white_check_mark: | :white_check_mark: | :white_check_mark: | :white_check_mark: |                    |                    |

qmassa is tested on some Intel and AMD GPUs but it relies heavily on kernel
drivers exposing consistent support across GPUs. If you have a problem,
please file an issue so we can debug it.

#### Limitations

* i915: the kernel driver doesn't track/report system memory used and thus
qmassa can't display it.

### Per DRM client

| Field        | Description                                       |
| ------------ | ------------------------------------------------- |
| PID          | Process ID                                        |
| SMEM         | Resident amount of system memory                  |
| VRAM         | Resident amount of device memory                  |
| MIN          | Minor number of /dev/dri device node being used   |
| [Engines]    | Engine usage in the last iteration                |
| CPU          | Process' overall CPUs usage in the last iteration |
| COMMAND      | [/proc/PID/comm] /proc/PID/cmdline                |

The memory usage for DRM clients follow the same format and units as
described in the previous per device section. All the values can also
be found in bytes when stats are saved to a JSON file.

The engines reported are driver and vendor specific, and are read directly
from DRM fdinfo files in /proc.

The CPU usage is measured by how much CPU time that process used versus the
total available CPU time across all online CPUs in the system for that
iteration. The total available CPU time is the time between two samples
multiplied by the number of online CPUs. This allows this value to stay
between 0% and 100%.

## Acknowledgements

qmassa uses <a href="https://ratatui.rs/">Ratatui</a> for displaying a nice
terminal-based UI and leverages [many](Cargo.toml) other Rust crates.

## License

Copyright © 2024 Ulisses Furquim

This project is distributed under the terms of the Apache License (Version 2.0).
See [LICENSE](LICENSE) for details.
