//! Palette and terminal control sequences for the Onyun CLI.

use std::fmt;

#[derive(Clone, Copy)]
pub(crate) enum Color {
    Heading,
    Text,
    Dim,
    Muted,
    Accent,
    Error,
}

pub(crate) struct Styled<'a> {
    text: &'a str,
    color: Color,
    enabled: bool,
}

pub(crate) const fn paint(text: &str, color: Color, enabled: bool) -> Styled<'_> {
    Styled {
        text,
        color,
        enabled,
    }
}

impl fmt::Display for Styled<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if !self.enabled {
            return f.write_str(self.text);
        }
        // Adaptive ANSI attributes & standard palette slots remain legible on both
        // light and dark terminal backgrounds. Amber/yellow (33) is Onyun's accent.
        let code = match self.color {
            Color::Heading => "1",
            Color::Text => return f.write_str(self.text),
            Color::Dim | Color::Muted => "2",
            Color::Accent => "33",
            Color::Error => "1;31",
        };
        write!(f, "\x1b[{code}m{}\x1b[0m", self.text)
    }
}

pub(crate) const fn clear_line() -> &'static str {
    "\r\x1b[2K"
}
