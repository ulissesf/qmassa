# Changelog

## [v1.2.0](https://github.com/ulissesf/qmassa/releases/tag/v1.2.0) - 2025-12-13

### Features

- CLI option and TUI toggle to show PCI IDs or names. (@ulissesf)
- Simplify TUI when no qmassa driver is available. (@ulissesf)
- Support non-PCI GPU DRM devices. (@ulissesf)
- Support all Rust libc options. (@ulissesf)

### Fixes

- Show only existing HW engines with i915 PMU. (@ulissesf)
- Fix crash with musl and tested on aarch64. (@ulissesf)

## [v1.1.0](https://github.com/ulissesf/qmassa/releases/tag/v1.1.0) - 2025-11-02

### Features

- Optimize JSON output for improved stream processing. (@ulissesf)

### Fixes

- Fix Rust 1.89 lifetime ellision warnings. (@ulissesf)
- Fix engines PMU with Xe driver when GPU doesn't support SR-IOV. (@ulissesf)

## [v1.0.0](https://github.com/ulissesf/qmassa/releases/tag/v1.0.0) - 2025-06-22

### Features

- SR-IOV support for Intel Xe DrmDriver PMU engine utilization reporting. (@ulissesf)
- TUI toggles to list all DRM clients and grouped by PID. (@ulissesf)
- TUI option to display DRM clients grouped by PID. (@ulissesf)
- Intel i915 & Xe DrmDrivers expose engines utlization from PMU. (@ulissesf)
- DrmDrivers can receive command-line options. (@ulissesf)
- DrmDrivers can report engines utilization instead of system's DRM fdinfo. (@ulissesf)
- Add prefix to temps/fans chart legends. (@ulissesf)

### Fixes

- Don't crash when no GPU frequencies are reported. (@ulissesf)
- Show stable and ordered list of Hwmon temps/fans. (@ulissesf)

## [v0.7.0](https://github.com/ulissesf/qmassa/releases/tag/v0.7.0) - 2025-04-20

### Features

- Stop qmassa when monitored PID exits. (@ulissesf)
- TUI app and plot sub-command accept a list of PCI devices. (@ulissesf)
- Display and plot fan speeds for Intel and AMD discrete GPUs. (@ulissesf)
- Display and plot temperatures for Intel and AMD discrete GPUs. (@ulissesf)

### Fixes

- Handle states with no engine stats when plotting to SVG. (@ulissesf)
- i915: check if throttle reason files exist. (@ulissesf)
- Add macro to retry interrupted ioctl()s. (@ulissesf)
- Reduce and tune dependencies to keep MSRV on v1.74. (@ulissesf)
- Don't reset engine capacity that led to wrong engine utilization. (@ulissesf)
- Fix crash when running with no qmassa driver implementation. (@ulissesf)

## [v0.6.0](https://github.com/ulissesf/qmassa/releases/tag/v0.6.0) - 2025-02-01

### Features

- Display the client's ID in the DRM clients list. (@ulissesf)
- Handle system memory region "memory" as a no driver fallback. (@ulissesf)
- UI polish: use CARD instead of PKG on discrete GPUs power charts. (@ulissesf)

### Fixes

- Require both cycles and total-cycles for engine utilization. (@ulissesf)

## [v0.5.0](https://github.com/ulissesf/qmassa/releases/tag/v0.5.0) - 2025-01-11

### Features

- Make it visible in the TUI when a PID tree is being monitored. (@ulissesf)
- Add replay sub-command to display TUI from JSON file. (@ulissesf)
- Add plot sub-command to generate SVG charts for device stats. (@rodrigovivi and @ulissesf)
- Only show VRAM data for discrete GPUs. (@ulissesf)
- Display all frequencies supported by drivers like gfx, media, etc. (only i915 and Xe for now). (@ulissesf)
- Add no TUI mode to just record stats. (@ulissesf)

### Fixes

- Better handling of invalid PIDs. (@ulissesf)
- Fix off-by-one error leading to invalid JSON files. (@ulissesf)
- Fix typos in README docs. (@ccallawa-intel)
- Set MSRV to 1.74.0 and keep lock file in version 3. (@ulissesf)
- Report usage relative to single CPU not whole system. (@ulissesf)

## [v0.4.0](https://github.com/ulissesf/qmassa/releases/tag/v0.4.0) - 2024-12-10

### Features

- Add multiple screens support. (@ulissesf)
- Highlight and selection of DRM client in the list. (@ulissesf)
- Display DRM client stats and charts in a separate screen. (@ulissesf)
- Update stats at fixed intervals and render UI whenever needed (new data or reacting to user events). (@ulissesf)

## [v0.3.0](https://github.com/ulissesf/qmassa/releases/tag/v0.3.0) - 2024-11-26

### Features

- GPU power reporting for AMD dGPUs through Hwmon. (@ulissesf)
- Display charts for all device statistics available. (@ulissesf)
- Gets power for Intel iGPUs from MSR as fallback for no perf event support. (@ulissesf)
- Minor UI improvements: better readability of long engine names; frozen headers in the DRM clients list. (@ulissesf)

### Fixes

- Do not override PL1 with Status in frequency chart. (@ulissesf)

## [v0.2.3](https://github.com/ulissesf/qmassa/releases/tag/v0.2.3) - 2024-11-11

### Features

- Same as v0.2.2.

### Fixes

- Fixes README links on GitHub and crates.io. (@ulissesf)

## [v0.2.2](https://github.com/ulissesf/qmassa/releases/tag/v0.2.2) - 2024-11-11

### Features

- Displays power usage for GPU and package on Intel integrated and discrete GPUs. (@ulissesf, leveraging initial work on Hwmon from @rodrigovivi)
- Added initial amdgpu support. (@ulissesf)
- Improved visibility of very long GPU device names (@ulissesf)
- Improved GPU frequency graph: using Braille markers, moved legend out of the way of the newest data, increased number of data points plotted, showing latest requested and actual frequencies in the legend. (@ulissesf)

### Fixes

- Don't abort when perf event is not supported. (@ulissesf)
- Make parsing DRM fdinfo files more robust and fix crash on amdgpu. (@ulissesf)

## [v0.2.1](https://github.com/ulissesf/qmassa/releases/tag/v0.2.1) - 2024-10-26

### Features

- Displays CPU usage per DRM client process (@ulissesf)
- Displays "Unknown" device type when information can't be retrieved from driver (@ulissesf)
- Add throttle reasons support and shows status and PL1 ones in the device frequency graph (@rodrigovivi)
- Shows overall engines usage per device (@ulissesf)
- Completed i915 support to display device type and available memory usage info (@ulissesf)

### Fixes

- Fix crashes running with release builds due to missing mutable bindings in ioctl()s unsafe code. (@ulissesf)

## [v0.2.0](https://github.com/ulissesf/qmassa/releases/tag/v0.2.0) - 2024-10-14

Initial version of qmassa!

### Features

- Initial UI with DRM GPU devices in tabs and a scrollable DRM client list (@ulissesf)
  - Displays device basic system info, memory usage and frequency graph
  - Displays DRM clients PID, device node minor number, memory usage, engines usage and command info
- Supports getting data from Xe driver and partially from i915 (@ulissesf)
- Basic command-line options to control update cadence, iterations, and saving data to JSON file (@ulissesf)

### Fixes

- Fix crash with devices with no PCI ID (@cmarcelo)
