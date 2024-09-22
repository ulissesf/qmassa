use std::collections::HashMap;
use std::{thread, time};

use anyhow::Result;
use log::debug;

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
use ratatui::{
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Style, Stylize},
    text::{Line, Text},
    widgets::{
        block::Title, Block, Borders,
        BorderType, Row, Table, Bar, BarChart, BarGroup},
    DefaultTerminal, Frame
};

use crate::qmdevice::QmDevice;
use crate::qmdrmclients::{QmDrmClients, QmDrmClientInfo};


pub struct App<'a>
{
    qmds: &'a HashMap<u32, QmDevice>,
    clis: &'a mut QmDrmClients,
    ms_ival: u64,
    exit: bool,
}

impl App<'_>
{
    pub fn new<'a>(qmdevs: &'a HashMap<u32, QmDevice>, clients: &'a mut QmDrmClients, interval: u64) -> App<'a>
    {
        App {
            qmds: qmdevs,
            clis: clients,
            ms_ival: interval,
            exit: false,
        }
    }

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

        Table::new(rows, widths).column_spacing(1)
    }

    fn client_engines(&self, cli: &QmDrmClientInfo) -> BarChart
    {
        let mut bars: Vec<Bar> = Vec::new();
        for eng in cli.engines() {
            bars.push(Bar::default().value(
                    cli.eng_utilization(eng, self.ms_ival).round() as u64));
        }

        BarChart::default()
            .bar_width(7)
            .bar_gap(2)
            .label_style(Style::new().white())
            .data(BarGroup::default().bars(&bars))
            .max(100)
    }

    fn render_qmd_clients(&self, qmd: &QmDevice, frame: &mut Frame, area: Rect)
    {
        let [dev_bar, stats_area] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(5),
        ]).areas(area);

        let dev_title = Title::from(Line::from(vec![
            "Dev=".into(),
            qmd.devnode.clone().into(),
            ", PCI_ID=".into(),
            qmd.vendor_id.clone().into(),
            ":".into(),
            qmd.device_id.clone().into(),
            ", Sysname=".into(),
            qmd.sysname.clone().into(),
            " ".into(),
        ]));
        frame.render_widget(
            Block::new().borders(Borders::TOP)
                .title(dev_title.alignment(Alignment::Left)),
            dev_bar,
        );

        // render DRM clients stats
        let [hdr_area, data_area] = Layout::vertical([
            Constraint::Length(2),
            Constraint::Min(2),
        ]).areas(stats_area);

        let data_constrs = [
            Constraint::Percentage(45),
            Constraint::Percentage(55),
        ];
        let [procmem_hdr, engines_hdr] = Layout::horizontal(
            data_constrs).areas(hdr_area);

        let widths = vec![
            Constraint::Min(6),
            Constraint::Min(11),
            Constraint::Min(6),
            Constraint::Min(6),
        ];
        let texts = vec![
            Text::from("PID").alignment(Alignment::Center),
            Text::from("CMD").alignment(Alignment::Center),
            Text::from("MEM").alignment(Alignment::Center),
            Text::from("RSS").alignment(Alignment::Center),
        ];
        frame.render_widget(Table::new([Row::new(texts)], widths)
            .column_spacing(1)
            .block(Block::new().borders(Borders::BOTTOM)),
            procmem_hdr);

        let infos = self.clis.device_active_clients(&qmd.minor());
        let engs = infos[0].engines();
        let mut widths = Vec::new();
        let mut texts = Vec::new();
        for e in &engs {
            widths.push(Constraint::Percentage(
                    (100/engs.len()).try_into().unwrap()));
            texts.push(Text::from(e.as_str()).alignment(Alignment::Center));
        }
        frame.render_widget(Table::new([Row::new(texts)], widths)
            .column_spacing(1)
            .block(Block::new().borders(Borders::BOTTOM)),
            engines_hdr);

        let mut constrs = Vec::new();
        for _ in 0..infos.len() {
            constrs.push(Constraint::Max(5));
        }
        let clis_area = Layout::vertical(constrs).split(data_area);

        let sep_constrs = [
            Constraint::Min(4),
            Constraint::Length(1),
        ];
        for (cli, area) in infos.iter().zip(clis_area.iter()) {
            let [cli_area, sep_bar] = Layout::vertical(sep_constrs)
                .areas(*area);
            let [procmem_area, engines_area] = Layout::horizontal(data_constrs)
                .areas(cli_area);

            frame.render_widget(self.client_procmem(cli), procmem_area);
            frame.render_widget(self.client_engines(cli), engines_area);
            frame.render_widget(Block::new().borders(Borders::BOTTOM), sep_bar);
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
            " qmassa! ".blue().bold(),  // TODO: add version
        ]));
        let instr = Title::from(Line::from(vec![
            " Quit: ".into(),
            "<Q> ".red().bold(),
        ]));

        frame.render_widget(
            Block::new().borders(Borders::TOP)
                .border_type(BorderType::Thick)
                .title(prog_name.alignment(Alignment::Center)),
            title_bar,
        );
        frame.render_widget(
            Block::new().borders(Borders::TOP)
                .border_type(BorderType::Thick)
                .title(instr.alignment(Alignment::Right)),
            status_bar,
        );

        let qmdks = self.clis.devices();
        let mut constrs = Vec::new();
         for _ in 0..qmdks.len() {
            constrs.push(Constraint::Min(6));
        }
        let areas = Layout::vertical(constrs).split(main_area);

        for (dkey, area) in qmdks.iter().zip(areas.iter()) {
            self.render_qmd_clients(self.qmds.get(dkey).unwrap(), frame, *area);
        }
    }

    fn handle_key_event(&mut self, key_event: KeyEvent) {
        match key_event.code {
            KeyCode::Char('q') => {
                self.exit = true;
            },
            _ => {}
        }
    }

    fn handle_events(&mut self) -> Result<()>
    {
        let ztime = time::Duration::from_millis(0);

        while event::poll(ztime)? {
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
            terminal.draw(|frame| self.draw(frame))?;
            self.handle_events()?;
            thread::sleep(ival);
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
}
