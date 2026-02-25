# qmmd!

<div align="center">
  <a title="qmmd" target="_blank" href="https://crates.io/crates/qmmd"><img alt="qmmd" src="https://img.shields.io/crates/v/qmmd"></a>
</div>

## General description

qmmd is the "qmassa metrics daemon" that provides a Prometheus endpoint over
HTTP to export GPU usage metrics. It uses the same internal library as the
qmassa TUI application. By default, qmmd doesn't scan the /proc filesystem and
process DRM fdinfo files for reporting GPU engines usage and relies on qmlib
implementations for each kernel driver.

## Requirements

The minimum requirements to compile & run qmmd are:

* Compile-time: Rust v1.88 or later, pkg-config and libudev development packages
* Runtime: Linux kernel v6.8 or later to report most usage stats

## How to install it

The recommendation is to install qmmd using cargo. If you want to install
the latest release on crates.io using qmmd's lock file:

```shell
cargo install --locked qmmd
```

If you want to install the latest development version using qmmd's lock file:

```shell
cargo install --locked --git https://github.com/ulissesf/qmassa qmmd
```

## How to use it

> [!IMPORTANT]
> You have to run qmmd as root so it can get all GPU usage metrics. In order
> to properly daemonize and manage it, please run qmmd under a systemd service
> (check the [sample service file](https://github.com/ulissesf/qmassa/blob/main/qmmd/systemd/qmmd.service)).

Export metrics from a specific GPU. The device is specified by its PCI device slot name or its sysname (for non-PCI devices).

```shell
sudo qmmd -d 0000:03:00.0
```

Running for only 5 iterations (stats updates).

```shell
sudo qmmd -n 5
```

Changing the interval between stats updates to 1s (1000 ms).

```shell
sudo qmmd -m 1000
```

Changing the IP to register the HTTP endpoint listener to 192.168.86.32.

```shell
sudo qmmd -i 192.168.86.32
```

Changing the port to register the HTTP endpoint listener to 9090.

```shell
sudo qmmd -p 9090
```

Attempt to use all DRM fdinfo files in the system to calculate GPU engines usage. This option will use more CPU than relying on default qmlib drivers.

```shell
sudo qmmd -f
```

Using perf PMU to report freqs only for 0000:03:00.0 with the xe driver.

```shell
sudo qmmd -o xe=devslot=0000:03:00.0,freqs=pmu
```

## Metrics description

The table below shows all the metric names (with units), description, type,
and labels. They follow the convention commonly used by Prometheus.

| Name:         | qmmd_gpu_info                                               |
| :------------ | :---------------------------------------------------------- |
| Description:  | GPU information metric (one per character device node)      |
| Type:         | Counter (constant value 1)                                  |
| Labels:       | device: PCI slot name or sysname                            |
|               | pci_id: GPU PCI ID (if applicable)                          |
|               | vendor_name: device vendor name from udev HW database       |
|               | device_name: device name from udev HW database              |
|               | revision: device revision                                   |
|               | driver_name: kernel driver being used                       |
|               | dev_type: Integrated, Discrete or Unknown. Virtualization function (PF, VF, or VFIO) when applicable. |
|               | dev_node: character device node (DRI or VFIO)               |

| Name:         | qmmd_gpu_memory_used_bytes                                  |
| :------------ | :---------------------------------------------------------- |
| Description:  | Amount of memory used by GPU                                |
| Type:         | Gauge                                                       |
| Labels:       | device: PCI slot name or sysname                            |
|               | mem_type: "smem" (system memory) or "vram" (device memory)  |

| Name:         | qmmd_gpu_memory_total_bytes                                 |
| :------------ | :---------------------------------------------------------- |
| Description:  | Total memory available for GPU usage                        |
| Type:         | Gauge                                                       |
| Labels:       | device: PCI slot name or sysname                            |
|               | mem_type: "smem" (system memory) or "vram" (device memory)  |

| Name:         | qmmd_gpu_engine_utilization_ratio                           |
| :------------ | :---------------------------------------------------------- |
| Description:  | GPU HW engine utilization ratio in the last iteration       |
| Type:         | Gauge                                                       |
| Labels:       | device: PCI slot name or sysname                            |
|               | engine: GPU HW engine name/ID                               |

| Name:         | qmmd_gpu_actual_frequency_hertz                             |
| :------------ | :---------------------------------------------------------- |
| Description:  | GPU part or specific controller current frequency           |
| Type:         | Gauge                                                       |
| Labels:       | device: PCI slot name or sysname                            |
|               | freq_id: GPU part or controller name/ID                     |

| Name:         | qmmd_gpu_maximum_frequency_hertz                            |
| :------------ | :---------------------------------------------------------- |
| Description:  | GPU part or specific controller maximum frequency           |
| Type:         | Gauge                                                       |
| Labels:       | device: PCI slot name or sysname                            |
|               | freq_id: GPU part or controller name/ID                     |

| Name:         | qmmd_gpu_power_watts                                        |
| :------------ | :---------------------------------------------------------- |
| Description:  | GPU power usage                                             |
| Type:         | Gauge                                                       |
| Labels:       | device: PCI slot name or sysname                            |
|               | domain: "gpu" (actual GPU) or "package" (iGPU: CPU+GPU+SMEM, dGPU: whole card) |

| Name:         | qmmd_gpu_temperature_celsius                                |
| :------------ | :---------------------------------------------------------- |
| Description:  | GPU part or specific controller temperature (only dGPUs)    |
| Type:         | Gauge                                                       |
| Labels:       | device: PCI slot name or sysname                            |
|               | sensor: GPU part or controller sensor                       |

| Name:         | qmmd_gpu_fan_speed_rpm                                      |
| :------------ | :---------------------------------------------------------- |
| Description:  | GPU fan speed (only dGPUs)                                  |
| Type:         | Gauge                                                       |
| Labels:       | device: PCI slot name or sysname                            |
|               | fan_id: GPU fan name/ID                                     |

Below is an example of the metrics exported by the Prometheus HTTP endpoint.

```shell
# TYPE qmmd_gpu_info counter
qmmd_gpu_info{device="0000:03:00.0",pci_id="8086:E212",vendor_name="Intel Corporation",device_name="Battlemage G21 [Arc Pro B50]",revision="00",driver_name="xe",dev_type="Discrete (PF)",dev_node="/dev/dri/card1"} 1
qmmd_gpu_info{device="0000:03:00.0",pci_id="8086:E212",vendor_name="Intel Corporation",device_name="Battlemage G21 [Arc Pro B50]",revision="00",driver_name="xe",dev_type="Discrete (PF)",dev_node="/dev/dri/renderD128"} 1

# TYPE qmmd_gpu_memory_used_bytes gauge
qmmd_gpu_memory_used_bytes{device="0000:03:00.0",mem_type="smem"} 11472896
qmmd_gpu_memory_used_bytes{device="0000:03:00.0",mem_type="vram"} 722874368

# TYPE qmmd_gpu_memory_total_bytes gauge
qmmd_gpu_memory_total_bytes{device="0000:03:00.0",mem_type="smem"} 67253100544
qmmd_gpu_memory_total_bytes{device="0000:03:00.0",mem_type="vram"} 17095983104

# TYPE qmmd_gpu_engine_utilization_ratio gauge
qmmd_gpu_engine_utilization_ratio{device="0000:03:00.0",engine="bcs"} 0
qmmd_gpu_engine_utilization_ratio{device="0000:03:00.0",engine="ccs"} 0.9698813172214144
qmmd_gpu_engine_utilization_ratio{device="0000:03:00.0",engine="rcs"} 0.021016510095973474
qmmd_gpu_engine_utilization_ratio{device="0000:03:00.0",engine="vcs"} 0
qmmd_gpu_engine_utilization_ratio{device="0000:03:00.0",engine="vecs"} 0

# TYPE qmmd_gpu_actual_frequency_hertz gauge
qmmd_gpu_actual_frequency_hertz{device="0000:03:00.0",freq_id="gt0"} 2500000000
qmmd_gpu_actual_frequency_hertz{device="0000:03:00.0",freq_id="gt1"} 1100000000

# TYPE qmmd_gpu_maximum_frequency_hertz gauge
qmmd_gpu_maximum_frequency_hertz{device="0000:03:00.0",freq_id="gt0"} 2600000000
qmmd_gpu_maximum_frequency_hertz{device="0000:03:00.0",freq_id="gt1"} 1500000000

# TYPE qmmd_gpu_power_watts gauge
qmmd_gpu_power_watts{device="0000:03:00.0",domain="gpu"} 33.96474249418945
qmmd_gpu_power_watts{device="0000:03:00.0",domain="package"} 47.56483612825638

# TYPE qmmd_gpu_temperature_celsius gauge
qmmd_gpu_temperature_celsius{device="0000:03:00.0",sensor="mctrl"} 60
qmmd_gpu_temperature_celsius{device="0000:03:00.0",sensor="pcie"} 58
qmmd_gpu_temperature_celsius{device="0000:03:00.0",sensor="pkg"} 59
qmmd_gpu_temperature_celsius{device="0000:03:00.0",sensor="vram"} 52
qmmd_gpu_temperature_celsius{device="0000:03:00.0",sensor="vram_ch_0"} 52
qmmd_gpu_temperature_celsius{device="0000:03:00.0",sensor="vram_ch_1"} 52
qmmd_gpu_temperature_celsius{device="0000:03:00.0",sensor="vram_ch_2"} 52
qmmd_gpu_temperature_celsius{device="0000:03:00.0",sensor="vram_ch_3"} 52
qmmd_gpu_temperature_celsius{device="0000:03:00.0",sensor="vram_ch_4"} 52
qmmd_gpu_temperature_celsius{device="0000:03:00.0",sensor="vram_ch_5"} 50
qmmd_gpu_temperature_celsius{device="0000:03:00.0",sensor="vram_ch_6"} 52
qmmd_gpu_temperature_celsius{device="0000:03:00.0",sensor="vram_ch_7"} 52

# TYPE qmmd_gpu_fan_speed_rpm gauge
qmmd_gpu_fan_speed_rpm{device="0000:03:00.0",fan_id="1"} 2119
```

The supported metrics depend on the qmlib drivers implementation. For
supported drivers and features, available driver options as well as
kernel driver limitations/gaps, please check the
[qmlib drivers](https://github.com/ulissesf/qmassa/blob/main/qmlib/DRIVERS.md)
information.

## License

Copyright © 2026 Ulisses Furquim

This project is distributed under the terms of the Apache License (Version 2.0).
See [LICENSE](https://github.com/ulissesf/qmassa/blob/main/LICENSE) for details.
