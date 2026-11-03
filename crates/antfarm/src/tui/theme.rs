//! The 16-color palette from the design: coral for waiting, amber for working, green for
//! finished, grey for still and exited. Shape always repeats the color, so `NO_COLOR`
//! loses nothing but color.

use ratatui::style::{Color, Modifier, Style};

use super::view::Class;

#[derive(Debug, Clone, Copy)]
pub struct Theme {
    pub color: bool,
}

impl Theme {
    pub fn from_env() -> Self {
        Self {
            color: std::env::var_os("NO_COLOR").is_none_or(|v| v.is_empty()),
        }
    }

    fn fg(&self, c: Color) -> Style {
        if self.color {
            Style::new().fg(c)
        } else {
            Style::new()
        }
    }

    pub fn class(&self, c: Class) -> Style {
        match c {
            Class::Waiting => self.fg(Color::LightMagenta).add_modifier(Modifier::BOLD),
            Class::Working => self.fg(Color::Yellow),
            Class::Finished | Class::Idle => self.fg(Color::Green),
            Class::Still | Class::Exited => self.fg(Color::DarkGray),
        }
    }

    pub fn dim(&self) -> Style {
        if self.color {
            Style::new().fg(Color::DarkGray)
        } else {
            Style::new().add_modifier(Modifier::DIM)
        }
    }

    pub fn label(&self) -> Style {
        self.dim()
    }

    pub fn strong(&self) -> Style {
        Style::new().add_modifier(Modifier::BOLD)
    }

    pub fn brand(&self) -> Style {
        if self.color {
            Style::new()
                .fg(Color::Black)
                .bg(Color::Yellow)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::new().add_modifier(Modifier::REVERSED | Modifier::BOLD)
        }
    }

    pub fn key(&self) -> Style {
        if self.color {
            Style::new().fg(Color::Black).bg(Color::Cyan)
        } else {
            Style::new().add_modifier(Modifier::REVERSED)
        }
    }

    pub fn waiting_band(&self) -> Style {
        if self.color {
            Style::new()
                .fg(Color::Black)
                .bg(Color::LightMagenta)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::new().add_modifier(Modifier::REVERSED | Modifier::BOLD)
        }
    }

    pub fn selected(&self) -> Style {
        Style::new().add_modifier(Modifier::REVERSED)
    }

    pub fn error(&self) -> Style {
        self.fg(Color::LightRed).add_modifier(Modifier::BOLD)
    }

    /// Bar color per tool family in the swimlanes. Ask and permission share the waiting
    /// color, since they are what the human answers.
    pub fn category(&self, c: antfarm_core::ToolCategory) -> Style {
        use antfarm_core::ToolCategory as C;
        self.fg(match c {
            C::Shell => Color::Blue,
            C::Read => Color::Cyan,
            C::Edit => Color::Magenta,
            C::Search => Color::LightCyan,
            C::Web => Color::LightBlue,
            C::Spawn => Color::Green,
            C::Ask => Color::LightMagenta,
            C::Other => Color::Gray,
        })
    }

    pub fn ok(&self) -> Style {
        self.fg(Color::Green)
    }
}
