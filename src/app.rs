use std::io::{Write, Seek, SeekFrom};
use std::cell::RefCell;
use std::cmp::{max, min};
use std::fs::File;
use std::time;

use anyhow::Result;
use serde_json;

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
use ratatui::{
    layout::{Alignment, Constraint, Layout, Rect, Size},
    style::{palette::tailwind, Style, Stylize},
    text::{Span, Line, Text},
    widgets::{block::Title, Axis, Block, Borders, BorderType, Chart,
        Dataset, Gauge, GraphType, LegendPosition, Row, Table, Tabs},
    symbols, DefaultTerminal, Frame,
};
use tui_widgets::scrollview::{ScrollView, ScrollViewState};

use crate::app_data::{AppData, AppDataDeviceState, AppDataClientStats};
use crate::Args;


struct DevicesTabState
{
    devs: Vec<String>,
    sel: usize,
}

impl DevicesTabState
{
    fn new(devs: Vec<String>) -> DevicesTabState
    {
        DevicesTabState {
            devs,
            sel: 0,
        }
    }

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

        if self.sel > 0 {
            self.sel -= 1;
        } else {
            self.sel = self.devs.len() - 1;
        }
    }
}

pub struct App
{
    data: AppData,
    args: Args,
    tab_state: Option<DevicesTabState>,
    clis_state: RefCell<ScrollViewState>,
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
                Text::from(cli.pid.to_string())
                    .alignment(Alignment::Center),
                Text::from(App::short_mem_string(mem_info.smem_rss))
                    .alignment(Alignment::Center),
                Text::from(App::short_mem_string(mem_info.vram_rss))
                    .alignment(Alignment::Center),
                Text::from(cli.drm_minor.to_string())
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
        for eng in cli.eng_stats.iter() {
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

    fn client_cmd(&self, cli: &AppDataClientStats) -> Text
    {
        Text::from(format!("[{}] {}", &cli.comm, &cli.cmdline))
            .alignment(Alignment::Left)
            .style(Style::new().white().on_black())
    }

    fn render_dev_stats(&self,
        dinfo: &AppDataDeviceState, tstamps: &Vec<u128>,
        frame: &mut Frame, area: Rect)
    {
        let [inf_area, memengs_area, sep, freqs_area] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(2),
            Constraint::Length(1),
            Constraint::Min(1),
        ]).areas(area);

        // render some device info and mem/engines stats
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

        let [hdr_area, gauges_area] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
        ]).areas(memengs_area);

        let mut memengs_widths = Vec::new();
        let mut hdrs_lst = Vec::new();

        memengs_widths.push(Constraint::Length(12));
        memengs_widths.push(Constraint::Length(12));
        hdrs_lst.push(Text::from("SMEM").alignment(Alignment::Center));
        hdrs_lst.push(Text::from("VRAM").alignment(Alignment::Center));

        for en in dinfo.eng_names.iter() {
            memengs_widths.push(Constraint::Fill(1));
            hdrs_lst.push(Text::from(en.to_uppercase())
                .alignment(Alignment::Center));
        }

        let memengs_hdr = [Row::new(hdrs_lst)];
        frame.render_widget(Table::new(memengs_hdr, &memengs_widths)
            .style(Style::new().white().bold().on_dark_gray())
            .column_spacing(1),
            hdr_area);

        let ind_gs = Layout::horizontal(&memengs_widths).split(gauges_area);

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

        let mut engs_gs = Vec::new();
        engs_gs.push(App::gauge_colored_from(smem_label, smem_ratio));
        engs_gs.push(App::gauge_colored_from(vram_label, vram_ratio));

        for eng in dinfo.dev_stats.eng_stats.iter() {
            let eut = eng.usage.last().unwrap();  // always present
            let label = Span::styled(
                  format!("{:.1}%", eut), Style::new().white());

            engs_gs.push(App::gauge_colored_from(label, eut/100.0));
        }

        for (eng_g, eng_a) in engs_gs.iter().zip(ind_gs.iter()) {
            frame.render_widget(eng_g, *eng_a);
        }

        // render separator line
        frame.render_widget(Block::new().borders(Borders::TOP)
                .border_type(BorderType::Plain)
                .border_style(Style::new().white().on_black()),
            sep);

        // render dev freqs stats
        let mut x_vals = Vec::new();
        for ts in tstamps.iter() {
            x_vals.push(*ts as f64 / 1000.0);
        }
        let x_bounds: [f64; 2];
        let mut x_axis: Vec<Span>;
        if x_vals.len() == 1 {
            let int_secs = self.args.ms_interval as f64 / 1000.0;
            x_bounds = [x_vals[0], x_vals[0] + int_secs];
            x_axis = vec![
                Span::raw(format!("{:.1}", x_bounds[0])),
                Span::raw(format!("{:.1}", x_bounds[1])),
            ];
        } else {
            let xvlen = x_vals.len();
            x_bounds = [x_vals[0], x_vals[xvlen - 1]];
            x_axis = vec![
                Span::raw(format!("{:.1}", x_vals[0])),
                Span::raw(format!("{:.1}", x_vals[xvlen / 2])),
            ];
            if x_vals.len() >= 3 {
                x_axis.push(Span::raw(format!("{:.1}", x_vals[xvlen - 1])));
            }
        }

        let mut maxy: u64 = 0;
        let mut miny: u64 = u64::MAX;
        let mut act_freq_ds = Vec::new();
        let mut cur_freq_ds = Vec::new();
        let mut tr_pl1 = Vec::new();
        let mut tr_status = Vec::new();

        for (fqs, xval) in dinfo.dev_stats.freqs.iter().zip(x_vals.iter()) {
            maxy = max(maxy, fqs.max_freq);
            miny = min(miny, fqs.min_freq);

            act_freq_ds.push((*xval, fqs.act_freq as f64));
            cur_freq_ds.push((*xval, fqs.cur_freq as f64));

            if fqs.throttle_reasons.pl1 {
                tr_pl1.push((*xval, ((miny + maxy) / 2) as f64));
            } else {
                tr_pl1.push((*xval, 0.0));
            }
            if fqs.throttle_reasons.status {
                tr_status.push((*xval, ((miny + maxy) / 2) as f64));
            } else {
                tr_status.push((*xval, 0.0));
            }
        }
        let miny = miny as f64;
        let maxy = maxy as f64;

        let y_axis = vec![
            Span::raw(format!("{}", miny)),
            Span::raw(format!("{}", (miny + maxy) / 2.0)),
            Span::raw(format!("{}", maxy)),
        ];
        let y_bounds = [miny, maxy];

        let datasets = vec![
            Dataset::default()
                .name("Requested")
                .marker(symbols::Marker::Braille)
                .style(tailwind::BLUE.c700)
                .graph_type(GraphType::Line)
                .data(&cur_freq_ds),
            Dataset::default()
                .name("Actual")
                .marker(symbols::Marker::Braille)
                .style(tailwind::GREEN.c700)
                .graph_type(GraphType::Line)
                .data(&act_freq_ds),
            Dataset::default()
                .name("Throttle: PL1")
                .marker(symbols::Marker::Braille)
                .style(tailwind::RED.c700)
                .graph_type(GraphType::Line)
                .data(&tr_pl1),
            Dataset::default()
                .name("Throttle: Status")
                .marker(symbols::Marker::Braille)
                .style(tailwind::ORANGE.c700)
                .graph_type(GraphType::Line)
                .data(&tr_status),
        ];

        frame.render_widget(Chart::new(datasets)
            .x_axis(Axis::default()
                .title("Time (s)")
                .style(Style::new().white())
                .bounds(x_bounds)
                .labels(x_axis))
            .y_axis(Axis::default()
                .title("Freq (MHz)")
                .style(Style::new().white())
                .bounds(y_bounds)
                .labels(y_axis))
            .legend_position(Some(LegendPosition::BottomLeft))
            .hidden_legend_constraints((Constraint::Min(0), Constraint::Min(0)))
            .style(Style::new().bold().on_black()),
            freqs_area);
    }

    fn render_drm_clients(&self,
        dinfo: &AppDataDeviceState, frame: &mut Frame, visible_area: Rect)
    {
        // get all client info and create scrollview with right size
        let mut cinfos: Vec<&AppDataClientStats> = Vec::new();
        let mut constrs = Vec::new();
        let mut clis_sv_w = visible_area.width;
        let mut clis_sv_h: u16 = 1;

        for cli in dinfo.clis_stats.iter() {
            if self.args.all_clients || cli.is_active {
                cinfos.push(cli);
                constrs.push(Constraint::Length(1));
                clis_sv_w = max(clis_sv_w,
                    (80 + cli.comm.len() + cli.cmdline.len() + 3) as u16);
                clis_sv_h += 1;
           }
        }

        let mut clis_sv = ScrollView::new(Size::new(clis_sv_w, clis_sv_h));
        let clis_sv_area = clis_sv.area();

        clis_sv.render_widget(Block::new()
            .borders(Borders::NONE)
            .style(Style::new().on_black()),
            clis_sv_area);

        // render DRM clients table headers
        let [hdr_area, data_area] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(1),
        ]).areas(clis_sv_area);

        clis_sv.render_widget(Block::new()
                .borders(Borders::NONE)
                .style(Style::new().on_dark_gray()),
                hdr_area);
        let line_widths = vec![
            Constraint::Max(22),
            Constraint::Length(1),
            Constraint::Max(42),
            Constraint::Max(7),
            Constraint::Length(1),
            Constraint::Min(5),
        ];
        let [pidmem_hdr, _, engines_hdr, cpu_hdr, _, cmd_hdr] =
            Layout::horizontal(&line_widths).areas(hdr_area);

        let texts = vec![
            Text::from("PID").alignment(Alignment::Center),
            Text::from("SMEM").alignment(Alignment::Center),
            Text::from("VRAM").alignment(Alignment::Center),
            Text::from("MIN").alignment(Alignment::Center),
        ];
        let pidmem_widths = vec![
            Constraint::Max(6),
            Constraint::Max(5),
            Constraint::Max(5),
            Constraint::Max(3),
        ];
        clis_sv.render_widget(Table::new([Row::new(texts)], &pidmem_widths)
            .column_spacing(1)
            .block(Block::new()
                .borders(Borders::NONE)
                .style(Style::new().white().bold().on_dark_gray())),
            pidmem_hdr);

        let mut texts = Vec::new();
        let mut eng_widths = Vec::new();
        for en in dinfo.eng_names.iter() {
            texts.push(Text::from(en.to_uppercase())
                .alignment(Alignment::Center));
            eng_widths.push(Constraint::Fill(1));
        }
        clis_sv.render_widget(Table::new([Row::new(texts)], &eng_widths)
            .column_spacing(1)
            .block(Block::new()
                .borders(Borders::NONE)
                .style(Style::new().white().bold().on_dark_gray())),
            engines_hdr);

        clis_sv.render_widget(Text::from("CPU")
            .alignment(Alignment::Center)
            .style(Style::new().white().bold().on_dark_gray()),
            cpu_hdr);
        clis_sv.render_widget(Text::from("COMMAND")
            .alignment(Alignment::Left)
            .style(Style::new().white().bold().on_dark_gray()),
            cmd_hdr);

        // render DRM clients data (if any)
        if cinfos.is_empty() {
            frame.render_stateful_widget(
                clis_sv, visible_area, &mut self.clis_state.borrow_mut());
            return;
        }

        let clis_area = Layout::vertical(constrs).split(data_area);
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

        frame.render_stateful_widget(
            clis_sv, visible_area, &mut self.clis_state.borrow_mut());
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
        let dev_title = Title::from(Line::from(vec![
            " ".into(),
            dinfo.vdr_dev_rev.clone().into(),
            " ".into(),
        ]).magenta().bold().on_black());
        frame.render_widget(Block::new()
            .borders(Borders::TOP)
            .border_type(BorderType::Double)
            .border_style(Style::new().white().bold().on_black())
            .title(dev_title.alignment(Alignment::Center)),
            dev_title_area);

        self.render_dev_stats(dinfo, &tstamps, frame, dev_stats_area);

        // render DRM clients block and stats
        let [clis_title_area, clis_stats_area] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(2),
        ]).areas(clis_blk_area);
        let clis_title = Title::from(Line::from(vec![" DRM clients ".into(),])
            .magenta().bold().on_black());
        frame.render_widget(Block::new()
            .borders(Borders::TOP)
            .border_type(BorderType::Double)
            .border_style(Style::new().white().bold().on_black())
            .title(clis_title.alignment(Alignment::Center)),
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

        let prog_name = Title::from(Line::from(vec![
            " qmassa! v".into(),
            env!("CARGO_PKG_VERSION").into(),
            " ".into(),])
            .style(Style::new().light_blue().bold().on_black()));
        let menu_blk = Block::bordered()
                .border_type(BorderType::Thick)
                .border_style(Style::new().cyan().bold().on_black())
                .title(prog_name.alignment(Alignment::Center));
        let tab_area = menu_blk.inner(menu_area);
        let instr = Title::from(Line::from(vec![
            " (Tab/BackTab) Next/prev device (↑/↓/←/→) Scroll clients (Q) Quit ".into(),])
            .style(Style::new().white().bold().on_black()));

        frame.render_widget(menu_blk, menu_area);
        frame.render_widget(
            Block::new().borders(Borders::NONE)
                .style(Style::new().on_black()),
            main_area);
        frame.render_widget(
            Block::new().borders(Borders::TOP)
                .border_type(BorderType::Thick)
                .border_style(Style::new().cyan().bold().on_black())
                .title(instr.alignment(Alignment::Center)),
            status_bar);

        // render selected DRM dev and DRM clients on main area
        let devs_ts = self.tab_state.as_ref().unwrap();

        if devs_ts.devs.is_empty() {
            frame.render_widget(Text::from("No DRM GPU devices")
                .alignment(Alignment::Center), tab_area);
            return;
        }

        let dn = &devs_ts.devs[devs_ts.sel];
        if let Some(dinfo) = self.data.get_device(dn) {
            self.render_devs_tab(devs_ts, frame, tab_area);
            let tstamps = self.data.timestamps();
            self.render_drm_device(dinfo, tstamps, frame, main_area);
        } else {
            frame.render_widget(Text::from(
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
                    devs_ts.next();
                }
            },
            KeyCode::BackTab => {
                if let Some(devs_ts) = &mut self.tab_state {
                    devs_ts.previous();
                }
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
            clis_state: RefCell::new(ScrollViewState::new()),
            exit: false,
        }
    }
}
