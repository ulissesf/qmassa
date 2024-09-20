use std::collections::HashMap;
use std::{thread, time};

use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
use ratatui::{
    layout::{Alignment, Constraint, Layout, Rect},
    style::{Color, Modifier, Style, Stylize},
    text::{Line, Span, Text},
    widgets::{block::Title, Block, Borders, BorderType, Paragraph},
    DefaultTerminal, Frame
};

use crate::qmdevice::QmDevice;
use crate::qmdrmclients::QmDrmClients;


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

    fn render_qmd_clients(&self, qmd: &QmDevice, frame: &mut Frame, area: Rect)
    {
        let [dev_bar, stats_area] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(5),
        ]).areas(area);

        let dev_title = Title::from(Line::from(vec![
            "Card=".into(),
            qmd.devnode.clone().into(),
            ", Minor=".into(),
            qmd.devnum.1.to_string().into(),
            ", Sysname=".into(),
            qmd.sysname.clone().into(),
            ", PCI_ID=".into(),
            qmd.vendor_id.clone().into(),
            ":".into(),
            qmd.device_id.clone().into(),
            " ".into(),
        ]));
        frame.render_widget(
            Block::new().borders(Borders::TOP)
                .title(dev_title.alignment(Alignment::Left)),
            dev_bar,
        );

        // render DRM clients stats
        // TODO
    }

    fn draw(&self, frame: &mut Frame)
    {
        let [title_bar, main_area, status_bar] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(6),
            Constraint::Length(1),
        ]).areas(frame.area());

        let prog_name = Title::from(Line::from(vec![
            "[ qmassa! ]".blue().bold(),  // TODO: add version
        ]));
        let instr = Title::from(Line::from(vec![
            "[ Quit: ".into(),
            "<Q> ".red().bold(),
            "]".into(),
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

        let mut qmdks:Vec<&u32> = self.clis.infos.keys().collect::<Vec<&_>>();
        qmdks.sort();

        let mut constraints = Vec::new();
        for _ in 0..qmdks.len() {
            constraints.push(Constraint::Min(6)); // TODO: check real min here
        }
        let areas = Layout::vertical(constraints).split(main_area);

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
        match event::read()? {
            Event::Key(key_event) if key_event.kind == KeyEventKind::Press => {
                self.handle_key_event(key_event)
            }
            _ => {}
        };

        Ok(())
    }

    fn do_run(&mut self, terminal: &mut DefaultTerminal) -> Result<()>
    {
        let ival = time::Duration::from_millis(self.ms_ival);

        while !self.exit {
            self.clis.refresh()?;
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
