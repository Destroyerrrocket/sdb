use std::io::Write;
use std::{
    io::{self, Stdout},
    usize,
};
use tracing::{Level, event};

use ratatui::{
    DefaultTerminal, Frame, Terminal, TerminalOptions, Viewport,
    crossterm::{
        event::{Event, KeyCode, KeyModifiers},
        style::Colors,
    },
    layout::{Constraint, Layout, Rect},
    prelude::CrosstermBackend,
    style::{Color, Style, Stylize},
    text::{Line, Span, ToSpan},
    widgets::{Block, List, Paragraph, Widget},
};
use tui_input::Input;
use tui_input::backend::crossterm::EventHandler;

use color_eyre::{
    Result,
    eyre::{WrapErr, bail},
};

pub struct Gui {
    debugger: sdblib::Debugger,

    // Past commands
    history: Vec<String>,
    history_current: String,
    index_history: usize,

    // Current input
    input: Input,
}

struct Writer<'a>(
    &'a mut Terminal<CrosstermBackend<Stdout>>,
    std::vec::Vec<u8>,
);

impl<'a> Writer<'a> {
    fn new(terminal: &'a mut Terminal<CrosstermBackend<Stdout>>) -> Self {
        Writer(terminal, Vec::new())
    }
}

impl std::io::Write for Writer<'_> {
    #[tracing::instrument(skip(self, buf))]
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.1.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        event!(Level::DEBUG, "Flushing output to terminal");
        for line in String::from_utf8_lossy(&self.1).lines() {
            self.0.insert_before(1, |buffer| {
                buffer.set_string(0, 0, line, Style::default());
            })?;
        }
        Ok(())
    }
}

impl Gui {
    pub fn new(debugger: sdblib::Debugger) -> Self {
        Gui {
            debugger,
            history: Vec::new(),
            history_current: String::new(),
            index_history: 0,
            input: Input::default(),
        }
    }

    pub fn run(&mut self) -> Result<()> {
        color_eyre::install()?;
        let mut terminal = ratatui::init_with_options(TerminalOptions {
            viewport: Viewport::Inline(1),
        });
        let result = self.run_impl(&mut terminal);
        ratatui::restore();
        result
    }

    fn run_impl(&mut self, terminal: &mut DefaultTerminal) -> Result<()> {
        loop {
            terminal.draw(|frame| self.render(frame))?;

            let event = ratatui::crossterm::event::read()?;
            if let Event::Key(key) = event {
                match key.code {
                    KeyCode::Enter => {
                        if !self.run_command(terminal)? {
                            break;
                        }
                    }
                    KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        break;
                    }
                    KeyCode::Up => {
                        self.move_history(-1);
                    }
                    KeyCode::Down => {
                        self.move_history(1);
                    }
                    _ => {
                        self.input.handle_event(&event);
                    }
                }
            }
        }
        Ok(())
    }

    fn move_history(&mut self, direction: isize) {
        if self.history.is_empty() {
            return;
        }

        let new_index = if direction > 0 {
            self.index_history.saturating_add(direction as usize)
        } else {
            self.index_history.saturating_sub((-direction) as usize)
        };

        if new_index > self.history.len() {
            return;
        }

        if self.index_history == new_index {
            return;
        }

        let use_line = if new_index == self.history.len() {
            self.history_current.clone()
        } else {
            if self.index_history == self.history.len() {
                self.history_current = self.input.value().to_string();
            }
            self.history[new_index].clone()
        };

        self.index_history = new_index;
        self.input = Input::default()
            .with_value(use_line)
            .with_cursor(usize::MAX);
    }

    fn run_command(&mut self, terminal: &mut DefaultTerminal) -> Result<bool> {
        let mut command = self.input.value_and_reset();
        if command.is_empty() {
            if let Some(other_command) = self.history.last() {
                command = other_command.clone();
            } else {
                return Ok(true);
            }
        }

        self.history.push(command.clone());
        self.index_history = self.history.len();
        self.history_current.clear();

        terminal.insert_before(1, |buffer| {
            Paragraph::new(Line::from(Span::styled(
                command.clone(),
                Style::default().fg(Color::Green),
            )))
            .render(buffer.area, buffer);
        })?;

        let mut writer = Writer::new(terminal);
        let res = crate::command::run_command(command.as_str(), &mut self.debugger, &mut writer);
        writer.flush()?;
        res
    }

    fn render(&self, frame: &mut Frame) {
        let [prompt_area, input_area] =
            Layout::horizontal([Constraint::Length(5), Constraint::Min(1)]).areas(frame.area());

        frame.render_widget(Paragraph::new("sdb> ").style(Color::Yellow), prompt_area);
        self.render_input(frame, input_area);
    }

    fn render_input(&self, frame: &mut Frame, area: Rect) {
        // keep 2 for borders and 1 for cursor
        let scroll = self.input.visual_scroll(area.width as usize);
        let input = Paragraph::new(self.input.value())
            .style(Style::bold(Color::White.into()))
            .scroll((0, scroll as u16));
        frame.render_widget(input, area);

        // Ratatui hides the cursor unless it's explicitly set. Position the  cursor past the
        // end of the input text and one line down from the border to the input line
        let x = self.input.visual_cursor().max(scroll) - scroll;
        frame.set_cursor_position((area.x + x as u16, area.y));
    }
}
