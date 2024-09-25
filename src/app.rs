use std::collections::HashMap;
use std::time;

use anyhow::Result;
use log::debug;

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
use ratatui::{
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Style, Stylize},
    text::{Line, Text},
    widgets::{block::Title, Block, Borders, BorderType, Row, Table, Gauge},
    DefaultTerminal, Frame
};

use crate::qmdevice::QmDevice;
use crate::qmdrmclients::{QmDrmClients, QmDrmClientInfo};


pub struct App<'a>
{
    qmds: &'a HashMap<u32, QmDevice>,
    clis: &'a mut QmDrmClients,
    ms_ival: u64,
    last_update: time::Instant,
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

    fn client_procmem(&self, cli: &QmDrmClientInfo) -> Table
    {
        let widths = [
            Constraint::Min(6),
            Constraint::Min(11),
            Constraint::Min(6),
            Constraint::Min(6),
        ];

        let rows = [Row::new([
                Text::from(cli.proc.pid.to_string())
                    .alignment(Alignment::Center),
                Text::from(cli.proc.comm.clone())
                    .alignment(Alignment::Center),
                Text::from(App::short_mem_string(cli.total_mem()))
                    .alignment(Alignment::Center),
                Text::from(App::short_mem_string(cli.resident_mem()))
                    .alignment(Alignment::Center),
        ])];

        Table::new(rows, widths)
            .column_spacing(1)
            .style(Style::new().white().on_black())
    }

    fn render_client_engines(&self, cli: &QmDrmClientInfo, frame: &mut Frame, area: Rect)
    {
        let ms_elapsed = self.last_update.elapsed().as_millis() as u64;

        let mut gauges: Vec<Gauge> = Vec::new();
        for eng in cli.engines() {
            gauges.push(Gauge::default()
                .style(Style::new().white().on_black())
                .ratio(cli.eng_utilization(eng, ms_elapsed)/100.0));
        }

        let mut constrs = Vec::new();
        let len = cli.engines().len();
        for _ in 0..len {
            constrs.push(Constraint::Percentage((100/len).try_into().unwrap()));
        }
        let places = Layout::horizontal(constrs).split(area);

        for (g, a) in gauges.iter().zip(places.iter()) {
            frame.render_widget(g, *a);
        }
    }

    fn render_qmd_clients(&self, qmd: &QmDevice, infos: &Vec<&QmDrmClientInfo>, frame: &mut Frame, area: Rect)
    {
        let dev_title = Title::from(Line::from(vec![
            " ".into(),
            qmd.device.clone().into(),
            " (".into(),
            qmd.vendor.clone().into(),
            ", ".into(),
            qmd.devnode.clone().into(),
            ") ".into(),
        ]).magenta().bold().on_black());
        let dev_block = Block::new()
            .borders(Borders::ALL)
            .border_type(BorderType::Double)
            .border_style(Style::new().white().bold().on_black())
            .title(dev_title.alignment(Alignment::Left));
        let stats_area = dev_block.inner(area);
        frame.render_widget(dev_block, area);

        // render DRM clients stats
        let [hdr_area, data_area] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(2),
        ]).areas(stats_area);
        let data_constrs = [
            Constraint::Percentage(45),
            Constraint::Percentage(55),
        ];
        let [procmem_hdr, engines_hdr] = Layout::horizontal(
            data_constrs).areas(hdr_area);

        let texts = vec![
            Text::from("PID").alignment(Alignment::Center),
            Text::from("CMD").alignment(Alignment::Center),
            Text::from("MEM").alignment(Alignment::Center),
            Text::from("RSS").alignment(Alignment::Center),
        ];
        let widths = vec![
            Constraint::Min(6),
            Constraint::Min(11),
            Constraint::Min(6),
            Constraint::Min(6),
        ];
        frame.render_widget(Table::new([Row::new(texts)], widths)
            .column_spacing(1)
            .block(Block::new()
                .borders(Borders::NONE)
                .style(Style::new().white().bold().on_dark_gray())
                ),
            procmem_hdr);

        let engs = infos[0].engines();
        let mut texts = Vec::new();
        let mut widths = Vec::new();
        for e in &engs {
            texts.push(Text::from(e.as_str()).alignment(Alignment::Center));
            widths.push(Constraint::Percentage(
                    (100/engs.len()).try_into().unwrap()));
        }
        frame.render_widget(Table::new([Row::new(texts)], widths)
            .column_spacing(1)
            .block(Block::new()
                .borders(Borders::NONE)
                .style(Style::new().white().bold().on_dark_gray())
                ),
            engines_hdr);

        let mut constrs = Vec::new();
        for _ in 0..infos.len() {
            constrs.push(Constraint::Length(1));
        }
        let clis_area = Layout::vertical(constrs).split(data_area);

        for (cli, area) in infos.iter().zip(clis_area.iter()) {
            let [procmem_area, engines_area] = Layout::horizontal(data_constrs)
                .areas(*area);

            frame.render_widget(self.client_procmem(cli), procmem_area);
            self.render_client_engines(cli, frame, engines_area);
        }
    }

    fn draw(&self, frame: &mut Frame)
    {
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

        let mut all_infos = Vec::new();
        let mut constrs = Vec::new();
        for d in self.clis.devices() {
            let inf = self.clis.device_active_clients(d);
            if !inf.is_empty() {
                all_infos.push((self.qmds.get(d).unwrap(), inf));
                constrs.push(Constraint::Min(1));
            }
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
        let ival = time::Duration::from_millis(self.ms_ival);
        while !self.exit {
            self.clis.refresh()?;
            debug!("{:#?}", self.clis.infos());

            let now = time::Instant::now();
            terminal.draw(|frame| self.draw(frame))?;
            self.last_update = now;

            self.handle_events(ival)?;
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

    pub fn new<'a>(qmdevs: &'a HashMap<u32, QmDevice>, clients: &'a mut QmDrmClients, interval: u64) -> App<'a>
    {
        App {
            qmds: qmdevs,
            clis: clients,
            ms_ival: interval,
            last_update: time::Instant::now(),
            exit: false,
        }
    }
}
