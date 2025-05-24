use std::cell::RefCell;
use std::cmp::{max, min};
use std::rc::Rc;

use itertools::Itertools;
use log::error;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    layout::{Alignment, Constraint, Layout, Rect, Size},
    style::{palette::tailwind, Color, Style, Stylize}, symbols,
    text::{Span, Line},
    widgets::{Axis, Block, Borders, BorderType, Chart,
        Dataset, GraphType, LegendPosition, Row, Table},
    Frame,
};
use tui_scrollview::{ScrollView, ScrollViewState, ScrollbarVisibility};

use crate::app_data::{AppData, AppDataClientStats};
use crate::app::{App, Screen, ScreenAction};


#[derive(Debug)]
pub struct DrmClientSelected
{
    pci_dev: String,
    is_dgfx: bool,
    pid: u32,
    client_key: Option<(u32, u32)>,
}

impl DrmClientSelected
{
    pub fn new(pci_dev: String, is_dgfx: bool,
        pid: u32, client_key: Option<(u32, u32)>) -> DrmClientSelected
    {
        DrmClientSelected {
            pci_dev,
            is_dgfx,
            pid,
            client_key,
        }
    }
}

const CLIENT_STATS_MEMINFO: u8 = 0;
const CLIENT_STATS_ENGINES: u8 = 1;
const CLIENT_STATS_CPU: u8 = 2;
const CLIENT_STATS_TOTAL: u8 = 3;

const CLIENT_STATS_OP_NEXT: u8 = 0;
const CLIENT_STATS_OP_PREV: u8 = 1;

#[derive(Debug)]
struct ClientStatsState
{
    sel: u8,
    last_op: u8,
}

impl ClientStatsState
{
    fn next(&mut self)
    {
        self.sel = (self.sel + 1) % CLIENT_STATS_TOTAL;
        self.last_op = CLIENT_STATS_OP_NEXT;
    }

    fn previous(&mut self)
    {
        self.sel = if self.sel == 0 {
            CLIENT_STATS_TOTAL - 1 } else { self.sel - 1 };
        self.last_op = CLIENT_STATS_OP_PREV;
    }

    fn repeat_op(&mut self)
    {
        if self.last_op == CLIENT_STATS_OP_NEXT {
            self.next();
        } else {
            self.previous();
        }
    }

    fn new() -> ClientStatsState
    {
        ClientStatsState {
            sel: CLIENT_STATS_MEMINFO,
            last_op: CLIENT_STATS_OP_NEXT,
        }
    }
}

#[derive(Debug)]
pub struct DrmClientScreen
{
    model: Rc<RefCell<dyn AppData>>,
    sel: DrmClientSelected,
    cmd_sv_state: RefCell<ScrollViewState>,
    stats_state: RefCell<ClientStatsState>,
}

impl Screen for DrmClientScreen
{
    fn name(&self) -> &str
    {
        "DRM Client Screen"
    }

    fn draw(&mut self, frame: &mut Frame, tab_area: Rect, main_area: Rect)
    {
        // render tab area with DRM client basic info
        let widths = if self.sel.client_key.is_some() {
            vec![Constraint::Fill(1); 4]
        } else {
            vec![Constraint::Fill(1); 2]
        };
        let rows = if self.sel.client_key.is_some() {
            let (drm_minor, client_id) =
                self.sel.client_key.as_ref().unwrap();
            vec![Row::new([
                Line::from(vec![
                    "PID: ".white().bold(),
                    format!("{}", self.sel.pid).into()])
                .alignment(Alignment::Center),
                Line::from(vec![
                    "DEV: ".white().bold(),
                    self.sel.pci_dev.clone().into()])
                .alignment(Alignment::Center),
                Line::from(vec![
                    "MINOR: ".white().bold(),
                    format!("{}", drm_minor).into()])
                .alignment(Alignment::Center),
                Line::from(vec![
                    "ID: ".white().bold(),
                    format!("{}", client_id).into()])
                .alignment(Alignment::Center),
            ])]
        } else {
            vec![Row::new([
                Line::from(vec![
                    "PID: ".white().bold(),
                    format!("{}", self.sel.pid).into()])
                .alignment(Alignment::Center),
                Line::from(vec![
                    "DEV: ".white().bold(),
                    self.sel.pci_dev.clone().into()])
                .alignment(Alignment::Center),
            ])]
        };
        frame.render_widget(Table::new(rows, widths)
            .style(Style::new().white().on_black())
            .column_spacing(1),
            tab_area);

        let max_chart_height = min(main_area.width / 4, main_area.height - 4);
        let [cmd_area, table_area, sep, chart_area] = Layout::vertical(vec![
            Constraint::Length(1),
            Constraint::Length(2),
            Constraint::Length(1),
            Constraint::Max(max_chart_height),
        ]).areas(main_area);

        let model = self.model.borrow();
        let di = model.get_device(&self.sel.pci_dev).unwrap();

        let sel_cli = di.find_client_stats(self.sel.pid,
            self.sel.client_key.unwrap());
        if sel_cli.is_none() {
            let line = Line::from(vec![
                ">>>".white().bold().on_red(),
                " This DRM client doesn't exist anymore \
                (process ended or DRM fd closed) ".into(),
                "<<<".white().bold().on_red(),
            ]);
            let lw = line.width();
            frame.render_widget(line.alignment(
                if lw < table_area.width as usize {
                    Alignment::Center } else { Alignment::Left }),
                table_area);
            return;
        }
        let sel_cli = sel_cli.unwrap();

        // render command scrollview
        self.render_command(sel_cli, frame, cmd_area);

        // skip engines selection if no engines are known
        let mut stats_st = self.stats_state.borrow_mut();
        if stats_st.sel == CLIENT_STATS_ENGINES &&
            sel_cli.eng_usage.is_empty() {
            stats_st.repeat_op();
        }
        drop(stats_st);

        // render stats table
        self.render_stats_table(sel_cli, frame, table_area);

        // render separator line
        frame.render_widget(Block::new().borders(Borders::TOP)
            .border_type(BorderType::Plain)
            .border_style(Style::new().white().on_black()),
            sep);

        // render selected chart
        self.render_chart(sel_cli, frame, chart_area);
    }

    fn handle_key_event(
        &mut self, key_event: KeyEvent) -> Option<ScreenAction>
    {
        match key_event.code {
            KeyCode::Right => {
                let mut st = self.cmd_sv_state.borrow_mut();
                st.scroll_right();
            },
            KeyCode::Left => {
                let mut st = self.cmd_sv_state.borrow_mut();
                st.scroll_left();
            },
            KeyCode::Char('>') | KeyCode::Char('.') => {
                let mut st = self.stats_state.borrow_mut();
                st.next();
            },
            KeyCode::Char('<') | KeyCode::Char(',') => {
                let mut st = self.stats_state.borrow_mut();
                st.previous();
            },
            _ => {}
        }

        None
    }

    fn status_bar_text(&mut self) -> Vec<Span>
    {
        vec![
            " (←→) Scroll".magenta().bold(),
            " (< >) Change chart".light_yellow().bold(),
        ]
    }
}

impl DrmClientScreen
{
    fn render_command(&self,
        cli: &AppDataClientStats, frame: &mut Frame, area: Rect)
    {
        let label = "COMMAND: ";
        let [label_area, cmd_area] = Layout::horizontal(vec![
            Constraint::Length(label.len() as u16),
            Constraint::Fill(1),
        ]).areas(area);

        let label_line = Line::from(label)
            .alignment(Alignment::Left)
            .style(Style::new().magenta().bold());
        let cmd_line = Line::from(format!("[{}] {}", &cli.comm, &cli.cmdline))
            .alignment(Alignment::Left)
            .style(Style::new().white());

        let mut state = self.cmd_sv_state.borrow_mut();
        let sv_w = (cli.comm.len() + cli.cmdline.len() + 3) as u16;
        let mut cmd_sv = ScrollView::new(Size::new(sv_w, 1))
            .scrollbars_visibility(ScrollbarVisibility::Never);
        cmd_sv.render_widget(cmd_line, cmd_sv.area());

        frame.render_widget(label_line, label_area);
        frame.render_stateful_widget(cmd_sv, cmd_area, &mut state);
    }

    fn render_stats_table(&self,
        cli: &AppDataClientStats, frame: &mut Frame, area: Rect)
    {
        let stats_st = self.stats_state.borrow();

        let [hdr_area, gauges_area] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
        ]).areas(area);

        let mut widths = Vec::new();
        widths.push(Constraint::Length(12));   // SMEM
        if self.sel.is_dgfx {
            widths.push(Constraint::Length(12));   // VRAM
        }
        for _ in cli.eng_usage.keys() {
            widths.push(Constraint::Fill(1));  // ENGINES
        }
        widths.push(Constraint::Length(7));    // CPU

        let gs_areas = Layout::horizontal(&widths).split(gauges_area);
        let en_width = if !cli.eng_usage.is_empty() {
            gs_areas[if self.sel.is_dgfx { 2 } else { 1 }].width as usize
        } else {
            0
        };

        // render headers
        let mut hdrs_lst = Vec::new();
        let wh_bold = Style::new().white().bold();
        let ly_bold = Style::new().light_yellow().bold();

        hdrs_lst.push(Line::from("SMEM")
            .alignment(Alignment::Center)
            .style(if stats_st.sel == CLIENT_STATS_MEMINFO {
                ly_bold } else { wh_bold }));
        if self.sel.is_dgfx {
            hdrs_lst.push(Line::from("VRAM")
                .alignment(Alignment::Center)
                .style(if stats_st.sel == CLIENT_STATS_MEMINFO {
                    ly_bold } else { wh_bold }));
        }
        for en in cli.eng_usage.keys().sorted() {
            hdrs_lst.push(Line::from(en.to_uppercase())
                .alignment(if en.len() > en_width {
                    Alignment::Left } else { Alignment::Center })
                .style(if stats_st.sel == CLIENT_STATS_ENGINES {
                    ly_bold } else { wh_bold }));
        }
        hdrs_lst.push(Line::from("CPU")
            .alignment(Alignment::Center)
            .style(if stats_st.sel == CLIENT_STATS_CPU {
                ly_bold } else { wh_bold }));

        let stats_hdr = [Row::new(hdrs_lst)];
        frame.render_widget(Table::new(stats_hdr, &widths)
            .style(Style::new().on_dark_gray())
            .column_spacing(1),
            hdr_area);

        // render stats gauges
        let mut stats_gs = Vec::new();

        let mi = cli.mem_info.back().unwrap();
        let smem_label = Span::styled(format!("{}/{}",
            App::short_mem_string(mi.smem_rss),
            App::short_mem_string(mi.smem_used)),
            Style::new().white());
        let smem_ratio = if mi.smem_used > 0 {
            mi.smem_rss as f64 / mi.smem_used as f64 } else { 0.0 };
        stats_gs.push(App::gauge_colored_from(smem_label, smem_ratio));
        if self.sel.is_dgfx {
            let vram_label = Span::styled(format!("{}/{}",
                App::short_mem_string(mi.vram_rss),
                App::short_mem_string(mi.vram_used)),
                Style::new().white());
            let vram_ratio = if mi.vram_used > 0 {
                mi.vram_rss as f64 / mi.vram_used as f64 } else { 0.0 };
            stats_gs.push(App::gauge_colored_from(vram_label, vram_ratio));
        }

        for en in cli.eng_usage.keys().sorted() {
            let eut = cli.eng_usage[en].back().unwrap();
            let label = Span::styled(
                format!("{:.1}%", eut), Style::new().white());

            stats_gs.push(App::gauge_colored_from(label, eut/100.0));
        }

        let cpu = cli.cpu_usage.back().unwrap();
        let cpu_label = Span::styled(
            format!("{:.1}%", cpu), Style::new().white());
        stats_gs.push(App::gauge_colored_from(cpu_label, cpu/100.0));

        for (st_g, st_a) in stats_gs.iter().zip(gs_areas.iter()) {
            frame.render_widget(st_g, *st_a);
        }
    }

    fn render_meminfo_chart(&self, x_vals: &Vec<f64>, x_axis: Axis,
        cli: &AppDataClientStats, frame: &mut Frame, area: Rect)
    {
        let mut sm_rss_vals = Vec::new();
        let mut sm_used_vals = Vec::new();
        let mut vr_rss_vals = Vec::new();
        let mut vr_used_vals = Vec::new();
        let nr_vals = x_vals.len();

        let miny = 0;
        let mut maxy = 1024;

        let mut idx = 0;
        if cli.mem_info.len() < nr_vals {
            idx = nr_vals - cli.mem_info.len();
            for i in 0..idx {
                sm_rss_vals.push((x_vals[i], 0.0));
                sm_used_vals.push((x_vals[i], 0.0));
                if self.sel.is_dgfx {
                    vr_rss_vals.push((x_vals[i], 0.0));
                    vr_used_vals.push((x_vals[i], 0.0));
                }
            }
        }
        for i in idx..nr_vals {
            let mi = &cli.mem_info[i-idx];

            sm_rss_vals.push((x_vals[i], mi.smem_rss as f64));
            sm_used_vals.push((x_vals[i], mi.smem_used as f64));
            maxy = max(maxy, mi.smem_used);

            if self.sel.is_dgfx {
                vr_rss_vals.push((x_vals[i], mi.vram_rss as f64));
                vr_used_vals.push((x_vals[i], mi.vram_used as f64));
                maxy = max(maxy, mi.vram_used);
            }
        }
        let mut datasets = vec![
            Dataset::default()
                .name("SMEM USED")
                .marker(symbols::Marker::Braille)
                .style(tailwind::BLUE.c700)
                .graph_type(GraphType::Line)
                .data(&sm_used_vals),
            Dataset::default()
                .name("SMEM RSS")
                .marker(symbols::Marker::Braille)
                .style(tailwind::GREEN.c700)
                .graph_type(GraphType::Line)
                .data(&sm_rss_vals),
        ];
        if self.sel.is_dgfx {
            datasets.push(Dataset::default()
                .name("VRAM USED")
                .marker(symbols::Marker::Braille)
                .style(tailwind::ORANGE.c700)
                .graph_type(GraphType::Line)
                .data(&vr_used_vals));
            datasets.push(Dataset::default()
                .name("VRAM RSS")
                .marker(symbols::Marker::Braille)
                .style(tailwind::YELLOW.c700)
                .graph_type(GraphType::Line)
                .data(&vr_rss_vals));
        }

        let y_bounds = [miny as f64, maxy as f64];
        let y_labels = vec![
            Span::raw(format!("{}", App::short_mem_string(miny))),
            Span::raw(format!("{}", App::short_mem_string((miny + maxy) / 2))),
            Span::raw(format!("{}", App::short_mem_string(maxy))),
        ];
        let y_axis = Axis::default()
            .title("Mem")
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
        cli: &AppDataClientStats, frame: &mut Frame, area: Rect)
    {
        let mut eng_vals = Vec::new();
        let nr_vals = x_vals.len();

        for en in cli.eng_usage.keys().sorted() {
            let mut nlst = Vec::new();
            let est = &cli.eng_usage[en];

            let mut idx = 0;
            if est.len() < nr_vals {
                idx = nr_vals - est.len();
                for i in 0..idx {
                    nlst.push((x_vals[i], 0.0));
                }
            }
            for i in idx..nr_vals {
                nlst.push((x_vals[i], est[i-idx]));
            }

            eng_vals.push(nlst);
        }

        let mut datasets = Vec::new();
        let mut color_idx = 1;

        for (en, ed) in cli.eng_usage.keys().sorted().zip(eng_vals.iter()) {
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

    fn render_cpu_chart(&self, x_vals: &Vec<f64>, x_axis: Axis,
        cli: &AppDataClientStats, frame: &mut Frame, area: Rect)
    {
        let mut cpu_vals = Vec::new();
        let nr_vals = x_vals.len();
        let mut max_y = 0.0;

        let mut idx = 0;
        if cli.cpu_usage.len() < nr_vals {
            idx = nr_vals - cli.cpu_usage.len();
            for i in 0..idx {
                cpu_vals.push((x_vals[i], 0.0));
            }
        }
        for i in idx..nr_vals {
            let val = cli.cpu_usage[i-idx];
            cpu_vals.push((x_vals[i], val));
            max_y = f64::max(max_y, val);
        }
        let max_y = f64::max(100.0, max_y);
        let datasets = vec![
            Dataset::default()
                .name("CPU")
                .marker(symbols::Marker::Braille)
                .style(tailwind::GREEN.c700)
                .graph_type(GraphType::Line)
                .data(&cpu_vals),
        ];

        let y_bounds = [0.0, max_y];
        let y_labels = vec![
            Span::raw("0"),
            Span::raw(format!("{:.0}", (max_y/2.0).round())),
            Span::raw(format!("{:.0}", max_y.ceil())),
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

    fn render_chart(&self,
        cli: &AppDataClientStats, frame: &mut Frame, area: Rect)
    {
        let model = self.model.borrow();
        let tstamps = model.timestamps();

        let mut x_vals = Vec::new();
        for ts in tstamps.iter() {
            x_vals.push(*ts as f64 / 1000.0);
        }
        let x_bounds: [f64; 2];
        let mut x_labels: Vec<Span>;
        if x_vals.len() == 1 {
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

        let stats_st = self.stats_state.borrow();
        match stats_st.sel {
            CLIENT_STATS_MEMINFO => {
                self.render_meminfo_chart(&x_vals, x_axis, cli, frame, area);
            },
            CLIENT_STATS_ENGINES => {
                self.render_engines_chart(&x_vals, x_axis, cli, frame, area);
            },
            CLIENT_STATS_CPU => {
                self.render_cpu_chart(&x_vals, x_axis, cli, frame, area);
            },
            _ => {
                error!("Unknon client stats selection: {:?}", stats_st.sel);
            }
        }
    }

    pub fn new(model: Rc<RefCell<dyn AppData>>,
        sel: DrmClientSelected) -> Box<dyn Screen>
    {
        Box::new(DrmClientScreen {
            model,
            sel,
            cmd_sv_state: RefCell::new(ScrollViewState::new()),
            stats_state: RefCell::new(ClientStatsState::new()),
        })
    }
}

