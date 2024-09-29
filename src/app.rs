use std::io::Write;
use std::fs::File;
use std::time;

use anyhow::Result;

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
use ratatui::{
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Style, Stylize},
    text::{Line, Text},
    widgets::{block::Title, Block, Borders, BorderType, Row, Table, Gauge},
    DefaultTerminal, Frame
};

use crate::qmdrmdevices::{QmDrmDevices, QmDrmDeviceInfo};
use crate::qmdrmclients::{QmDrmClients, QmDrmClientInfo};
use crate::Args;


pub struct App<'a>
{
    qmds: &'a QmDrmDevices,
    clis: &'a mut QmDrmClients,
    args: &'a Args,
    exit: bool,
}

impl App<'_>
{
    fn short_mem_string(val: u64) -> String
    {
        let mut nval: u64 = val;
        let mut unit = "";

        if nval > 1024 * 1024 * 1024 {
            nval /= 1024 * 1024 * 1024;
            unit = "G";
        } else if nval > 1024 * 1024 {
            nval /= 1024 * 1024;
            unit = "M";
        } else if nval > 1024 {
            nval /= 1024;
            unit = "K";
        }

        let mut vstr = nval.to_string();
        vstr.push_str(unit);

        vstr
    }

    fn client_pidmem(&self, cli: &QmDrmClientInfo, widths: &Vec<Constraint>) -> Table
    {
        let rows = [Row::new([
                Text::from(cli.proc.pid.to_string())
                    .alignment(Alignment::Center),
                Text::from(App::short_mem_string(cli.total_mem()))
                    .alignment(Alignment::Center),
                Text::from(App::short_mem_string(cli.resident_mem()))
                    .alignment(Alignment::Center),
                Text::from(cli.drm_minor.to_string())
                    .alignment(Alignment::Center),
        ])];

        Table::new(rows, widths)
            .column_spacing(1)
            .style(Style::new().white().on_black())
    }

    fn render_client_engines(&self, cli: &QmDrmClientInfo,
        constrs: &Vec<Constraint>, frame: &mut Frame, area: Rect)
    {
        let mut gauges: Vec<Gauge> = Vec::new();
        for eng in cli.engines() {
            gauges.push(Gauge::default()
                .style(Style::new().white().on_black())
                .ratio(cli.eng_utilization(eng)/100.0));
        }
        let places = Layout::horizontal(constrs).split(area);

        for (g, a) in gauges.iter().zip(places.iter()) {
            frame.render_widget(g, *a);
        }
    }

    fn client_proc(&self, cli: &QmDrmClientInfo) -> Text
    {
        Text::from(cli.proc.cmdline.clone())
            .alignment(Alignment::Left)
            .style(Style::new().white().on_black())
    }

    fn render_qmd_clients(&self, qmd: &QmDrmDeviceInfo,
        infos: &Vec<&QmDrmClientInfo>, frame: &mut Frame, area: Rect)
    {
        // render pci device block and title
        let dev_title = Title::from(Line::from(vec![
            " ".into(),
            qmd.vendor.clone().into(),
            " ".into(),
            qmd.device.clone().into(),
            " (".into(),
            qmd.pci_dev.clone().into(),
            ") ".into(),
        ]).magenta().bold().on_black());
        let dev_block = Block::new()
            .borders(Borders::ALL)
            .border_type(BorderType::Double)
            .border_style(Style::new().white().bold().on_black())
            .title(dev_title.alignment(Alignment::Left));
        let stats_area = dev_block.inner(area);
        frame.render_widget(dev_block, area);

        // if no DRM clients, nothing more to render
        if infos.is_empty() {
            return;
        }

        // render table headers
        let [hdr_area, data_area] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(1),
        ]).areas(stats_area);
        let line_widths = vec![
            Constraint::Max(22),
            Constraint::Length(1),
            Constraint::Max(42),
            Constraint::Length(1),
            Constraint::Min(4),
        ];
        frame.render_widget(
            Block::new().borders(Borders::NONE)
                .style(Style::new().on_dark_gray()),
            hdr_area,
        );
        let [pidmem_hdr, _, engines_hdr, _, cmd_hdr] = Layout::horizontal(
            &line_widths).areas(hdr_area);

        let texts = vec![
            Text::from("PID").alignment(Alignment::Center),
            Text::from("MEM").alignment(Alignment::Center),
            Text::from("RSS").alignment(Alignment::Center),
            Text::from("MIN").alignment(Alignment::Center),
        ];
        let pidmem_widths = vec![
            Constraint::Max(6),
            Constraint::Max(5),
            Constraint::Max(5),
            Constraint::Max(3),
        ];
        frame.render_widget(Table::new([Row::new(texts)], &pidmem_widths)
            .column_spacing(1)
            .block(Block::new()
                .borders(Borders::NONE)
                .style(Style::new().white().bold().on_dark_gray())),
            pidmem_hdr);

        let engs = infos[0].engines();
        let mut texts = Vec::new();
        let mut eng_widths = Vec::new();
        for e in &engs {
            texts.push(Text::from(e.to_uppercase())
                .alignment(Alignment::Center));
            eng_widths.push(Constraint::Percentage(
                    (100/engs.len()).try_into().unwrap()));
        }
        frame.render_widget(Table::new([Row::new(texts)], &eng_widths)
            .column_spacing(1)
            .block(Block::new()
                .borders(Borders::NONE)
                .style(Style::new().white().bold().on_dark_gray())),
            engines_hdr);

        frame.render_widget(Text::from("CMD")
            .alignment(Alignment::Center)
            .style(Style::new().white().bold().on_dark_gray()),
            cmd_hdr);

        // render DRM clients data
        let mut constrs = Vec::new();
        for _ in 0..infos.len() {
            constrs.push(Constraint::Length(1));
        }
        let clis_area = Layout::vertical(constrs).split(data_area);

        for (cli, area) in infos.iter().zip(clis_area.iter()) {
            let [pidmem_area, _, engines_area, _, cmd_area] =
                Layout::horizontal(&line_widths).areas(*area);

            frame.render_widget(
                self.client_pidmem(cli, &pidmem_widths), pidmem_area);
            self.render_client_engines(cli, &eng_widths, frame, engines_area);
            frame.render_widget(self.client_proc(cli), cmd_area);
        }
    }

    fn draw(&self, frame: &mut Frame)
    {
        // render title & status bar, and clean main area background
        let [title_bar, main_area, status_bar] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(0),
            Constraint::Length(1),
        ]).areas(frame.area());

        let prog_name = Title::from(Line::from(vec![
            " qmassa! v".into(),
            env!("CARGO_PKG_VERSION").into(),
            " ".into(),])
            .style(Style::new().light_blue().bold().on_black()));
        let instr = Title::from(Line::from(vec![
            " (Q) Quit ".into(),])
            .style(Style::new().white().bold().on_black()));

        frame.render_widget(
            Block::new().borders(Borders::TOP)
                .border_type(BorderType::Thick)
                .border_style(Style::new().cyan().bold().on_black())
                .title(prog_name.alignment(Alignment::Center)),
            title_bar,
        );
        frame.render_widget(
            Block::new().borders(Borders::NONE)
                .style(Style::new().on_black()),
            main_area,
        );
        frame.render_widget(
            Block::new().borders(Borders::TOP)
                .border_type(BorderType::Thick)
                .border_style(Style::new().cyan().bold().on_black())
                .title(instr.alignment(Alignment::Right)),
            status_bar,
        );

        // divide main area and render each pci dev and its DRM clients
        let mut all_infos = Vec::new();
        let mut constrs = Vec::new();

        let devs = if self.args.every_device {
            self.qmds.devices()
        } else {
            self.clis.active_devices()
        };

        for d in devs {
            let inf = if self.args.all_clients {
                self.clis.device_clients(d)
            } else {
                self.clis.device_active_clients(d)
            };

            all_infos.push((self.qmds.device_info(d).unwrap(), inf));
            constrs.push(Constraint::Min(1));
        }
        if all_infos.is_empty() {
            return;
        }

        let areas = Layout::vertical(constrs).split(main_area);
        for (dev_info, area) in all_infos.iter().zip(areas.iter()) {
            let (qmd, infos) = dev_info;
            self.render_qmd_clients(qmd, infos, frame, *area);
        }
    }

    fn handle_key_event(&mut self, key_event: KeyEvent) {
        match key_event.code {
            KeyCode::Char('q') | KeyCode::Char('Q') => {
                self.exit = true;
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

        let start_time = time::Instant::now();
        let mut txt_file: Option<File> = None;

        if let Some(txt_fname) = &self.args.to_txt {
            let mut f = File::create(txt_fname)?;
            writeln!(f, "{:#?}", self.qmds)?;
            txt_file = Some(f);
        }

        while !self.exit {
            if max_iterations >= 0 && nr == max_iterations {
                self.exit = true;
                break;
            }

            self.clis.refresh()?;
            if let Some(tf) = &mut txt_file {
                writeln!(tf, ">>> Iteration: {:?}", nr+1)?;
                writeln!(tf, ">>> Timestamp: {:?}",
                    start_time.elapsed().as_millis())?;
                writeln!(tf, "{:#?}", self.clis)?;
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

    pub fn new<'a>(qmdevs: &'a QmDrmDevices,
        clients: &'a mut QmDrmClients, args: &'a Args) -> App<'a>
    {
        App {
            qmds: qmdevs,
            clis: clients,
            args: args,
            exit: false,
        }
    }
}
