use std::io::{self, Stdout};

use color_eyre::eyre::Result;
use crossterm::{
    cursor::{Hide, Show},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};

pub type CrosstermTerminal = Terminal<CrosstermBackend<Stdout>>;

pub struct Tui {
    terminal: CrosstermTerminal,
}

impl Tui {
    pub fn new() -> Result<Self> {
        let backend = CrosstermBackend::new(io::stdout());
        let terminal = Terminal::new(backend)?;
        Ok(Self { terminal })
    }

    pub fn enter(&mut self) -> Result<()> {
        struct EnterGuard {
            armed: bool,
        }

        impl Drop for EnterGuard {
            fn drop(&mut self) {
                if self.armed {
                    let _ = Tui::reset();
                }
            }
        }

        let mut guard = EnterGuard { armed: true };

        enable_raw_mode()?;
        execute!(io::stdout(), EnterAlternateScreen, Hide)?;

        self.terminal.clear()?;

        // Install a panic hook that restores the terminal before printing the panic.
        let original_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |panic_info| {
            let _ = Self::reset();
            original_hook(panic_info);
        }));

        guard.armed = false;
        Ok(())
    }

    pub fn exit(&mut self) -> Result<()> {
        Self::reset()?;
        Ok(())
    }

    pub fn draw<F>(&mut self, f: F) -> Result<()>
    where
        F: FnOnce(&mut ratatui::Frame),
    {
        self.terminal.draw(f)?;
        Ok(())
    }

    fn reset() -> Result<()> {
        disable_raw_mode()?;
        execute!(io::stdout(), Show, LeaveAlternateScreen)?;
        Ok(())
    }
}
