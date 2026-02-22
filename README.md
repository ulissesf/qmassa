# Rust tools to monitor GPU usage stats on Linux

## Tools

This repository has the 2 main tools below that share the internal [qmlib](qmlib) library.

### [qmassa!](qmassa) - TUI tool to display GPU usage stats <a title="qmassa" target="_blank" href="https://crates.io/crates/qmassa"><img alt="qmassa" src="https://img.shields.io/crates/v/qmassa"></a>

![qmassa](https://github.com/ulissesf/qmassa/blob/assets/assets/qmassa.gif?raw=true)

### [qmmd!](qmmd) - Prometheus HTTP endpoint to export GPU usage metrics <a title="qmmd" target="_blank" href="https://crates.io/crates/qmmd"><img alt="qmmd" src="https://img.shields.io/crates/v/qmmd"></a>

Below is an example of the metrics exported by qmmd.

```shell
# TYPE qmmd_gpu_info counter
qmmd_gpu_info{device="0000:03:00.0",pci_id="8086:E212",vendor_name="Intel Corporation",device_name="Battlemage G21 [Arc Pro B50]",revision="00",driver_name="xe",dev_type="Discrete (PF)",dev_nodes="/dev/dri/card1,/dev/dri/renderD128"} 1

# TYPE qmmd_gpu_smem_used_bytes gauge
qmmd_gpu_smem_used_bytes{device="0000:03:00.0"} 11489280

# TYPE qmmd_gpu_smem_total_bytes gauge
qmmd_gpu_smem_total_bytes{device="0000:03:00.0"} 67253104640

# TYPE qmmd_gpu_vram_used_bytes gauge
qmmd_gpu_vram_used_bytes{device="0000:03:00.0"} 853356544

# TYPE qmmd_gpu_vram_total_bytes gauge
qmmd_gpu_vram_total_bytes{device="0000:03:00.0"} 17095983104

# TYPE qmmd_gpu_engine_utilization_pct gauge
qmmd_gpu_engine_utilization_pct{device="0000:03:00.0",name="bcs"} 0
qmmd_gpu_engine_utilization_pct{device="0000:03:00.0",name="ccs"} 96.34436390193495
qmmd_gpu_engine_utilization_pct{device="0000:03:00.0",name="rcs"} 2.4435453317647053
qmmd_gpu_engine_utilization_pct{device="0000:03:00.0",name="vcs"} 0
qmmd_gpu_engine_utilization_pct{device="0000:03:00.0",name="vecs"} 0

# TYPE qmmd_gpu_frequency_mhz gauge
qmmd_gpu_frequency_mhz{device="0000:03:00.0",name="gt0"} 1700
qmmd_gpu_frequency_mhz{device="0000:03:00.0",name="gt1"} 750

# TYPE qmmd_gpu_power_watts gauge
qmmd_gpu_power_watts{device="0000:03:00.0"} 51.71729378231119

# TYPE qmmd_gpu_package_power_watts gauge
qmmd_gpu_package_power_watts{device="0000:03:00.0"} 70.05370146692682

# TYPE qmmd_gpu_temperature_celsius gauge
qmmd_gpu_temperature_celsius{device="0000:03:00.0",name="mctrl"} 64
qmmd_gpu_temperature_celsius{device="0000:03:00.0",name="pcie"} 61
qmmd_gpu_temperature_celsius{device="0000:03:00.0",name="pkg"} 64
qmmd_gpu_temperature_celsius{device="0000:03:00.0",name="vram"} 54
qmmd_gpu_temperature_celsius{device="0000:03:00.0",name="vram_ch_0"} 54
qmmd_gpu_temperature_celsius{device="0000:03:00.0",name="vram_ch_1"} 52
qmmd_gpu_temperature_celsius{device="0000:03:00.0",name="vram_ch_2"} 54
qmmd_gpu_temperature_celsius{device="0000:03:00.0",name="vram_ch_3"} 54
qmmd_gpu_temperature_celsius{device="0000:03:00.0",name="vram_ch_4"} 52
qmmd_gpu_temperature_celsius{device="0000:03:00.0",name="vram_ch_5"} 52
qmmd_gpu_temperature_celsius{device="0000:03:00.0",name="vram_ch_6"} 54
qmmd_gpu_temperature_celsius{device="0000:03:00.0",name="vram_ch_7"} 52

# TYPE qmmd_gpu_fan_speed_rpm gauge
qmmd_gpu_fan_speed_rpm{device="0000:03:00.0",name="1"} 2319
```

## License

Copyright © 2024-2026 Ulisses Furquim

This project is distributed under the terms of the Apache License (Version 2.0).
See [LICENSE](LICENSE) for details.
