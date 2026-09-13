//! Terminal mode detection (port of `shared/TerminalMode.scala`).

/// Whether the process has an interactive console (port of `TerminalMode.readMode`, the
/// `System.console() != null` check).
pub struct TerminalMode;

impl TerminalMode {
    pub fn read_mode() -> bool {
        use std::io::IsTerminal;
        std::io::stdin().is_terminal()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `read_mode` is the port of `System.console() != null`: it reports whether **stdin** is a
    /// terminal. The assertion necessarily restates the `std` call it delegates to, but that call is
    /// the contract: it is total — no `expect`, no panic when the process has no console at all —
    /// and it is `stdin`, not the stdout or stderr a reader might assume.
    #[test]
    fn read_mode_reports_whether_stdin_is_a_terminal_without_panicking() {
        use std::io::IsTerminal;
        assert_eq!(TerminalMode::read_mode(), std::io::stdin().is_terminal());
        // Called twice: it is a query, not a one-shot latch.
        assert_eq!(TerminalMode::read_mode(), TerminalMode::read_mode());
    }
}
