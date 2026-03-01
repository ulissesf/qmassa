# Rust tools to monitor GPU stats on Linux

## Tools

This repository has the 2 main tools below that share the internal [qmlib](qmlib) library.

### [qmassa!](qmassa) - TUI tool to display GPU usage stats <a title="qmassa" target="_blank" href="https://crates.io/crates/qmassa"><img alt="qmassa" src="https://img.shields.io/crates/v/qmassa"></a>

![qmassa](https://github.com/ulissesf/qmassa/blob/assets/assets/qmassa.gif?raw=true)

### [qmmd!](qmmd) - Prometheus HTTP endpoint to export GPU usage metrics <a title="qmmd" target="_blank" href="https://crates.io/crates/qmmd"><img alt="qmmd" src="https://img.shields.io/crates/v/qmmd"></a>

Below is an example of the metrics exported by qmmd.

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

## License

Copyright © 2024-2026 Ulisses Furquim

This project is distributed under the terms of the Apache License (Version 2.0).
See [LICENSE](LICENSE) for details.
