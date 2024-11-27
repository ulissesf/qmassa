use std::io::{Write, Seek, SeekFrom};
use std::cell::RefCell;
use std::cmp::max;
use std::fs::File;
use std::time;

use anyhow::Result;
use itertools::Itertools;
use log::error;
use serde_json;

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
use ratatui::{
    layout::{Alignment, Constraint, Layout, Rect, Size},
    style::{palette::tailwind, Color, Style, Stylize},
    text::{Span, Line},
    widgets::{Axis, Block, Borders, BorderType, Chart,
        Dataset, Gauge, GraphType, LegendPosition, Row, Table, Tabs},
    symbols, DefaultTerminal, Frame,
};
use tui_scrollview::{ScrollView, ScrollViewState, ScrollbarVisibility};

use crate::app_data::{AppData, AppDataDeviceState, AppDataClientStats};
use crate::Args;


struct DevicesTabState
{
    devs: Vec<String>,
    sel: usize,
}

impl DevicesTabState
{
    fn next(&mut self)
    {
        if self.devs.is_empty() {
            return;
        }

        self.sel = (self.sel + 1) % self.devs.len();
    }

    fn previous(&mut self)
    {
        if self.devs.is_empty() {
            return;
        }

        self.sel = if self.sel == 0 {
            self.devs.len() - 1 } else { self.sel - 1 };
    }

    fn new(devs: Vec<String>) -> DevicesTabState
    {
        DevicesTabState {
            devs,
            sel: 0,
        }
    }
}

const DEVICE_STATS_FREQS: u8 = 0;
const DEVICE_STATS_POWER: u8 = 1;
const DEVICE_STATS_MEMINFO: u8 = 2;
const DEVICE_STATS_ENGINES: u8 = 3;
const DEVICE_STATS_TOTAL: u8 = 4;

const DEVICE_STATS_OP_NEXT: u8 = 0;
const DEVICE_STATS_OP_PREV: u8 = 1;

struct DeviceStatsState
{
    sel: u8,
    last_op: u8,
}

impl DeviceStatsState
{
    fn next(&mut self)
    {
        self.sel = (self.sel + 1) % DEVICE_STATS_TOTAL;
        self.last_op = DEVICE_STATS_OP_NEXT;
    }

    fn previous(&mut self)
    {
        self.sel = if self.sel == 0 {
            DEVICE_STATS_TOTAL - 1 } else { self.sel - 1 };
        self.last_op = DEVICE_STATS_OP_PREV;
    }

    fn repeat_op(&mut self)
    {
        if self.last_op == DEVICE_STATS_OP_NEXT {
            self.next();
        } else {
            self.previous();
        }
    }

    fn new() -> DeviceStatsState
    {
        DeviceStatsState {
            sel: DEVICE_STATS_FREQS,
            last_op: DEVICE_STATS_OP_NEXT,
        }
    }
}

struct ClientsViewState
{
    hdr_state: ScrollViewState,
    stats_state: ScrollViewState,
}

impl ClientsViewState
{
    fn scroll_right(&mut self)
    {
        self.hdr_state.scroll_right();
        self.stats_state.scroll_right();
    }

    fn scroll_left(&mut self)
    {
        self.hdr_state.scroll_left();
        self.stats_state.scroll_left();
    }

    fn scroll_up(&mut self)
    {
        self.stats_state.scroll_up();
    }

    fn scroll_down(&mut self)
    {
        self.stats_state.scroll_down();
    }

    fn scroll_to_top(&mut self)
    {
        self.hdr_state.scroll_to_top();
        self.stats_state.scroll_to_top();
    }

    fn new() -> ClientsViewState
    {
        ClientsViewState {
            hdr_state: ScrollViewState::new(),
            stats_state: ScrollViewState::new(),
        }
    }
}

pub struct App
{
    data: AppData,
    args: Args,
    tab_state: Option<DevicesTabState>,
    stats_state: RefCell<DeviceStatsState>,
    clis_state: RefCell<ClientsViewState>,
    exit: bool,
}

impl App
{
    fn short_mem_string(val: u64) -> String
    {
        let mut nval: u64 = val;
        let mut unit = "";

        if nval >= 1024 * 1024 * 1024 {
            nval /= 1024 * 1024 * 1024;
            unit = "G";
        } else if nval >= 1024 * 1024 {
            nval /= 1024 * 1024;
            unit = "M";
        } else if nval >= 1024 {
            nval /= 1024;
            unit = "K";
        }

        let mut vstr = nval.to_string();
        vstr.push_str(unit);

        vstr
    }

    fn gauge_colored_from(label: Span, ratio: f64) -> Gauge
    {
        let rt = if ratio > 1.0 { 1.0 } else { ratio };
        let gstyle = if rt > 0.7 {
            tailwind::RED.c500
        } else if rt > 0.3 {
            tailwind::ORANGE.c500
        } else {
            tailwind::GREEN.c500
        };

        Gauge::default()
            .label(label)
            .gauge_style(gstyle)
            .use_unicode(true)
            .ratio(rt)
    }

    fn client_pidmem(&self,
        cli: &AppDataClientStats, widths: &Vec<Constraint>) -> Table
    {
        let mem_info = cli.mem_info.last().unwrap();  // always present

        let rows = [Row::new([
                Line::from(cli.pid.to_string())
                    .alignment(Alignment::Center),
                Line::from(App::short_mem_string(mem_info.smem_rss))
                    .alignment(Alignment::Center),
                Line::from(App::short_mem_string(mem_info.vram_rss))
                    .alignment(Alignment::Center),
                Line::from(cli.drm_minor.to_string())
                    .alignment(Alignment::Center),
        ])];

        Table::new(rows, widths)
            .column_spacing(1)
            .style(Style::new().white().on_black())
    }

    fn render_client_engines(&self, cli: &AppDataClientStats,
        constrs: &Vec<Constraint>, clis_sv: &mut ScrollView, area: Rect)
    {
        let mut gauges: Vec<Gauge> = Vec::new();
        for en in cli.eng_stats.keys().sorted() {
            let eng = cli.eng_stats.get(en).unwrap();
            let eut = eng.usage.last().unwrap();  // always present
            let label = Span::styled(
                format!("{:.1}%", eut), Style::new().white());

            gauges.push(App::gauge_colored_from(label, eut/100.0));
        }
        let places = Layout::horizontal(constrs).split(area);

        for (g, a) in gauges.iter().zip(places.iter()) {
            clis_sv.render_widget(g, *a);
        }
    }

    fn client_cpu_usage(&self, cli: &AppDataClientStats) -> Gauge
    {
        let cpu = *cli.cpu_usage.last().unwrap();  // always present
        let label = Span::styled(
            format!("{:.1}%", cpu), Style::new().white());

        App::gauge_colored_from(label, cpu/100.0)
    }

    fn client_cmd(&self, cli: &AppDataClientStats) -> Line
    {
        Line::from(format!("[{}] {}", &cli.comm, &cli.cmdline))
            .alignment(Alignment::Left)
            .style(Style::new().white().on_black())
    }

    fn render_drm_clients(&self,
        dinfo: &AppDataDeviceState, frame: &mut Frame, visible_area: Rect)
    {
        // get all client info and create scrollviews with right size
        let mut cinfos: Vec<&AppDataClientStats> = Vec::new();
        let mut constrs = Vec::new();
        let mut clis_sv_w = visible_area.width;
        let mut clis_sv_h: u16 = 0;

        for cli in dinfo.clis_stats.iter() {
            if self.args.all_clients || cli.is_active {
                cinfos.push(cli);
                constrs.push(Constraint::Length(1));
                clis_sv_w = max(clis_sv_w,
                    (80 + cli.comm.len() + cli.cmdline.len() + 3) as u16);
                clis_sv_h += 1;
           }
        }

        let mut hdr_sv = ScrollView::new(Size::new(clis_sv_w, 1))
            .scrollbars_visibility(ScrollbarVisibility::Never);
        let hdr_sv_area = hdr_sv.area();
        let mut clis_sv = ScrollView::new(Size::new(clis_sv_w, clis_sv_h));
        let clis_sv_area = clis_sv.area();

        let [vis_hdr_area, vis_clis_area] = Layout::vertical(vec![
            Constraint::Length(1),
            Constraint::Fill(1),
        ]).areas(visible_area);

        // render DRM clients headers scrollview
        hdr_sv.render_widget(Block::new()
                .borders(Borders::NONE)
                .style(Style::new().on_dark_gray()),
                hdr_sv_area);
        let line_widths = vec![
            Constraint::Max(22),
            Constraint::Length(1),
            Constraint::Max(42),
            Constraint::Max(7),
            Constraint::Length(1),
            Constraint::Min(5),
        ];
        let [pidmem_hdr, _, engines_hdr, cpu_hdr, _, cmd_hdr] =
            Layout::horizontal(&line_widths).areas(hdr_sv_area);

        let texts = vec![
            Line::from("PID").alignment(Alignment::Center),
            Line::from("SMEM").alignment(Alignment::Center),
            Line::from("VRAM").alignment(Alignment::Center),
            Line::from("MIN").alignment(Alignment::Center),
        ];
        let pidmem_widths = vec![
            Constraint::Max(6),
            Constraint::Max(5),
            Constraint::Max(5),
            Constraint::Max(3),
        ];
        hdr_sv.render_widget(Table::new([Row::new(texts)], &pidmem_widths)
            .column_spacing(1)
            .block(Block::new()
                .borders(Borders::NONE)
                .style(Style::new().white().bold().on_dark_gray())),
            pidmem_hdr);

        let mut texts = Vec::new();
        let mut eng_widths = Vec::new();
        let en_width = if !dinfo.eng_names.is_empty() {
            engines_hdr.width as usize / dinfo.eng_names.len() } else { 0 };
        for en in dinfo.eng_names.iter() {
            texts.push(Line::from(en.to_uppercase())
                .alignment(if en.len() > en_width {
                    Alignment::Left } else { Alignment::Center }));
            eng_widths.push(Constraint::Fill(1));
        }
        hdr_sv.render_widget(Table::new([Row::new(texts)], &eng_widths)
            .column_spacing(1)
            .block(Block::new()
                .borders(Borders::NONE)
                .style(Style::new().white().bold().on_dark_gray())),
            engines_hdr);

        hdr_sv.render_widget(Line::from("CPU")
            .alignment(Alignment::Center)
            .style(Style::new().white().bold().on_dark_gray()),
            cpu_hdr);
        hdr_sv.render_widget(Line::from("COMMAND")
            .alignment(Alignment::Left)
            .style(Style::new().white().bold().on_dark_gray()),
            cmd_hdr);

        // render DRM clients data (if any) scrollview
        clis_sv.render_widget(Block::new()
            .borders(Borders::NONE)
            .style(Style::new().on_black()),
            clis_sv_area);

        if !cinfos.is_empty() {
            let clis_area = Layout::vertical(constrs).split(clis_sv_area);
            for (cli, area) in cinfos.iter().zip(clis_area.iter()) {
                let [pidmem_area, _, engines_area, cpu_area, _, cmd_area] =
                    Layout::horizontal(&line_widths).areas(*area);

                clis_sv.render_widget(
                    self.client_pidmem(cli, &pidmem_widths), pidmem_area);
                self.render_client_engines(
                    cli, &eng_widths, &mut clis_sv, engines_area);
                clis_sv.render_widget(self.client_cpu_usage(cli), cpu_area);
                clis_sv.render_widget(self.client_cmd(cli), cmd_area);
            }
        }

        // render header and clients data to frame's visible area
        let mut st = self.clis_state.borrow_mut();
        frame.render_stateful_widget(
            hdr_sv, vis_hdr_area, &mut st.hdr_state);
        frame.render_stateful_widget(
            clis_sv, vis_clis_area, &mut st.stats_state);
    }

    fn render_meminfo_chart(&self, x_vals: &Vec<f64>, x_axis: Axis,
        dinfo: &AppDataDeviceState, frame: &mut Frame, area: Rect)
    {
        let mut smem_vals = Vec::new();
        let mut vram_vals = Vec::new();

        for (mi, xval) in dinfo.dev_stats.mem_info.iter().zip(x_vals.iter()) {
            smem_vals.push((*xval, mi.smem_used as f64));
            vram_vals.push((*xval, mi.vram_used as f64));
        }
        let datasets = vec![
            Dataset::default()
                .name("SMEM")
                .marker(symbols::Marker::Braille)
                .style(tailwind::BLUE.c700)
                .graph_type(GraphType::Line)
                .data(&smem_vals),
            Dataset::default()
                .name("VRAM")
                .marker(symbols::Marker::Braille)
                .style(tailwind::GREEN.c700)
                .graph_type(GraphType::Line)
                .data(&vram_vals),
        ];

        let lmi = dinfo.dev_stats.mem_info.last().unwrap();  // always present
        let maxy = max(lmi.smem_total, lmi.vram_total);
        let miny = 0;

        let y_bounds = [miny as f64, maxy as f64];
        let y_labels = vec![
            Span::raw(format!("{}", App::short_mem_string(miny))),
            Span::raw(format!("{}", App::short_mem_string((miny + maxy) / 2))),
            Span::raw(format!("{}", App::short_mem_string(maxy))),
        ];
        let y_axis = Axis::default()
            .title("Mem Used")
            .style(Style::new().white())
            .bounds(y_bounds)
            .labels(y_labels);

        frame.render_widget(Chart::new(datasets)
            .x_axis(x_axis)
            .y_axis(y_axis)
            .legend_position(Some(LegendPosition::BottomLeft))
            .hidden_legend_constraints((Constraint::Min(0), Constraint::Min(0)))
            .style(Style::new().bold().on_black()),
            area);
    }

    fn render_engines_chart(&self, x_vals: &Vec<f64>, x_axis: Axis,
        dinfo: &AppDataDeviceState, frame: &mut Frame, area: Rect)
    {
        let mut eng_vals = Vec::new();
        let nr_vals = x_vals.len();

        for en in dinfo.eng_names.iter() {
            let mut nlst = Vec::new();
            let est = dinfo.dev_stats.eng_stats.get(en).unwrap();

            let mut idx = 0;
            if est.usage.len() < nr_vals {
                idx = nr_vals - est.usage.len();
                for i in 0..idx {
                    nlst.push((x_vals[i], 0.0));
                }
            }
            for i in idx..nr_vals {
                nlst.push((x_vals[i], est.usage[i-idx]));
            }

            eng_vals.push(nlst);
        }

        let mut datasets = Vec::new();
        let mut color_idx = 1;

        for (en, ed) in dinfo.eng_names.iter().zip(eng_vals.iter()) {
            datasets.push(Dataset::default()
                .name(en.to_uppercase())
                .marker(symbols::Marker::Braille)
                .style(Color::Indexed(color_idx))
                .graph_type(GraphType::Line)
                .data(ed));
            color_idx += 1;
        }

        let y_bounds = [0.0, 100.0];
        let y_labels = vec![
            Span::raw("0"),
            Span::raw("50"),
            Span::raw("100"),
        ];
        let y_axis = Axis::default()
            .title("Usage (%)")
            .style(Style::new().white())
            .bounds(y_bounds)
            .labels(y_labels);

        frame.render_widget(Chart::new(datasets)
            .x_axis(x_axis)
            .y_axis(y_axis)
            .legend_position(Some(LegendPosition::BottomLeft))
            .hidden_legend_constraints((Constraint::Min(0), Constraint::Min(0)))
            .style(Style::new().bold().on_black()),
            area);
    }

    fn render_power_chart(&self, x_vals: &Vec<f64>, x_axis: Axis,
        dinfo: &AppDataDeviceState, frame: &mut Frame, area: Rect)
    {
        let mut gpu_vals = Vec::new();
        let mut pkg_vals = Vec::new();
        let mut maxy = 0.0;
        let miny = 0.0;

        for (pwr, xval) in dinfo.dev_stats.power.iter().zip(x_vals.iter()) {
            maxy = f64::max(maxy, pwr.gpu_cur_power);
            maxy = f64::max(maxy, pwr.pkg_cur_power);
            gpu_vals.push((*xval, pwr.gpu_cur_power));
            pkg_vals.push((*xval, pwr.pkg_cur_power));
        }
        if maxy == 0.0 {
            maxy = 100.0;
        }

        let datasets = vec![
            Dataset::default()
                .name("PKG")
                .marker(symbols::Marker::Braille)
                .style(tailwind::BLUE.c700)
                .graph_type(GraphType::Line)
                .data(&pkg_vals),
            Dataset::default()
                .name("GPU")
                .marker(symbols::Marker::Braille)
                .style(tailwind::GREEN.c700)
                .graph_type(GraphType::Line)
                .data(&gpu_vals),
        ];

        let y_bounds = [miny, maxy];
        let y_labels = vec![
            Span::raw(format!("{:.1}", miny)),
            Span::raw(format!("{:.1}", (miny + maxy) / 2.0)),
            Span::raw(format!("{:.1}", maxy)),
        ];
        let y_axis = Axis::default()
            .title("Power (W)")
            .style(Style::new().white())
            .bounds(y_bounds)
            .labels(y_labels);

        frame.render_widget(Chart::new(datasets)
            .x_axis(x_axis)
            .y_axis(y_axis)
            .legend_position(Some(LegendPosition::BottomLeft))
            .hidden_legend_constraints((Constraint::Min(0), Constraint::Min(0)))
            .style(Style::new().bold().on_black()),
            area);
    }

    fn render_freqs_chart(&self, x_vals: &Vec<f64>, x_axis: Axis,
        dinfo: &AppDataDeviceState, frame: &mut Frame, area: Rect)
    {
        let mut cur_freq_ds = Vec::new();
        let mut act_freq_ds = Vec::new();
        let mut tr_pl1 = Vec::new();
        let mut tr_status = Vec::new();

        let miny = dinfo.freq_limits.minimum as f64;
        let maxy = dinfo.freq_limits.maximum as f64;

        for (fqs, xval) in dinfo.dev_stats.freqs.iter().zip(x_vals.iter()) {
            cur_freq_ds.push((*xval, fqs.cur_freq as f64));
            act_freq_ds.push((*xval, fqs.act_freq as f64));

            if fqs.throttle_reasons.pl1 {
                tr_pl1.push((*xval, (miny + maxy) / 2.0));
            } else {
                tr_pl1.push((*xval, -1.0));  // hide it
            }
            if fqs.throttle_reasons.status {
                tr_status.push((*xval, (miny + maxy) / 2.0));
            } else {
                tr_status.push((*xval, -1.0));  // hide it
            }
        }
        let fq = dinfo.dev_stats.freqs.last().unwrap();  // always present

        let datasets = vec![
            Dataset::default()
                .name(format!("Requested [{}]", fq.cur_freq))
                .marker(symbols::Marker::Braille)
                .style(tailwind::BLUE.c700)
                .graph_type(GraphType::Line)
                .data(&cur_freq_ds),
            Dataset::default()
                .name(format!("Actual    [{}]", fq.act_freq))
                .marker(symbols::Marker::Braille)
                .style(tailwind::GREEN.c700)
                .graph_type(GraphType::Line)
                .data(&act_freq_ds),
            Dataset::default()
                .name("Throttle: Status")
                .marker(symbols::Marker::Braille)
                .style(tailwind::ORANGE.c700)
                .graph_type(GraphType::Line)
                .data(&tr_status),
            Dataset::default()
                .name("Throttle: PL1")
                .marker(symbols::Marker::Braille)
                .style(tailwind::RED.c700)
                .graph_type(GraphType::Line)
                .data(&tr_pl1),
        ];

        let y_bounds = [miny, maxy];
        let y_labels = vec![
            Span::raw(format!("{}", miny)),
            Span::raw(format!("{}", (miny + maxy) / 2.0)),
            Span::raw(format!("{}", maxy)),
        ];
        let y_axis = Axis::default()
            .title("Freq (MHz)")
            .style(Style::new().white())
            .bounds(y_bounds)
            .labels(y_labels);

        frame.render_widget(Chart::new(datasets)
            .x_axis(x_axis)
            .y_axis(y_axis)
            .legend_position(Some(LegendPosition::BottomLeft))
            .hidden_legend_constraints((Constraint::Min(0), Constraint::Min(0)))
            .style(Style::new().bold().on_black()),
            area);
    }

    fn render_dev_stats(&self, dinfo: &AppDataDeviceState,
        tstamps: &Vec<u128>, frame: &mut Frame, area: Rect)
    {
        let [inf_area, dstats_area, sep, chart_area] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(2),
            Constraint::Length(1),
            Constraint::Min(1),
        ]).areas(area);

        // render some device info and mem/engines/freqs/power stats
        let widths = vec![
            Constraint::Fill(1),
            Constraint::Fill(1),
            Constraint::Fill(2),
        ];
        let rows = [Row::new([
            Line::from(vec![
                "DRIVER: ".white().bold(),
                dinfo.drv_name.clone().into()])
            .alignment(Alignment::Center),
            Line::from(vec![
                "TYPE: ".white().bold(),
                dinfo.dev_type.clone().into()])
            .alignment(Alignment::Center),
            Line::from(vec![
                "DEVICE NODES: ".white().bold(),
                dinfo.dev_nodes.clone().into()])
            .alignment(Alignment::Center),
        ])];
        frame.render_widget(Table::new(rows, widths)
            .style(Style::new().white().on_black())
            .column_spacing(1),
            inf_area);

        let mut ds_st = self.stats_state.borrow_mut();
        if ds_st.sel == DEVICE_STATS_ENGINES && dinfo.eng_names.is_empty() {
            ds_st.repeat_op();
        }

        let [hdr_area, gauges_area] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
        ]).areas(dstats_area);

        let mut dstats_widths = Vec::new();
        dstats_widths.push(Constraint::Length(12));   // SMEM
        dstats_widths.push(Constraint::Length(12));   // VRAM
        for _ in dinfo.eng_names.iter() {
            dstats_widths.push(Constraint::Fill(1));  // ENGINES
        }
        dstats_widths.push(Constraint::Length(10));   // FREQS
        dstats_widths.push(Constraint::Length(12));   // POWER

        // split area for gauges early to calculate max engine name length
        let ind_gs = Layout::horizontal(&dstats_widths).split(gauges_area);
        let en_width = if !dinfo.eng_names.is_empty() {
            ind_gs[2].width as usize } else { 0 };

        let mut hdrs_lst = Vec::new();
        let wh_bold = Style::new().white().bold();
        let ly_bold = Style::new().light_yellow().bold();

        hdrs_lst.push(Line::from("SMEM")
            .alignment(Alignment::Center)
            .style(if ds_st.sel == DEVICE_STATS_MEMINFO {
                ly_bold } else { wh_bold }));
        hdrs_lst.push(Line::from("VRAM")
            .alignment(Alignment::Center)
            .style(if ds_st.sel == DEVICE_STATS_MEMINFO {
                ly_bold } else { wh_bold }));
        for en in dinfo.eng_names.iter() {
            hdrs_lst.push(Line::from(en.to_uppercase())
                .alignment(if en.len() > en_width {
                    Alignment::Left } else { Alignment::Center })
                .style(if ds_st.sel == DEVICE_STATS_ENGINES {
                    ly_bold } else { wh_bold }));
        }
        hdrs_lst.push(Line::from("FREQS")
            .alignment(Alignment::Center)
            .style(if ds_st.sel == DEVICE_STATS_FREQS {
                ly_bold } else { wh_bold }));
        hdrs_lst.push(Line::from("POWER")
            .alignment(Alignment::Center)
            .style(if ds_st.sel == DEVICE_STATS_POWER {
                ly_bold } else { wh_bold }));

        let dstats_hdr = [Row::new(hdrs_lst)];
        frame.render_widget(Table::new(dstats_hdr, &dstats_widths)
            .style(Style::new().on_dark_gray())
            .column_spacing(1),
            hdr_area);

        let mut dstats_gs = Vec::new();

        let mi = dinfo.dev_stats.mem_info.last().unwrap();  // always present
        let smem_label = Span::styled(format!("{}/{}",
            App::short_mem_string(mi.smem_used),
            App::short_mem_string(mi.smem_total)),
            Style::new().white());
        let smem_ratio = if mi.smem_total > 0 {
            mi.smem_used as f64 / mi.smem_total as f64 } else { 0.0 };
        let vram_label = Span::styled(format!("{}/{}",
            App::short_mem_string(mi.vram_used),
            App::short_mem_string(mi.vram_total)),
            Style::new().white());
        let vram_ratio = if mi.vram_total > 0 {
            mi.vram_used as f64 / mi.vram_total as f64 } else { 0.0 };
        dstats_gs.push(App::gauge_colored_from(smem_label, smem_ratio));
        dstats_gs.push(App::gauge_colored_from(vram_label, vram_ratio));

        for en in dinfo.dev_stats.eng_stats.keys().sorted() {
            let eng = dinfo.dev_stats.eng_stats.get(en).unwrap();
            let eut = eng.usage.last().unwrap();  // always present
            let label = Span::styled(
                format!("{:.1}%", eut), Style::new().white());

            dstats_gs.push(App::gauge_colored_from(label, eut/100.0));
        }

        let freqs = dinfo.dev_stats.freqs.last().unwrap();  // always present
        let freqs_label = Span::styled(
            format!("{}/{}", freqs.act_freq, freqs.cur_freq),
            Style::new().white());
        let freqs_ratio = if freqs.cur_freq > 0 {
            freqs.act_freq as f64 / freqs.cur_freq as f64 } else { 0.0 };
        dstats_gs.push(App::gauge_colored_from(freqs_label, freqs_ratio));

        let pwr = dinfo.dev_stats.power.last().unwrap();  // always present
        let pwr_label = Span::styled(
            format!("{:.1}/{:.1}", pwr.gpu_cur_power, pwr.pkg_cur_power),
            Style::new().white());
        let pwr_ratio = if pwr.pkg_cur_power > 0.0 {
            pwr.gpu_cur_power / pwr.pkg_cur_power } else { 0.0 };
        dstats_gs.push(App::gauge_colored_from(pwr_label, pwr_ratio));

        for (eng_g, eng_a) in dstats_gs.iter().zip(ind_gs.iter()) {
            frame.render_widget(eng_g, *eng_a);
        }

        // render separator line
        frame.render_widget(Block::new().borders(Borders::TOP)
            .border_type(BorderType::Plain)
            .border_style(Style::new().white().on_black()),
            sep);

        // render selected chart
        let mut x_vals = Vec::new();
        for ts in tstamps.iter() {
            x_vals.push(*ts as f64 / 1000.0);
        }
        let x_bounds: [f64; 2];
        let mut x_labels: Vec<Span>;
        if x_vals.len() == 1 {
            let int_secs = self.args.ms_interval as f64 / 1000.0;
            x_bounds = [x_vals[0], x_vals[0] + int_secs];
            x_labels = vec![
                Span::raw(format!("{:.1}", x_bounds[0])),
                Span::raw(format!("{:.1}", x_bounds[1])),
            ];
        } else {
            let xvlen = x_vals.len();
            x_bounds = [x_vals[0], x_vals[xvlen - 1]];
            x_labels = vec![
                Span::raw(format!("{:.1}", x_vals[0])),
                Span::raw(format!("{:.1}", x_vals[xvlen / 2])),
            ];
            if x_vals.len() >= 3 {
                x_labels.push(Span::raw(format!("{:.1}", x_vals[xvlen - 1])));
            }
        }
        let x_axis = Axis::default()
            .title("Time (s)")
            .style(Style::new().white())
            .bounds(x_bounds)
            .labels(x_labels);

        match ds_st.sel {
            DEVICE_STATS_FREQS => {
                self.render_freqs_chart(
                    &x_vals, x_axis, dinfo, frame, chart_area);
            },
            DEVICE_STATS_POWER => {
                self.render_power_chart(
                    &x_vals, x_axis, dinfo, frame, chart_area);
            },
            DEVICE_STATS_MEMINFO => {
                self.render_meminfo_chart(
                    &x_vals, x_axis, dinfo, frame, chart_area);
            },
            DEVICE_STATS_ENGINES => {
                self.render_engines_chart(
                    &x_vals, x_axis, dinfo, frame, chart_area);
            },
            _ => {
                error!("Unknown device stats selection: {:?}", ds_st.sel);
            }
        }
    }

    fn render_drm_device(&self,
        dinfo: &AppDataDeviceState, tstamps: &Vec<u128>,
        frame: &mut Frame, area: Rect)
    {
        let [dev_blk_area, clis_blk_area] = Layout::vertical([
            Constraint::Max(21),
            Constraint::Min(8),
        ]).areas(area);

        // render pci device block and stats
        let [dev_title_area, dev_stats_area] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(5),
        ]).areas(dev_blk_area);
        let dev_title = Line::from(vec![
            " ".into(),
            dinfo.vdr_dev_rev.clone().into(),
            " ".into(),
        ]).magenta().bold().on_black();
        let dev_title_len = dinfo.vdr_dev_rev.len() + 2;
        frame.render_widget(Block::new()
            .borders(Borders::TOP)
            .border_type(BorderType::Double)
            .border_style(Style::new().white().bold().on_black())
            .title_top(dev_title.alignment(
                    if dev_title_len > dev_title_area.width as usize {
                        Alignment::Left } else { Alignment::Center })),
            dev_title_area);

        self.render_dev_stats(dinfo, &tstamps, frame, dev_stats_area);

        // render DRM clients block and stats
        let [clis_title_area, clis_stats_area] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(2),
        ]).areas(clis_blk_area);
        let clis_title = Line::from(vec![" DRM clients ".into(),])
            .magenta().bold().on_black();
        frame.render_widget(Block::new()
            .borders(Borders::TOP)
            .border_type(BorderType::Double)
            .border_style(Style::new().white().bold().on_black())
            .title_top(clis_title.alignment(Alignment::Center)),
            clis_title_area);

        // if no DRM clients, nothing more to render
        if dinfo.clis_stats.is_empty() {
            return;
        }

        self.render_drm_clients(dinfo, frame, clis_stats_area);
    }

    fn render_devs_tab(&self,
        devs_ts: &DevicesTabState, frame: &mut Frame, area: Rect)
    {
        frame.render_widget(Tabs::new(devs_ts.devs.clone())
            .style(Style::new().white().bold().on_black())
            .highlight_style(Style::new().magenta().bold().on_black())
            .select(devs_ts.sel),
            area);
    }

    fn draw(&mut self, frame: &mut Frame)
    {
        // if not done yet, initialize tab state with devices
        if self.tab_state.is_none() {
            let mut dv: Vec<String> = Vec::new();

            if let Some(pdev) = &self.args.dev_slot {
                dv.push(pdev.clone());
            } else {
                for di in self.data.devices() {
                    dv.push(di.pci_dev.clone());
                }
            }

            self.tab_state = Some(DevicesTabState::new(dv));
        }

        // render title/menu & status bar, clean main area background
        let [menu_area, main_area, status_bar] = Layout::vertical([
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(1),
        ]).areas(frame.area());

        let prog_name = Line::from(vec![
            " qmassa! v".into(),
            env!("CARGO_PKG_VERSION").into(),
            " ".into(),])
            .style(Style::new().light_blue().bold().on_black());
        let menu_blk = Block::bordered()
            .border_type(BorderType::Thick)
            .border_style(Style::new().cyan().bold().on_black())
            .title_top(prog_name.alignment(Alignment::Center));
        let tab_area = menu_blk.inner(menu_area);
        let instr = Line::from(vec![
            " (Tab) Next device".magenta().bold(),
            " (< >) Change chart".light_yellow().bold(),
            " (↑ ↓ ← →) Scroll clients".white().bold(),
            " (Q) Quit ".white().bold(),])
            .style(Style::new().on_black());

        frame.render_widget(menu_blk, menu_area);
        frame.render_widget(
            Block::new().borders(Borders::NONE)
                .style(Style::new().on_black()),
            main_area);
        frame.render_widget(
            Block::new().borders(Borders::TOP)
                .border_type(BorderType::Thick)
                .border_style(Style::new().cyan().bold().on_black())
                .title_top(instr.alignment(Alignment::Center)),
            status_bar);

        // render selected DRM dev and DRM clients on main area
        let devs_ts = self.tab_state.as_ref().unwrap();

        if devs_ts.devs.is_empty() {
            frame.render_widget(Line::from("No DRM GPU devices")
                .alignment(Alignment::Center), tab_area);
            return;
        }

        let dn = &devs_ts.devs[devs_ts.sel];
        if let Some(dinfo) = self.data.get_device(dn) {
            self.render_devs_tab(devs_ts, frame, tab_area);
            let tstamps = self.data.timestamps();
            self.render_drm_device(dinfo, tstamps, frame, main_area);
        } else {
            frame.render_widget(Line::from(
                    format!("No DRM GPU device at PCI slot: {:?}", dn))
                .alignment(Alignment::Center), tab_area);
        }
    }

    fn handle_key_event(&mut self, key_event: KeyEvent) {
        match key_event.code {
            KeyCode::Char('q') | KeyCode::Char('Q') => {
                self.exit = true;
            },
            KeyCode::Tab => {
                if let Some(devs_ts) = &mut self.tab_state {
                    let mut st = self.clis_state.borrow_mut();
                    devs_ts.next();
                    st.scroll_to_top();
                }
            },
            KeyCode::BackTab => {
                if let Some(devs_ts) = &mut self.tab_state {
                    let mut st = self.clis_state.borrow_mut();
                    devs_ts.previous();
                    st.scroll_to_top();
                }
            },
            KeyCode::Char('>') | KeyCode::Char('.') => {
                let mut st = self.stats_state.borrow_mut();
                st.next();
            },
            KeyCode::Char('<') | KeyCode::Char(',') => {
                let mut st = self.stats_state.borrow_mut();
                st.previous();
            },
            KeyCode::Right => {
                let mut st = self.clis_state.borrow_mut();
                st.scroll_right();
            },
            KeyCode::Left => {
                let mut st = self.clis_state.borrow_mut();
                st.scroll_left();
            },
            KeyCode::Up => {
                let mut st = self.clis_state.borrow_mut();
                st.scroll_up();
            },
            KeyCode::Down => {
                let mut st = self.clis_state.borrow_mut();
                st.scroll_down();
            },
            _ => {}
        }
    }

    fn handle_events(&mut self, ival: time::Duration) -> Result<()>
    {
        if event::poll(ival)? {
            match event::read()? {
                Event::Key(key_event)
                    if key_event.kind == KeyEventKind::Press => {
                        self.handle_key_event(key_event)
                    }
                _ => {}
            };
        }

        Ok(())
    }

    fn do_run(&mut self, terminal: &mut DefaultTerminal) -> Result<()>
    {
        let ival = time::Duration::from_millis(self.args.ms_interval);
        let max_iterations = self.args.nr_iterations;
        let mut nr = 0;

        let mut json_file: Option<File> = None;
        if let Some(fname) = &self.args.to_json {
            let mut f = File::create(fname)?;
            // start json data array
            writeln!(f, "[\n]")?;
            json_file = Some(f);
        }

        while !self.exit {
            if max_iterations >= 0 && nr == max_iterations {
                self.exit = true;
                break;
            }

            self.data.refresh()?;
            if let Some(jf) = &mut json_file {
                // overwrite last 2 bytes == "]\n" with new state
                jf.seek(SeekFrom::End(-2))?;
                if nr >= 1 {
                    writeln!(jf, ",")?;
                }
                serde_json::to_writer_pretty(&mut *jf, self.data.state())?;
                writeln!(jf, "\n]")?;
            }

            terminal.draw(|frame| self.draw(frame))?;
            self.handle_events(ival)?;

            nr += 1;
        }

        Ok(())
    }

    pub fn run(&mut self) -> Result<()>
    {
        let mut terminal = ratatui::init();
        let res = self.do_run(&mut terminal);
        ratatui::restore();

        res
    }

    pub fn from(data: AppData, args: Args) -> App
    {
        App {
            data,
            args,
            tab_state: None,
            stats_state: RefCell::new(DeviceStatsState::new()),
            clis_state: RefCell::new(ClientsViewState::new()),
            exit: false,
        }
    }
}
