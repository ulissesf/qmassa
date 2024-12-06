use core::fmt::Debug;
use std::cell::RefCell;
use std::io::{Write, Seek, SeekFrom};
use std::fs::File;
use std::rc::Rc;
use std::time;

use anyhow::Result;
use serde_json;

use crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind};
use ratatui::{
    layout::{Alignment, Constraint, Layout, Rect},
    style::{palette::tailwind, Style, Stylize},
    text::{Span, Line},
    widgets::{Block, Borders, BorderType, Gauge},
    DefaultTerminal, Frame,
};

use crate::app_data::AppData;
use crate::Args;

mod main_screen;
mod drm_client_screen;
use main_screen::MainScreen;


#[derive(Debug)]
pub struct AppModel
{
    pub data: AppData,
    pub args: Args,
}

#[derive(Debug)]
#[allow(dead_code)]
pub enum ScreenAction
{
    Exit,
    Enter(Box<dyn Screen>),
}

pub trait Screen
{
    fn name(&self) -> &str;

    fn draw(&mut self, frame: &mut Frame, tab_area: Rect, main_area: Rect);

    fn handle_key_event(&mut self, key_event: KeyEvent) -> Option<ScreenAction>;

    fn status_bar_text(&mut self) -> Vec<Span>;
}

impl Debug for dyn Screen
{
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "Screen({:?})", self.name())
    }
}

#[derive(Debug)]
pub struct AppScreens
{
    stack: Vec<Box<dyn Screen>>,
}

impl AppScreens
{
    pub fn enter(&mut self, scr: Box<dyn Screen>)
    {
        self.stack.push(scr);
    }

    pub fn exit(&mut self)
    {
        self.stack.pop();
    }

    pub fn current(&mut self) -> Option<&mut Box<dyn Screen>>
    {
        self.stack.last_mut()
    }

    pub fn len(&mut self) -> usize
    {
        self.stack.len()
    }

    fn new() -> AppScreens
    {
        AppScreens {
            stack: Vec::new(),
        }
    }
}

#[derive(Debug)]
pub struct App
{
    model: Rc<RefCell<AppModel>>,
    screens: AppScreens,
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

    fn draw(&mut self, frame: &mut Frame)
    {
        // render title/menu & status bar, clean main area background
        let [menu_area, main_area, status_bar] = Layout::vertical([
            Constraint::Length(3),
            Constraint::Fill(1),
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

        let st_len = self.screens.len();
        let scr = self.screens.current().unwrap();  // always >= 1 screens

        let mut st_bar_text = scr.status_bar_text();
        if st_len > 1 {
            st_bar_text.push(" (Esc) Back".white().bold());
        }
        st_bar_text.push(" (Q) Quit ".white().bold());

        let instr = Line::from(st_bar_text).style(Style::new().on_black());

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

        // render current screen content into tab and main areas
        scr.draw(frame, tab_area, main_area);
    }

    fn handle_key_event(&mut self, key_event: KeyEvent) {
        match key_event.code {
            KeyCode::Char('q') | KeyCode::Char('Q') => {
                self.exit = true;
            },
            KeyCode::Esc => {
                self.screens.exit();
                if self.screens.current().is_none() {
                    self.exit = true;
                }
            },
            _ => {
                if let Some(scr) = self.screens.current() {
                    if let Some(act) = scr.handle_key_event(key_event) {
                        if let ScreenAction::Enter(nscr) = act {
                            self.screens.enter(nscr);
                        } else {
                            self.screens.exit();
                        }
                    }
                }
            }
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
        let model = self.model.borrow();
        let ival = time::Duration::from_millis(model.args.ms_interval);
        let max_iterations = model.args.nr_iterations;

        let mut json_file: Option<File> = None;
        if let Some(fname) = &model.args.to_json {
            let mut f = File::create(fname)?;
            // start json data array
            writeln!(f, "[\n]")?;
            json_file = Some(f);
        }
        drop(model);

        let mut nr = 0;
        while !self.exit {
            if max_iterations >= 0 && nr == max_iterations {
                self.exit = true;
                break;
            }

            let mut model = self.model.borrow_mut();
            model.data.refresh()?;
            if let Some(jf) = &mut json_file {
                // overwrite last 2 bytes == "]\n" with new state
                jf.seek(SeekFrom::End(-2))?;
                if nr >= 1 {
                    writeln!(jf, ",")?;
                }
                serde_json::to_writer_pretty(&mut *jf, model.data.state())?;
                writeln!(jf, "\n]")?;
            }
            drop(model);

            terminal.draw(|frame| self.draw(frame))?;
            self.handle_events(ival)?;

            nr += 1;
        }

        Ok(())
    }

    pub fn run(&mut self) -> Result<()>
    {
        let main_scr = MainScreen::new(self.model.clone());
        self.screens.enter(main_scr);

        let mut terminal = ratatui::init();
        let res = self.do_run(&mut terminal);
        ratatui::restore();

        res
    }

    pub fn from(data: AppData, args: Args) -> App
    {
        App {
            model: Rc::new(RefCell::new(AppModel {
                data,
                args,
            })),
            screens: AppScreens::new(),
            exit: false,
        }
    }
}
