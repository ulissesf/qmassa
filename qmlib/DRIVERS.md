## Driver support

The table below shows the current drivers and features supported to collect
GPU device and DRM client stats.

| Driver | Dev Type | Mem Info | Engines | Freqs   | Power   | Client Mem Info | Temperatures | Fans |
| ------ | :------: | :------: | :-----: | :-----: | :-----: | :-------------: | :----------: | :--: |
| xe     | :white_check_mark: | :white_check_mark: | :white_check_mark: (via DRM fdinfo or perf PMU) | :white_check_mark: (via sysfs or perf PMU) | :white_check_mark: (iGPUs: via perf PMU or MSR, dGPUs: via hwmon) | :white_check_mark: | :white_check_mark: (only dGPUs via hwmon, Linux kernel 6.15+) | :white_check_mark: (only dGPUs via hwmon, Linux kernel 6.16+) |
| i915   | :white_check_mark: | :white_check_mark: | :white_check_mark: (via DRM fdinfo or perf PMU) | :white_check_mark: (via sysfs or perf PMU) | :white_check_mark: (iGPUs: via perf PMU or MSR, dGPUs: via hwmon) | :white_check_mark: | :white_check_mark: (only dGPUs via hwmon) | :white_check_mark: (only dGPUs via hwmon) |
| amdgpu | :white_check_mark: | :white_check_mark: | :white_check_mark: (via DRM fdinfo) | :white_check_mark: (via sysfs) | :white_check_mark: (only dGPUs via hwmon) | :white_check_mark: (Linux kernel 6.13+) | :white_check_mark: (only dGPUs via hwmon) | :white_check_mark: (only dGPUs via hwmon) |
| xe-vfio-pci | :white_check_mark: |  | :white_check_mark: (via perf PMU) |  |  |  |  |  |
| *      |  |  | :white_check_mark: (via DRM fdinfo) |  |  | :white_check_mark: (only "memory" region in DRM fdinfo) |  |  |

Testing is done on some Intel and AMD GPUs, but there's an expectation on
kernel drivers exposing consistent support across GPUs. If you have a problem,
please file an issue so we can debug it.

## Driver options

The tables in this section outline which drivers in qmlib can be passed
extra options to control how or from where they report their stats.

| Options for i915               | Description                                |
| ------------------------------ | ------------------------------------------ |
| devslot=<PCI slot or sysname\> | Applies other options to a specific device |
| engines=pmu                    | Engines usage reporting from perf PMU      |
| freqs=pmu                      | Frequencies reporting from perf PMU        |
| power=msr                      | iGPU only: use MSR to report power instead of perf PMU |

| Options for xe                 | Description                                |
| ------------------------------ | ------------------------------------------ |
| devslot=<PCI slot or sysname\> | Applies other options to a specific device |
| engines=pmu                    | Engines usage reporting from perf PMU. Supports SR-IOV functions (PF, VF) and VFIO on Linux kernel 6.19+. |
| freqs=pmu                      | Frequencies reporting from perf PMU        |
| power=msr                      | iGPU only: use MSR to report power instead of perf PMU |

| Options for amdgpu             | Description                                |
| ------------------------------ | ------------------------------------------ |
| devslot=<PCI slot or sysname\> | Applies other options to a specific device |
| engines=sysfs                  | Engines usage reporting from sysfs (*_busy_percent files) |

## Kernel driver limitations/gaps

| Kernel driver | Limitations/gaps                                           |
| ------------- | ---------------------------------------------------------- |
| i915          | Doesn't track/report system memory used.                   |
| amdgpu        | Processes using kfd don't report engines and memory usage through any DRM client fdinfo. |
