use futures::{FutureExt, StreamExt};
use std::io::Write;
use std::{io::Stdout, u16};
use tokio::{io::AsyncBufReadExt, sync::mpsc, task::JoinHandle};
use tracing::{Level, event};

use ratatui::{
    DefaultTerminal, Frame, Terminal, TerminalOptions, Viewport,
    crossterm::event::{KeyCode, KeyModifiers},
    layout::{Constraint, Layout, Rect},
    prelude::CrosstermBackend,
    style::{Color, Style, Stylize},
    text::{Line, Span},
    widgets::{Paragraph, Widget},
};

use tui_input::{Input, backend::crossterm::EventHandler};

use color_eyre::Result;

struct Writer<'a>(
    &'a mut Terminal<CrosstermBackend<Stdout>>,
    std::vec::Vec<u8>,
);

impl<'a> Writer<'a> {
    const fn new(terminal: &'a mut Terminal<CrosstermBackend<Stdout>>) -> Self {
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

#[derive(Clone, Debug)]
enum Event {
    Error,
    Tick,
    ChildOutput(String),
    Crossterm(ratatui::crossterm::event::Event),
}

#[derive(Debug)]
pub struct TokioEventHandler {
    _tx: mpsc::UnboundedSender<Event>,
    rx: mpsc::UnboundedReceiver<Event>,
    _task: Option<JoinHandle<()>>,
}

impl TokioEventHandler {
    pub fn new(child_output: Option<std::process::ChildStdout>) -> Self {
        let tick_rate = std::time::Duration::from_millis(250);

        let (tx, rx) = mpsc::unbounded_channel();
        let tx2 = tx.clone();

        let task = tokio::spawn(async move {
            let mut reader = ratatui::crossterm::event::EventStream::new();
            let mut interval = tokio::time::interval(tick_rate);
            let mut child_output_reader = child_output.map(|stdout| {
                tokio::io::BufReader::new(tokio::process::ChildStdout::from_std(stdout).unwrap())
                    .lines()
            });
            loop {
                let delay = interval.tick();
                let crossterm_event = reader.next().fuse();
                if child_output_reader.is_none() {
                    tokio::select! {
                        maybe_event = crossterm_event => {
                            match maybe_event {
                                Some(Ok(evt)) => {
                                    tx.send(Event::Crossterm(evt)).unwrap();
                                }
                                Some(Err(_)) => {
                                    let _ = tx.send(Event::Error);
                                }
                                None => {},
                            }
                        },
                        _ = delay => {
                            let _ = tx.send(Event::Tick);
                        },
                    }
                } else {
                    let child_output_reader_unwrap = child_output_reader.as_mut().unwrap();
                    let child_output = child_output_reader_unwrap.next_line();
                    tokio::select! {
                        maybe_event = crossterm_event => {
                            match maybe_event {
                                Some(Ok(evt)) => {
                                    tx.send(Event::Crossterm(evt)).unwrap();
                                }
                                Some(Err(_)) => {
                                    let _ = tx.send(Event::Error);
                                }
                                None => {},
                            }
                        },
                        _ = delay => {
                            let _ = tx.send(Event::Tick);
                        },
                        maybe_line = child_output => {
                            if let Ok(line) = maybe_line {
                                if let Some(line) = line {
                                    tx.send(Event::ChildOutput(line)).unwrap();
                                } else {
                                    child_output_reader = None;
                                }
                            } else {
                                let _ = tx.send(Event::Error);
                                child_output_reader = None;
                            }
                        },
                    }
                }
            }
        });

        Self {
            _tx: tx2,
            rx,
            _task: Some(task),
        }
    }

    async fn next(&mut self) -> Result<Event> {
        self.rx
            .recv()
            .await
            .ok_or_else(|| color_eyre::eyre::eyre!("Unable to get event"))
    }
}

pub struct Gui {
    debugger: sdblib::Debugger,

    // Past commands
    history: Vec<String>,
    history_current: String,
    index_history: usize,

    // Current input
    input: Input,
    // Child program output
    child_output: Option<std::process::ChildStdout>,
}

impl Gui {
    pub fn new(
        debugger: sdblib::Debugger,
        output_ran_command: Option<std::process::ChildStdout>,
    ) -> Self {
        Self {
            debugger,
            history: Vec::new(),
            history_current: String::new(),
            index_history: 0,
            input: Input::default(),
            child_output: output_ran_command,
        }
    }

    pub async fn run(&mut self) -> Result<()> {
        color_eyre::install()?;
        let mut events = TokioEventHandler::new(self.child_output.take());

        let mut terminal = ratatui::init_with_options(TerminalOptions {
            viewport: Viewport::Inline(1),
        });
        self.run_impl(&mut terminal, &mut events).await
    }

    async fn run_impl(
        &mut self,
        terminal: &mut DefaultTerminal,
        events: &mut TokioEventHandler,
    ) -> Result<()> {
        loop {
            terminal.draw(|frame| self.render(frame))?;

            let event = events.next().await?;
            match event {
                Event::Error => {
                    return Err(color_eyre::eyre::eyre!("Error receiving event"));
                }
                Event::Tick => {
                    // Nothing to do on tick for now
                }
                Event::ChildOutput(str) => {
                    let mut writer = Writer::new(terminal);
                    writeln!(writer, "{str}")?;
                    writer.flush()?;
                }
                Event::Crossterm(crossterm) => {
                    let ratatui::crossterm::event::Event::Key(key) = crossterm else {
                        continue;
                    };
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
                            self.input.handle_event(&crossterm);
                        }
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

        let new_index = self.index_history.saturating_add_signed(direction);

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
            .scroll((0, scroll.try_into().unwrap()));
        frame.render_widget(input, area);

        // Ratatui hides the cursor unless it's explicitly set. Position the  cursor past the
        // end of the input text and one line down from the border to the input line
        let x = self.input.visual_cursor().max(scroll) - scroll;
        frame.set_cursor_position((area.x + u16::try_from(x).unwrap(), area.y));
    }
}
