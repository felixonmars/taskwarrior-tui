use color_eyre::eyre::Result;
use crossterm::event::KeyEvent;
use serde_derive::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::{
  command::Command,
  components::{task_report::TaskReport, Component},
  config::Config,
  tui,
};

#[derive(Default, Debug, Copy, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Mode {
  #[default]
  TaskReport,
  TaskContext,
  Calendar,
  Error,
}

pub struct App {
  pub config: Config,
  pub tick_rate: f64,
  pub frame_rate: f64,
  pub components: Vec<Box<dyn Component>>,
  pub should_quit: bool,
  pub should_suspend: bool,
  pub mode: Mode,
  pub last_tick_key_events: Vec<KeyEvent>,
}

impl App {
  pub fn new(tick_rate: f64, frame_rate: f64, report: &str) -> Result<Self> {
    let app = TaskReport::new().report(report.into());
    let config = Config::new()?;
    let mode = Mode::TaskReport;
    Ok(Self {
      tick_rate,
      frame_rate,
      components: vec![Box::new(app)],
      should_quit: false,
      should_suspend: false,
      config,
      mode,
      last_tick_key_events: Vec::new(),
    })
  }

  pub async fn run(&mut self) -> Result<()> {
    let (command_tx, mut command_rx) = mpsc::unbounded_channel();

    let mut tui = tui::Tui::new()?;
    tui.tick_rate(self.tick_rate);
    tui.frame_rate(self.frame_rate);
    tui.enter()?;

    for component in self.components.iter_mut() {
      component.register_command_handler(command_tx.clone())?;
    }

    for component in self.components.iter_mut() {
      component.register_config_handler(self.config.clone())?;
    }

    for component in self.components.iter_mut() {
      component.init()?;
    }

    loop {
      if let Some(e) = tui.next().await {
        match e {
          tui::Event::Quit => command_tx.send(Command::Quit)?,
          tui::Event::Tick => command_tx.send(Command::Tick)?,
          tui::Event::Render => command_tx.send(Command::Render)?,
          tui::Event::Resize(x, y) => command_tx.send(Command::Resize(x, y))?,
          tui::Event::Key(key) => {
            self.last_tick_key_events.push(key);
            if let Some(keymap) = self.config.keybindings.get(&self.mode) {
              if let Some(command) = keymap.get(&self.last_tick_key_events) {
                command_tx.send(command.clone())?;
              };
            };
          },
          _ => {},
        }
        for component in self.components.iter_mut() {
          if let Some(command) = component.handle_events(Some(e.clone()))? {
            command_tx.send(command)?;
          }
        }
      }

      while let Ok(command) = command_rx.try_recv() {
        if command != Command::Tick && command != Command::Render {
          log::debug!("{command:?}");
        }
        match command {
          Command::Tick => {
            self.last_tick_key_events.drain(..);
          },
          Command::Quit => self.should_quit = true,
          Command::Suspend => self.should_suspend = true,
          Command::Resume => self.should_suspend = false,
          Command::Render => {
            tui.draw(|f| {
              for component in self.components.iter_mut() {
                let r = component.draw(f, f.size());
                if let Err(e) = r {
                  command_tx.send(Command::Error(format!("Failed to draw: {:?}", e))).unwrap();
                }
              }
            })?;
          },
          _ => {},
        }
        for component in self.components.iter_mut() {
          if let Some(command) = component.update(command.clone())? {
            command_tx.send(command)?
          };
        }
      }
      if self.should_suspend {
        tui.suspend()?;
        command_tx.send(Command::Resume)?;
        tui = tui::Tui::new()?;
        tui.tick_rate(self.tick_rate);
        tui.frame_rate(self.frame_rate);
        tui.enter()?;
      } else if self.should_quit {
        tui.stop()?;
        break;
      }
    }
    tui.exit()?;
    Ok(())
  }
}
