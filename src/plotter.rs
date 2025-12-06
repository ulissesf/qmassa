use std::cmp::max;

use anyhow::{bail, Result};
use itertools::Itertools;
use plotters::prelude::*;

use crate::app_data::AppDataJson;


#[derive(Debug)]
struct StatData
{
    label: String,
    points: Vec<(f64, f64)>,
}

impl StatData
{
    fn add_point(&mut self, pt: (f64, f64))
    {
        self.points.push(pt);
    }

    fn new(label: &str) -> StatData
    {
        StatData {
            label: label.to_string(),
            points: Vec::new(),
        }
    }
}

const CHART_MEMINFO: usize = 0;
const CHART_ENGINES: usize = 1;
const CHART_FREQS: usize = 2;
const CHART_POWER: usize = 3;
const CHART_TEMPS: usize = 4;
const CHART_FANS: usize = 5;
const CHARTS_TOTAL: usize = 6;

#[derive(Debug)]
pub struct Plotter
{
    jsondata: AppDataJson,
    out_prefix: String,
    dev_slots: Vec<String>,
    sel_charts: [bool; CHARTS_TOTAL],
}

impl Plotter
{
    fn plot_chart(&self, out_file: &str, title: &str,
        x_desc: &str, y_desc: &str, x_max: f64, y_max: f64,
        datasets: &Vec<StatData>) -> Result<()>
    {
        let root = SVGBackend::new(out_file, (1200, 720))
            .into_drawing_area();
        root.fill(&WHITE)?;

        let mut chart = ChartBuilder::on(&root)
            .caption(title, ("sans-serif", (5).percent_height()))
            .set_label_area_size(LabelAreaPosition::Left, (8).percent())
            .set_label_area_size(LabelAreaPosition::Bottom, (4).percent())
            .margin((1).percent())
            .build_cartesian_2d(0.0..x_max, 0.0..y_max)?;
        chart
            .configure_mesh()
            .x_desc(x_desc)
            .y_desc(y_desc)
            .draw()?;

        for (idx, ds) in datasets.iter().enumerate() {
            let color = Palette99::pick(idx).mix(0.9);
            chart
                .draw_series(LineSeries::new(
                    ds.points.iter().map(|&pt| pt),
                    color.stroke_width(3)))?
                .label(&ds.label)
                .legend(move |(x, y)| Rectangle::new([(x, y - 5),
                    (x + 10, y + 5)], color.filled()));
        }
        chart.configure_series_labels().border_style(BLACK).draw()?;

        root.present()?;
        println!("qmassa: Chart {:?} saved to {:?}", title, out_file);

        Ok(())
    }

    // TODO: figure out plotting DRM client stats charts
    pub fn plot(&self) -> Result<()>
    {
        let plot_meminfo = self.sel_charts[CHART_MEMINFO];
        let plot_engines = self.sel_charts[CHART_ENGINES];
        let plot_freqs = self.sel_charts[CHART_FREQS];
        let plot_power = self.sel_charts[CHART_POWER];
        let plot_temps = self.sel_charts[CHART_TEMPS];
        let plot_fans = self.sel_charts[CHART_FANS];
        let nr_devices = self.jsondata
            .states().front().unwrap().devs_state.len();

        for idx in 0..nr_devices {
            let di = &self.jsondata.states().back().unwrap().devs_state[idx];
            if !self.dev_slots.is_empty() &&
                !self.dev_slots.iter().any(|ds| di.pci_dev == *ds) {
                continue;
            }

            let mut meminfo: Vec<StatData> = Vec::new();
            let mut engs_names: Vec<&str> = Vec::new();
            let mut engines: Vec<StatData> = Vec::new();
            let mut freqs: Vec<Vec<StatData>> = Vec::new();
            let mut power: Vec<StatData> = Vec::new();
            let mut temps: Vec<StatData> = Vec::new();
            let mut fans: Vec<StatData> = Vec::new();

            let mut max_power = 0.0;
            let mut max_temp = 0.0;
            let mut max_fan = 0.0;

            let has_temps = di.dev_stats.temps.back().is_some();
            let has_fans = di.dev_stats.fans.back().is_some();

            if plot_meminfo {
                meminfo.push(StatData::new("SMEM"));
                if di.dev_type.is_discrete() {
                    meminfo.push(StatData::new("VRAM"));
                }
            }
            if plot_engines {
                for en in di.eng_names.iter().sorted() {
                    engs_names.push(en);
                    engines.push(StatData::new(&en.to_uppercase()));
                }
            }
            if plot_freqs {
                for _ in di.freq_limits.iter() {
                    let mut nv = Vec::new();
                    nv.push(StatData::new("MIN"));
                    nv.push(StatData::new("MAX"));
                    nv.push(StatData::new("REQ"));
                    nv.push(StatData::new("ACT"));
                    freqs.push(nv);
                }
            }
            if plot_power {
                power.push(StatData::new("GPU"));
                let pkg_str = if di.dev_type.is_discrete() {
                    "CARD" } else { "PKG" };
                power.push(StatData::new(pkg_str));
            }
            if plot_temps && has_temps {
                let tmps_st = di.dev_stats.temps.back().unwrap();
                for tmp in tmps_st.iter() {
                    temps.push(StatData::new(&tmp.name.to_uppercase()));
                }
            }
            if plot_fans && has_fans {
                let fans_st = di.dev_stats.fans.back().unwrap();
                for fan in fans_st.iter() {
                    fans.push(StatData::new(&fan.name.to_uppercase()));
                }
            }

            for state in self.jsondata.states().iter() {
                let tstamp = *state.timestamps.back().unwrap() as f64 / 1000.0;
                let dinfo = &state.devs_state[idx];

                if plot_meminfo {
                    let mi = dinfo.dev_stats.mem_info.back().unwrap();
                    meminfo[0].add_point((tstamp,
                        mi.smem_used as f64 / (1024.0 * 1024.0)));
                    if dinfo.dev_type.is_discrete() {
                        meminfo[1].add_point((tstamp,
                            mi.vram_used as f64 / (1024.0 * 1024.0)));
                    }
                }
                if plot_engines {
                    for (nr, en) in engs_names.iter().enumerate() {
                        let eu = if dinfo.dev_stats
                            .eng_usage.contains_key(*en) {
                            *dinfo.dev_stats.eng_usage[*en].back().unwrap()
                        } else {
                            0.0
                        };
                        engines[nr].add_point((tstamp, eu));
                    }
                }
                if plot_freqs {
                    let fq = dinfo.dev_stats.freqs.back().unwrap();
                    for nr in 0..dinfo.freq_limits.len() {
                        freqs[nr][0]
                            .add_point((tstamp, fq[nr].min_freq as f64));
                        freqs[nr][1]
                            .add_point((tstamp, fq[nr].max_freq as f64));
                        freqs[nr][2]
                            .add_point((tstamp, fq[nr].cur_freq as f64));
                        freqs[nr][3]
                            .add_point((tstamp, fq[nr].act_freq as f64));
                    }
                }
                if plot_power {
                    let pwr = dinfo.dev_stats.power.back().unwrap();
                    max_power = f64::max(max_power, pwr.gpu_cur_power);
                    max_power = f64::max(max_power, pwr.pkg_cur_power);
                    power[0].add_point((tstamp, pwr.gpu_cur_power));
                    power[1].add_point((tstamp, pwr.pkg_cur_power));
                }
                if plot_temps && has_temps {
                    let tmps_st = dinfo.dev_stats.temps.back().unwrap();
                    for (nr, tmp) in tmps_st.iter().enumerate() {
                        let tv = tmp.temp;
                        max_temp = f64::max(max_temp, tv);
                        temps[nr].add_point((tstamp, tv));
                    }
                }
                if plot_fans && has_fans {
                    let fans_st = dinfo.dev_stats.fans.back().unwrap();
                    for (nr, fan) in fans_st.iter().enumerate() {
                        let sv = fan.speed as f64;
                        max_fan = f64::max(max_fan, sv);
                        fans[nr].add_point((tstamp, sv));
                    }
                }
            }

            let last_state = self.jsondata.states().back().unwrap();
            let x_max = *last_state.timestamps.back().unwrap() as f64 / 1000.0;

            if plot_meminfo {
                let out_file = format!("{}-{}-meminfo.svg",
                    &self.out_prefix, &di.pci_dev);
                let mi = di.dev_stats.mem_info.back().unwrap();
                let y_max = max(mi.smem_total, mi.vram_total) as f64 /
                    (1024.0 * 1024.0);
                self.plot_chart(
                    &out_file, &format!("{} - Memory Info", &di.pci_dev),
                    "Time (s)", "Memory used (MiB)",
                    x_max, y_max, &meminfo)?;
            }
            if plot_engines {
                let out_file = format!("{}-{}-engines.svg",
                    &self.out_prefix, &di.pci_dev);
                self.plot_chart(
                    &out_file, &format!("{} - Engines Usage", &di.pci_dev),
                    "Time (s)", "Usage (%)",
                    x_max, 100.0, &engines)?;
            }
            if plot_freqs {
                for (nr, fl) in di.freq_limits.iter().enumerate() {
                    let out_file = format!("{}-{}-freqs-{}.svg",
                        &self.out_prefix, &di.pci_dev, &fl.name);
                    self.plot_chart(
                        &out_file,
                        &format!("{} - {} Frequencies",
                            &di.pci_dev, &fl.name.to_uppercase()),
                        "Time (s)", "Frequency (MHz)",
                        x_max, fl.maximum as f64, &freqs[nr])?;
                }
            }
            if plot_power {
                let out_file = format!("{}-{}-power.svg",
                    &self.out_prefix, &di.pci_dev);
                self.plot_chart(
                    &out_file, &format!("{} - Power Usage", &di.pci_dev),
                    "Time (s)", "Power (W)",
                    x_max, max_power, &power)?;
            }
            if plot_temps && has_temps {
                let out_file = format!("{}-{}-temps.svg",
                    &self.out_prefix, &di.pci_dev);
                self.plot_chart(
                    &out_file, &format!("{} - Temperatures", &di.pci_dev),
                    "Time (s)", "Temperature (C)",
                    x_max, max_temp, &temps)?;
            }
            if plot_fans && has_fans {
                let out_file = format!("{}-{}-fans.svg",
                    &self.out_prefix, &di.pci_dev);
                self.plot_chart(
                    &out_file, &format!("{} - Fans", &di.pci_dev),
                    "Time (s)", "Speed (RPM)",
                    x_max, max_fan, &fans)?;
            }
        }

        Ok(())
    }

    pub fn from(jsondata: AppDataJson,
        out_prefix: String,
        dev_slots_opt: Option<String>,
        charts_opt: Option<String>) -> Result<Plotter>
    {
        let mut dev_slots = Vec::new();
        if let Some(slots_str) = dev_slots_opt {
            dev_slots = slots_str
                .split(',')
                .map(|d| d.to_string())
                .collect();

            for dev in dev_slots.iter() {
                if !jsondata.states().front().unwrap().devs_state
                    .iter().any(|d| d.pci_dev == *dev) {
                    bail!("No DRM GPU device {:?} in the JSON file", dev);
                }
            }
        }

        let sel_charts = if let Some(charts_str) = charts_opt {
            let mut sc = [false; CHARTS_TOTAL];
            let charts: Vec<_> = charts_str
                .split(',')
                .collect();

            for c in charts.into_iter() {
                match c {
                    "meminfo" => sc[CHART_MEMINFO] = true,
                    "engines" => sc[CHART_ENGINES] = true,
                    "freqs" => sc[CHART_FREQS] = true,
                    "power" => sc[CHART_POWER] = true,
                    "temps" => sc[CHART_TEMPS] = true,
                    "fans" => sc[CHART_FANS] = true,
                    _ => bail!("Invalid chart {:?} requested", c),
                }
            }

            sc
        } else {
            [true; CHARTS_TOTAL]
        };

        Ok(Plotter {
            jsondata,
            out_prefix,
            dev_slots,
            sel_charts,
        })
    }
}
