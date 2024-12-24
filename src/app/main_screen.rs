use std::cell::RefCell;
use std::cmp::max;
use std::collections::VecDeque;
use std::rc::Rc;

use itertools::Itertools;
use log::error;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::{Alignment, Constraint, Layout, Rect, Size},
    style::{palette::tailwind, Color, Style, Stylize}, symbols,
    text::{Span, Line},
    widgets::{Axis, Block, Borders, BorderType, Chart,
        Dataset, Gauge, GraphType, LegendPosition, Row, Table, Tabs},
    Frame,
};
use tui_scrollview::{ScrollView, ScrollViewState, ScrollbarVisibility};

use crate::app_data::{AppData, AppDataDeviceState, AppDataClientStats};
use crate::app::{App, Screen, ScreenAction};
use crate::app::drm_client_screen::{DrmClientScreen, DrmClientSelected};


#[derive(Debug)]
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

    fn is_empty(&self) -> bool
    {
        self.devs.is_empty()
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

const DEVICE_STATS_OP_NEXT: i8 = 0;
const DEVICE_STATS_OP_PREV: i8 = 1;

#[derive(Debug)]
struct DeviceStatsState
{
    sel: u8,
    sub_sel: u8,
    req_op: i8,
}

impl DeviceStatsState
{
    fn exec_next(&mut self, nr_charts: &Vec<u8>)
    {
        let nr_cur = nr_charts[self.sel as usize];
        if nr_cur > 1 && (self.sub_sel + 1) < nr_cur {
            self.sub_sel += 1;
        } else {
            self.sub_sel = 0;
            self.sel = (self.sel + 1) % DEVICE_STATS_TOTAL;
            if nr_charts[self.sel as usize] == 0 {  // engines can be empty
                self.sel = (self.sel + 1) % DEVICE_STATS_TOTAL;
            }
        }
    }

    fn exec_prev(&mut self, nr_charts: &Vec<u8>)
    {
        let nr_cur = nr_charts[self.sel as usize];
        if nr_cur > 1 && self.sub_sel > 0 {
            self.sub_sel -= 1;
        } else {
            self.sel = if self.sel == 0 {
                DEVICE_STATS_TOTAL - 1 } else { self.sel - 1 };
            if nr_charts[self.sel as usize] == 0 {  // engines can be empty
                self.sel = if self.sel == 0 {
                    DEVICE_STATS_TOTAL - 1 } else { self.sel - 1 };
            }
            self.sub_sel = nr_charts[self.sel as usize] - 1;
        }
    }

    fn exec_req(&mut self, nr_charts: &Vec<u8>)
    {
        if self.req_op < 0 {
            return;
        }

        if self.req_op == DEVICE_STATS_OP_NEXT {
            self.exec_next(nr_charts);
        } else if self.req_op == DEVICE_STATS_OP_PREV {
            self.exec_prev(nr_charts);
        }
        self.req_op = -1;
    }

    fn req_next(&mut self)
    {
        self.req_op = DEVICE_STATS_OP_NEXT;
    }

    fn req_previous(&mut self)
    {
        self.req_op = DEVICE_STATS_OP_PREV;
    }

    fn new() -> DeviceStatsState
    {
        DeviceStatsState {
            sel: DEVICE_STATS_FREQS,
            sub_sel: 0,
            req_op: -1,
        }
    }
}

#[derive(Debug)]
struct ClientsViewState
{
    sel_row: u16,
    sel_client: Option<DrmClientSelected>,
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
        self.sel_row = self.sel_row.saturating_sub(1);
        self.sel_client = None;
    }

    fn scroll_down(&mut self)
    {
        self.sel_row = self.sel_row.saturating_add(1);
        self.sel_client = None;
    }

    fn scroll_to_top(&mut self)
    {
        self.sel_row = 0;
        self.sel_client = None;
        self.hdr_state.scroll_to_top();
        self.stats_state.scroll_to_top();
    }

    fn new() -> ClientsViewState
    {
        ClientsViewState {
            sel_row: 0,
            sel_client: None,
            hdr_state: ScrollViewState::new(),
            stats_state: ScrollViewState::new(),
        }
    }
}

#[derive(Debug)]
pub struct MainScreen
{
    model: Rc<RefCell<dyn AppData>>,
    tab_state: Option<DevicesTabState>,
    dstats_state: RefCell<DeviceStatsState>,
    clis_state: RefCell<ClientsViewState>,
}

impl Screen for MainScreen
{
    fn name(&self) -> &str
    {
        "Main Screen"
    }

    fn draw(&mut self, frame: &mut Frame, tab_area: Rect, main_area: Rect)
    {
        // if not done yet, initialize tab state with devices
        if self.tab_state.is_none() {
            let model = self.model.borrow();
            let mut dv: Vec<String> = Vec::new();

            if let Some(pdev) = &model.args().dev_slot {
                dv.push(pdev.clone());
            } else {
                for di in model.devices() {
                    dv.push(di.pci_dev.clone());
                }
            }

            self.tab_state = Some(DevicesTabState::new(dv));
        }

        // render selected DRM dev and DRM clients on main area
        let devs_ts = self.tab_state.as_ref().unwrap();

        if devs_ts.is_empty() {
            frame.render_widget(Line::from("No DRM GPU devices")
                .alignment(Alignment::Center), tab_area);
            return;
        }

        let model = self.model.borrow();
        let dn = &devs_ts.devs[devs_ts.sel];
        if let Some(dinfo) = model.get_device(dn) {
            self.render_devs_tab(devs_ts, frame, tab_area);
            let tstamps = model.timestamps();
            self.render_drm_device(dinfo, tstamps, frame, main_area);
        } else {
            frame.render_widget(Line::from(
                    format!("No DRM GPU device at PCI slot: {:?}", dn))
                .alignment(Alignment::Center), tab_area);
        }
    }

    fn handle_key_event(&mut self, key_event: KeyEvent) -> Option<ScreenAction>
    {
        match key_event.code {
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
                let mut st = self.dstats_state.borrow_mut();
                st.req_next();
            },
            KeyCode::Char('<') | KeyCode::Char(',') => {
                let mut st = self.dstats_state.borrow_mut();
                st.req_previous();
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
            KeyCode::Enter => {
                let mut st = self.clis_state.borrow_mut();
                let sel_opt = st.sel_client.take();
                if let Some(sel) = sel_opt {
                    let nscr = DrmClientScreen::new(self.model.clone(), sel);
                    return Some(ScreenAction::Enter(nscr));
                }
            },
            _ => {}
        }

        None
    }

    fn status_bar_text(&mut self) -> Vec<Span>
    {
        vec![
            " (Tab) Next dev".magenta().bold(),
            " (< >) Change chart".light_yellow().bold(),
            " (↑↓←→) Scroll".white().bold(),
            " (Enter) Select".white().bold(),
        ]
    }
}

impl MainScreen
{
    fn client_pidmem(&self, cli: &AppDataClientStats,
        is_dgfx: bool, widths: &Vec<Constraint>) -> Table
    {
        let mem_info = cli.mem_info.back().unwrap();

        let mut lines = vec![
            Line::from(cli.pid.to_string())
                .alignment(Alignment::Center),
            Line::from(App::short_mem_string(mem_info.smem_rss))
                .alignment(Alignment::Center),
        ];
        if is_dgfx {
            lines.push(Line::from(App::short_mem_string(mem_info.vram_rss))
                .alignment(Alignment::Center));
        }
        lines.push(Line::from(cli.drm_minor.to_string())
            .alignment(Alignment::Center));

        let rows = [Row::new(lines),];
        Table::new(rows, widths)
            .column_spacing(1)
            .style(Style::new().white())
    }

    fn render_client_engines(&self, cli: &AppDataClientStats,
        constrs: &Vec<Constraint>, clis_sv: &mut ScrollView, area: Rect)
    {
        let mut gauges: Vec<Gauge> = Vec::new();
        for en in cli.eng_stats.keys().sorted() {
            let eng = cli.eng_stats.get(en).unwrap();
            let eut = eng.usage.back().unwrap();
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
        let cpu = cli.cpu_usage.back().unwrap();
        let label = Span::styled(
            format!("{:.1}%", cpu), Style::new().white());

        App::gauge_colored_from(label, cpu/100.0)
    }

    fn client_cmd(&self, cli: &AppDataClientStats) -> Line
    {
        Line::from(format!("[{}] {}", &cli.comm, &cli.cmdline))
            .alignment(Alignment::Left)
            .style(Style::new().white())
    }

    fn render_drm_clients(&self,
        dinfo: &AppDataDeviceState, frame: &mut Frame, visible_area: Rect)
    {
        let is_dgfx = dinfo.dev_type.is_discrete();

        // get all client info and create scrollviews with right size
        let mut cinfos: Vec<&AppDataClientStats> = Vec::new();
        let mut constrs = Vec::new();
        let mut clis_sv_w = max(90, visible_area.width);
        let mut clis_sv_h: u16 = 0;

        let model = self.model.borrow();
        for cli in dinfo.clis_stats.iter() {
            if cli.is_active || model.args().all_clients {
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

        // adjust selected row/client and data scrollview state
        let mut state = self.clis_state.borrow_mut();
        let y_offset = state.stats_state.offset().y;
        let horiz_bar = (clis_sv_w > vis_clis_area.width) as u16;
        let nr_vis_clis = vis_clis_area.height.saturating_sub(horiz_bar);

        if cinfos.is_empty() {
            state.sel_row = 0;
        } else {
            if state.sel_row >= cinfos.len() as u16 {
                state.sel_row = cinfos.len() as u16 - 1;
            }
            let sel = cinfos[state.sel_row as usize];
            state.sel_client = Some(DrmClientSelected::new(
                dinfo.pci_dev.clone(), is_dgfx,
                sel.pid, sel.drm_minor, sel.client_id));

            if state.sel_row < y_offset {
                state.stats_state.scroll_up();
            }
            if state.sel_row >= y_offset + nr_vis_clis {
                state.stats_state.scroll_down();
            }
        }

        // render DRM clients headers scrollview
        hdr_sv.render_widget(Block::new()
                .borders(Borders::NONE)
                .style(Style::new().on_dark_gray()),
                hdr_sv_area);
        let line_widths = vec![
            Constraint::Max(if is_dgfx { 22 } else { 17 }),
            Constraint::Length(1),
            Constraint::Max(42),
            Constraint::Max(7),
            Constraint::Length(1),
            Constraint::Min(5),
        ];
        let [pidmem_hdr, _, engines_hdr, cpu_hdr, _, cmd_hdr] =
            Layout::horizontal(&line_widths).areas(hdr_sv_area);

        let mut texts = vec![
            Line::from("PID").alignment(Alignment::Center),
            Line::from("SMEM").alignment(Alignment::Center),
        ];
        let mut pidmem_widths = vec![
            Constraint::Length(6),
            Constraint::Length(5),
        ];
        if is_dgfx {
            texts.push(Line::from("VRAM").alignment(Alignment::Center));
            pidmem_widths.push(Constraint::Length(5));
        }
        texts.push(Line::from("MIN").alignment(Alignment::Center));
        pidmem_widths.push(Constraint::Length(3));
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
            let mut row_nr = 0;
            let clis_area = Layout::vertical(constrs).split(clis_sv_area);
            for (cli, area) in cinfos.iter().zip(clis_area.iter()) {
               if row_nr == state.sel_row {
                    clis_sv.render_widget(Block::new()
                        .borders(Borders::NONE)
                        .style(Style::new().on_light_blue()),
                        *area);
                }
                let [pidmem_area, _, engines_area, cpu_area, _, cmd_area] =
                    Layout::horizontal(&line_widths).areas(*area);

                clis_sv.render_widget(
                    self.client_pidmem(cli, is_dgfx, &pidmem_widths),
                    pidmem_area);
                self.render_client_engines(
                    cli, &eng_widths, &mut clis_sv, engines_area);
                clis_sv.render_widget(self.client_cpu_usage(cli), cpu_area);
                clis_sv.render_widget(self.client_cmd(cli), cmd_area);

                row_nr += 1;
            }
        }

        // render header and clients data to frame's visible area
        frame.render_stateful_widget(
            hdr_sv, vis_hdr_area, &mut state.hdr_state);
        frame.render_stateful_widget(
            clis_sv, vis_clis_area, &mut state.stats_state);
    }

    fn render_meminfo_chart(&self, x_vals: &Vec<f64>, x_axis: Axis,
        dinfo: &AppDataDeviceState, frame: &mut Frame, area: Rect)
    {
        let is_dgfx = dinfo.dev_type.is_discrete();

        let mut smem_vals = Vec::new();
        let mut vram_vals = Vec::new();

        for (mi, xval) in dinfo.dev_stats.mem_info.iter().zip(x_vals.iter()) {
            smem_vals.push((*xval, mi.smem_used as f64));
            if is_dgfx {
                vram_vals.push((*xval, mi.vram_used as f64));
            }
        }
        let mut datasets = vec![
            Dataset::default()
                .name("SMEM")
                .marker(symbols::Marker::Braille)
                .style(tailwind::BLUE.c700)
                .graph_type(GraphType::Line)
                .data(&smem_vals),
        ];
        if is_dgfx {
            datasets.push(Dataset::default()
                .name("VRAM")
                .marker(symbols::Marker::Braille)
                .style(tailwind::GREEN.c700)
                .graph_type(GraphType::Line)
                .data(&vram_vals));
        }

        let lmi = dinfo.dev_stats.mem_info.back().unwrap();
        let maxy = if is_dgfx {
            max(lmi.smem_total, lmi.vram_total)
        } else {
            lmi.smem_total
        };
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
        dinfo: &AppDataDeviceState, fq_sel: u8, frame: &mut Frame, area: Rect)
    {
        let fq_nr = fq_sel as usize;
        let mut cur_freq_ds = Vec::new();
        let mut act_freq_ds = Vec::new();
        let mut tr_pl1 = Vec::new();
        let mut tr_status = Vec::new();

        let miny = dinfo.freq_limits[fq_nr].minimum as f64;
        let maxy = dinfo.freq_limits[fq_nr].maximum as f64;

        for (fqs, xval) in dinfo.dev_stats.freqs.iter().zip(x_vals.iter()) {
            cur_freq_ds.push((*xval, fqs[fq_nr].cur_freq as f64));
            act_freq_ds.push((*xval, fqs[fq_nr].act_freq as f64));

            if fqs[fq_nr].throttle_reasons.pl1 {
                tr_pl1.push((*xval, (miny + maxy) / 2.0));
            } else {
                tr_pl1.push((*xval, -1.0));  // hide it
            }
            if fqs[fq_nr].throttle_reasons.status {
                tr_status.push((*xval, (miny + maxy) / 2.0));
            } else {
                tr_status.push((*xval, -1.0));  // hide it
            }
        }

        let fq = &dinfo.dev_stats.freqs.back().unwrap()[fq_nr];
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
        tstamps: &VecDeque<u128>, frame: &mut Frame, area: Rect)
    {
        let is_dgfx = dinfo.dev_type.is_discrete();
        let nr_engines = dinfo.eng_names.len();
        let nr_freqs = dinfo.dev_stats.freqs.back().unwrap().len();

        // nr_stats = smem + vram (if dgfx) + # engines + # freqs + power
        let nr_stats = 1 + is_dgfx as usize + nr_engines + nr_freqs + 1;
        // Can stats fit in just a single table row or not?
        // If not, separate meminfo + engines and freqs + power
        let one_row = nr_stats * 10 <= area.width as usize;

        let [inf_area, dstats_area, sep, chart_area] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(if one_row { 2 } else { 4 }),
            Constraint::Length(1),
            Constraint::Fill(1),
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
                dinfo.dev_type.to_string().into()])
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

        // change selected chart, if needed
        let nr_charts: Vec<u8> = vec![
            nr_freqs as u8,          // FREQS
            1,                       // POWER
            1,                       // MEMINFO
            (nr_engines > 0) as u8,  // ENGINES
        ];
        let mut ds_st = self.dstats_state.borrow_mut();
        ds_st.exec_req(&nr_charts);

        let hdr_area: Rect;
        let mut hdr2_area = Rect::ZERO;
        let gauges_area: Rect;
        let mut gauges2_area = Rect::ZERO;
        if one_row {
            [hdr_area, gauges_area] = Layout::vertical([
                Constraint::Length(1),
                Constraint::Length(1),
            ]).areas(dstats_area);
        } else {
            [hdr_area, gauges_area, hdr2_area, gauges2_area] =
                Layout::vertical([
                    Constraint::Length(1),
                    Constraint::Length(1),
                    Constraint::Length(1),
                    Constraint::Length(1),
                ]).areas(dstats_area);
        }

        let mut dstats_widths: Vec<Constraint> = Vec::new();
        let mut dstats2_widths: Vec<Constraint> = Vec::new();
        dstats_widths.push(Constraint::Length(12));   // SMEM
        if is_dgfx {
            dstats_widths.push(Constraint::Length(12));   // VRAM
        }
        for _ in 0..nr_engines {
            dstats_widths.push(Constraint::Fill(1));  // ENGINES
        }
        let ds_widths_ref: &mut Vec<Constraint> = if one_row {
            &mut dstats_widths } else { &mut dstats2_widths };
        for _ in 0..nr_freqs {
            ds_widths_ref.push(Constraint::Min(10));      // FREQS
        }
        ds_widths_ref.push(Constraint::Min(12));      // POWER

        // split area for gauges early to calculate max engine name length
        let gs_areas = Layout::horizontal(&dstats_widths).split(gauges_area);
        let en_width = if nr_engines > 0 {
            gs_areas[if is_dgfx { 2 } else { 1 }].width as usize } else { 0 };
        let gs2_areas = if one_row {
            Rc::new([])
        } else {
            Layout::horizontal(&dstats2_widths).split(gauges2_area)
        };

        let mut hdrs_lst: Vec<Line> = Vec::new();
        let mut hdrs2_lst: Vec<Line> = Vec::new();
        let wh_bold = Style::new().white().bold();
        let ly_bold = Style::new().light_yellow().bold();

        hdrs_lst.push(Line::from("SMEM")
            .alignment(Alignment::Center)
            .style(if ds_st.sel == DEVICE_STATS_MEMINFO {
                ly_bold } else { wh_bold }));
        if is_dgfx {
            hdrs_lst.push(Line::from("VRAM")
                .alignment(Alignment::Center)
                .style(if ds_st.sel == DEVICE_STATS_MEMINFO {
                    ly_bold } else { wh_bold }));
        }
        for en in dinfo.eng_names.iter() {
            hdrs_lst.push(Line::from(en.to_uppercase())
                .alignment(if en.len() > en_width {
                    Alignment::Left } else { Alignment::Center })
                .style(if ds_st.sel == DEVICE_STATS_ENGINES {
                    ly_bold } else { wh_bold }));
        }
        let hdrs_lst_ref: &mut Vec<Line> = if one_row {
            &mut hdrs_lst } else { &mut hdrs2_lst };
        for fq_nr in 0..nr_freqs {
            let fql = &dinfo.freq_limits[fq_nr];
            let label = if fql.name.is_empty() {
                format!("FREQS")
            } else {
                format!("FRQ:{}", &fql.name.to_uppercase())
            };
            hdrs_lst_ref.push(Line::from(label)
                .alignment(Alignment::Center)
                .style(if ds_st.sel == DEVICE_STATS_FREQS &&
                    ds_st.sub_sel == fq_nr as u8 { ly_bold } else { wh_bold }));
        }
        hdrs_lst_ref.push(Line::from("POWER")
            .alignment(Alignment::Center)
            .style(if ds_st.sel == DEVICE_STATS_POWER {
                ly_bold } else { wh_bold }));

        let dstats_hdr = [Row::new(hdrs_lst)];
        frame.render_widget(Table::new(dstats_hdr, &dstats_widths)
            .style(Style::new().on_dark_gray())
            .column_spacing(1),
            hdr_area);
        if !one_row {
            let dstats2_hdr = [Row::new(hdrs2_lst)];
            frame.render_widget(Table::new(dstats2_hdr, &dstats2_widths)
                .style(Style::new().on_dark_gray())
                .column_spacing(1),
                hdr2_area);
        }

        let mut dstats_gs: Vec<Gauge> = Vec::new();
        let mut dstats2_gs: Vec<Gauge> = Vec::new();

        let mi = dinfo.dev_stats.mem_info.back().unwrap();
        let smem_label = Span::styled(format!("{}/{}",
            App::short_mem_string(mi.smem_used),
            App::short_mem_string(mi.smem_total)),
            Style::new().white());
        let smem_ratio = if mi.smem_total > 0 {
            mi.smem_used as f64 / mi.smem_total as f64 } else { 0.0 };
        dstats_gs.push(App::gauge_colored_from(smem_label, smem_ratio));
        if is_dgfx {
            let vram_label = Span::styled(format!("{}/{}",
                App::short_mem_string(mi.vram_used),
                App::short_mem_string(mi.vram_total)),
                Style::new().white());
            let vram_ratio = if mi.vram_total > 0 {
                mi.vram_used as f64 / mi.vram_total as f64 } else { 0.0 };
            dstats_gs.push(App::gauge_colored_from(vram_label, vram_ratio));
        }

        for en in dinfo.eng_names.iter() {
            let eng = dinfo.dev_stats.eng_stats.get(en).unwrap();
            let eut = eng.usage.back().unwrap();
            let label = Span::styled(
                format!("{:.1}%", eut), Style::new().white());

            dstats_gs.push(App::gauge_colored_from(label, eut/100.0));
        }

        let ds_gs_ref: &mut Vec<Gauge> = if one_row {
            &mut dstats_gs } else { &mut dstats2_gs };

        for fq in dinfo.dev_stats.freqs.back().unwrap().iter() {
            let fq_label = Span::styled(
                format!("{}/{}", fq.act_freq, fq.cur_freq),
                Style::new().white());
            let fq_ratio = if fq.cur_freq > 0 {
                fq.act_freq as f64 / fq.cur_freq as f64 } else { 0.0 };
            ds_gs_ref.push(App::gauge_colored_from(fq_label, fq_ratio));
        }

        let pwr = dinfo.dev_stats.power.back().unwrap();
        let pwr_label = Span::styled(
            format!("{:.1}/{:.1}", pwr.gpu_cur_power, pwr.pkg_cur_power),
            Style::new().white());
        let pwr_ratio = if pwr.pkg_cur_power > 0.0 {
            pwr.gpu_cur_power / pwr.pkg_cur_power } else { 0.0 };
        ds_gs_ref.push(App::gauge_colored_from(pwr_label, pwr_ratio));

        for (ds_g, ds_a) in dstats_gs.iter().zip(gs_areas.iter()) {
            frame.render_widget(ds_g, *ds_a);
        }
        if !one_row {
            for (ds2_g, ds2_a) in dstats2_gs.iter().zip(gs2_areas.iter()) {
                frame.render_widget(ds2_g, *ds2_a);
            }
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
            let model = self.model.borrow();
            let int_secs = model.args().ms_interval as f64 / 1000.0;
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
            if xvlen >= 3 {
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
                    &x_vals, x_axis, dinfo, ds_st.sub_sel, frame, chart_area);
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

    fn render_drm_device(&self, dinfo: &AppDataDeviceState,
        tstamps: &VecDeque<u128>, frame: &mut Frame, area: Rect)
    {
        let [dev_blk_area, clis_blk_area] = Layout::vertical([
            Constraint::Max(24),
            Constraint::Min(5),
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

        self.render_dev_stats(dinfo, tstamps, frame, dev_stats_area);

        // render DRM clients block and stats
        let [clis_title_area, clis_stats_area] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(2),
        ]).areas(clis_blk_area);
        let mut clis_title_str = String::from(" DRM clients ");
        let pid_opt = self.model.borrow().args().pid.clone();
        if let Some(base_pid) = pid_opt {
            if !base_pid.is_empty() {
                clis_title_str.push_str(
                    &format!("(PID tree at {}) ", &base_pid));
            }
        }
        let clis_title = Line::from(vec![clis_title_str.into(),])
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

    pub fn new(model: Rc<RefCell<dyn AppData>>) -> Box<dyn Screen>
    {
        Box::new(MainScreen {
            model,
            tab_state: None,
            dstats_state: RefCell::new(DeviceStatsState::new()),
            clis_state: RefCell::new(ClientsViewState::new()),
        })
    }
}
