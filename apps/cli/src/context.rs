use std::io;
use std::io::IsTerminal;

/// How the process was invoked, captured once at startup. Prompt-or-fail
/// decisions read this instead of re-probing the terminal at each site.
#[derive(Clone, Copy)]
pub(crate) struct ExecContext {
    /// Stdin is a TTY, so commands may ask questions.
    pub(crate) interactive: bool,
}

impl ExecContext {
    pub(crate) fn detect() -> Self {
        Self {
            interactive: io::stdin().is_terminal(),
        }
    }
}
