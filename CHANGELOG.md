# Changelog

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
