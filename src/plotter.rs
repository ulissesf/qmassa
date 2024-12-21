use std::collections::HashMap;

use anyhow::Result;
use plotters::prelude::*;

use crate::app_data::AppDataJson;


pub struct Plotter
{
    jsondata: AppDataJson,
    output_file: String,
    charts_filter: Option<Vec<String>>,
}

impl Plotter
{
    pub fn new(jsondata: AppDataJson, output_file: String,
        charts_filter: Option<Vec<String>>) -> Plotter
    {
        Plotter {
            jsondata,
            output_file,
            charts_filter,
        }
    }

    pub fn plot(&self) -> Result<()>
    {
        let devices = self.jsondata.states();

        let metrics = vec![
            "min_freq", "cur_freq", "act_freq", "max_freq", "gpu_cur_power",
            "pkg_cur_power", "smem_used", "vram_used", "ccs", "rcs", "vecs",
            "vcs", "bcs",
        ];

        let mut valid_charts: HashMap<String, Vec<(String, String, f64, usize, f64)>> = HashMap::new();

        let mut timestamp: usize = 0;

        for device in devices {
            if let Some(last_timestamp) = device.timestamps.back() {
                timestamp = *last_timestamp as usize;
            }
            for dev_state in &device.devs_state {
                let pci_dev = &dev_state.pci_dev;
                let dev_name = &dev_state.vdr_dev_rev;
                let stats = &dev_state.dev_stats;
                let freq_limits_max = &dev_state.freq_limits[0].maximum;

                for metric_name in &metrics {
                    if let Some(filter) = &self.charts_filter {
                        if !filter.contains(&metric_name.to_string()) {
                            continue;
                        }
                    }

                    let mut values = vec![];
                    match *metric_name {
                        "min_freq" => values.extend(
                            stats.freqs.iter().map(|f| f[0].min_freq as f64),
                        ),
                        "cur_freq" => values.extend(
                            stats.freqs.iter().map(|f| f[0].cur_freq as f64),
                        ),
                        "act_freq" => values.extend(
                            stats.freqs.iter().map(|f| f[0].act_freq as f64),
                        ),
                        "max_freq" => values.extend(
                            stats.freqs.iter().map(|f| f[0].max_freq as f64),
                        ),
                        "gpu_cur_power" => values.extend(
                            stats.power.iter().map(|p| p.gpu_cur_power as f64),
                        ),
                        "pkg_cur_power" => values.extend(
                            stats.power.iter().map(|p| p.pkg_cur_power as f64),
                        ),
                        "smem_used" => values.extend(
                            stats.mem_info.iter().map(|m| m.smem_used as f64),
                        ),
                        "vram_used" => values.extend(
                            stats.mem_info.iter().map(|m| m.vram_used as f64),
                        ),
                        engine_name if stats.eng_stats.contains_key(engine_name) => {
                            if let Some(engine_stats) =
                                stats.eng_stats.get(engine_name)
                            {
                                values.extend(
                                    engine_stats.usage.iter().map(|&u| u as f64),
                                );
                            }
                        }
                        _ => {
                            println!(
                                "Metric does not match any known category: {}",
                                metric_name
                            );
                        }
                    }

                    if let Some(last_value) = values.last() {
                        if *last_value > 0.0 {
                            valid_charts
                                .entry(pci_dev.clone())
                                .or_insert_with(Vec::new)
                                .push((
                                    dev_name.to_string(),
                                    metric_name.to_string(),
                                    *last_value,
                                    timestamp,
                                    *freq_limits_max as f64,
                                ));
                        }
                    }
                }
            }
        }

        let max_x = timestamp;
        let mut max_y;

        let mut max_power = f64::NEG_INFINITY;
        let mut max_mem = f64::NEG_INFINITY;

        let mut metrics_grouped: HashMap<
            String,
            Vec<(String, String, f64, usize, f64)>,
        > = HashMap::new();

        for (pci_dev, metrics) in &valid_charts {
            for (dev_name, metric_name, value, timestamp, freq_max) in metrics {
                metrics_grouped
                    .entry(metric_name.clone())
                    .or_insert_with(Vec::new)
                    .push((
                        pci_dev.clone(),
                        dev_name.clone(),
                        *value,
                        *timestamp,
                        freq_max.clone(),
                    ));

                if metric_name.contains("_power") {
                    max_power = max_power.max(*value);
                } else if metric_name.contains("mem") {
                    max_mem = max_mem.max(*value);
                }
            }
        }

        let cols = 1;
        let rows = metrics_grouped.len();
        let root = BitMapBackend::new(&self.output_file, (1200, 2000))
            .into_drawing_area();
        root.fill(&WHITE)?;
        let areas = root.split_evenly((rows, cols));

        for ((metric_name, data), area) in metrics_grouped.into_iter().zip(areas)
        {
            match metric_name.clone() {
                name if name.contains("_freq") => {
                    max_y = data
                        .iter()
                        .map(|(_, _, value, _, _)| *value)
                        .fold(f64::NEG_INFINITY, f64::max);
                }
                name if name.contains("_power") => {
                    max_y = max_power;
                }
                name if name.contains("_mem") => {
                    max_y = max_mem;
                }
                _ => {
                    max_y = 100.0;
                }
            }

            let mut chart = ChartBuilder::on(&area)
                .caption(
                    format!(
                        "{} for {}",
                        metric_name,
                        data.first()
                            .map(|(_, dev_name, _, _, _)| dev_name)
                            .unwrap_or(&String::new())
                    ),
                    ("sans-serif", 12),
                )
                .x_label_area_size(20)
                .y_label_area_size(40)
                .build_cartesian_2d(0..max_x as i32, 0.0..max_y)?;

            chart.configure_mesh().draw()?;

            let points: Vec<(i32, f64)> = data
                .iter()
                .map(|(_, _, value, timestamp, _)| (*timestamp as i32, *value))
                .collect();

            chart.draw_series(LineSeries::new(points, &BLUE))?;
        }

        root.present()?;
        println!("Charts saved to {}", self.output_file);
        Ok(())
    }
}
