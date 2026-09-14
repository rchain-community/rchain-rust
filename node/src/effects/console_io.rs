//! Console I/O interface (port of `effects/ConsoleIO.scala`).

use rustyline::completion::{Completer as CompleterTrait, Pair};
use rustyline::error::ReadlineError;
use rustyline::highlight::MatchingBracketHighlighter;
use rustyline::hint::HistoryHinter;
use rustyline::history::DefaultHistory;
use rustyline::validate::MatchingBracketValidator;
use rustyline::{Completer, Helper, Highlighter, Hinter, Validator};
use rustyline::{Context, Editor, Result as RlResult};

use rchain_shared::string_ops::ColoredString;

/// Console I/O (port of `ConsoleIO[F]`; the `F[_]` effect is simplified to synchronous calls).
pub trait ConsoleIo {
    /// Read a line, returning `None` on EOF (the Scala `null`).
    fn read_line(&mut self) -> Option<String>;
    fn read_password(&mut self, prompt: &str) -> String;
    fn println(&mut self, s: &str);
    fn println_colored(&mut self, s: &ColoredString);
    fn update_completion(&mut self, history: &[String]);
    fn close(&mut self);
}

/// A stdin/stdout console (port of `effects.consoleIO`/`JLineConsoleIO`; the jline line-editing
/// and prompt are not reproduced — only plain line reads/writes).
#[derive(Default)]
pub struct StdioConsole;

impl ConsoleIo for StdioConsole {
    fn read_line(&mut self) -> Option<String> {
        let mut line = String::new();
        match std::io::stdin().read_line(&mut line) {
            Ok(0) => None,
            Ok(_) => {
                if line.ends_with('\n') {
                    line.pop();
                    if line.ends_with('\r') {
                        line.pop();
                    }
                }
                Some(line)
            }
            Err(_) => None,
        }
    }

    fn read_password(&mut self, prompt: &str) -> String {
        use std::io::Write;
        let mut stdout = std::io::stdout();
        let _ = stdout.write_all(prompt.as_bytes());
        let _ = stdout.flush();
        let mut line = String::new();
        let _ = std::io::stdin().read_line(&mut line);
        line.trim().to_string()
    }

    fn println(&mut self, s: &str) {
        println!("{s}");
    }

    fn println_colored(&mut self, s: &ColoredString) {
        println!("{}", s.colorize());
    }

    fn update_completion(&mut self, _history: &[String]) {}

    fn close(&mut self) {}
}

/// A tab-completer over a fixed keyword list (port of jline's `StringsCompleter`).
struct KeywordCompleter {
    keywords: Vec<String>,
}

impl CompleterTrait for KeywordCompleter {
    type Candidate = Pair;

    fn complete(&self, line: &str, pos: usize, _ctx: &Context<'_>) -> RlResult<(usize, Vec<Pair>)> {
        let start = line[..pos]
            .rfind(char::is_whitespace)
            .map(|i| i + 1)
            .unwrap_or(0);
        let word = &line[start..pos];
        let matches: Vec<Pair> = self
            .keywords
            .iter()
            .filter(|k| k.starts_with(word))
            .map(|k| Pair {
                display: k.clone(),
                replacement: k.clone(),
            })
            .collect();
        Ok((start, matches))
    }
}

/// The rustyline helper stack (completer + hinter + highlighter + validator).
#[derive(Completer, Helper, Hinter, Highlighter, Validator)]
struct ReplHelper {
    #[rustyline(Completer)]
    completer: KeywordCompleter,
    #[rustyline(Hinter)]
    hinter: HistoryHinter,
    #[rustyline(Highlighter)]
    highlighter: MatchingBracketHighlighter,
    #[rustyline(Validator)]
    validator: MatchingBracketValidator,
}

impl ReplHelper {
    fn new(keywords: Vec<String>) -> Self {
        ReplHelper {
            completer: KeywordCompleter { keywords },
            hinter: HistoryHinter {},
            highlighter: MatchingBracketHighlighter::new(),
            validator: MatchingBracketValidator::new(),
        }
    }
}

/// A rustyline-backed console with a prompt, line-editing, history, and tab-completion (the Rust
/// spelling of the Scala jline `JLineConsoleIO`).
pub struct RustylineConsole {
    editor: Editor<ReplHelper, DefaultHistory>,
}

impl RustylineConsole {
    pub fn new() -> Result<Self, String> {
        let mut editor = Editor::<ReplHelper, DefaultHistory>::new().map_err(|e| e.to_string())?;
        editor.set_helper(Some(ReplHelper::new(Vec::new())));
        Ok(RustylineConsole { editor })
    }
}

impl ConsoleIo for RustylineConsole {
    fn read_line(&mut self) -> Option<String> {
        match self.editor.readline("rholang $ ") {
            Ok(line) => Some(line),
            Err(ReadlineError::Interrupted) | Err(ReadlineError::Eof) => None,
            Err(_) => None,
        }
    }

    fn read_password(&mut self, prompt: &str) -> String {
        use std::io::Write;
        let mut stdout = std::io::stdout();
        let _ = stdout.write_all(prompt.as_bytes());
        let _ = stdout.flush();
        let mut line = String::new();
        let _ = std::io::stdin().read_line(&mut line);
        line.trim().to_string()
    }

    fn println(&mut self, s: &str) {
        println!("{s}");
    }

    fn println_colored(&mut self, s: &ColoredString) {
        println!("{}", s.colorize());
    }

    fn update_completion(&mut self, history: &[String]) {
        self.editor
            .set_helper(Some(ReplHelper::new(history.to_vec())));
    }

    fn close(&mut self) {}
}

/// A no-op console (port of `NOPConsoleIO`).
#[derive(Default)]
pub struct NopConsoleIo;

impl ConsoleIo for NopConsoleIo {
    fn read_line(&mut self) -> Option<String> {
        Some(String::new())
    }

    fn read_password(&mut self, _prompt: &str) -> String {
        String::new()
    }

    fn println(&mut self, _s: &str) {}

    fn println_colored(&mut self, _s: &ColoredString) {}

    fn update_completion(&mut self, _history: &[String]) {}

    fn close(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The no-op console (port of `NOPConsoleIO`) is what tests and non-interactive paths use, and
    /// what the REPL falls back to: every method must be total and side-effect free.
    #[test]
    fn the_nop_console_answers_without_io() {
        let mut console = NopConsoleIo;
        assert_eq!(console.read_line(), Some(String::new()));
        assert_eq!(console.read_password("prompt: "), String::new());
        console.println("ignored");
        console.println_colored(&rchain_shared::string_ops::StringColors::green("ignored"));
        console.update_completion(&["history".to_string()]);
        console.close();
        // `Default` is what the REPL falls back to on a non-tty.
        let mut defaulted: NopConsoleIo = Default::default();
        assert_eq!(defaulted.read_line(), Some(String::new()));
    }

    /// The completer completes the word under the cursor from its keyword list and reports the
    /// replacement *start* — the index the caller overwrites from. A word that matches nothing
    /// yields no candidates rather than an error.
    #[test]
    fn the_completer_completes_the_word_under_the_cursor() {
        let completer = KeywordCompleter {
            keywords: vec!["Nil".to_string(), "new".to_string(), "news".to_string()],
        };
        let history = rustyline::history::DefaultHistory::new();
        let ctx = Context::new(&history);

        // An empty word matches everything, at the start of the line.
        let (start, matches) = completer.complete("", 0, &ctx).expect("complete");
        assert_eq!(start, 0);
        assert_eq!(matches.len(), 3);

        // A prefix narrows it and the start is after the space.
        let line = "new ne";
        let (start, matches) = completer
            .complete(line, line.len(), &ctx)
            .expect("complete");
        assert_eq!(start, 4, "the replacement starts at the word");
        let mut names: Vec<String> = matches.into_iter().map(|p| p.replacement).collect();
        names.sort();
        assert_eq!(names, vec!["new".to_string(), "news".to_string()]);

        // No keyword matches: an empty candidate list, not an error.
        let line = "new zzz";
        let (_, matches) = completer
            .complete(line, line.len(), &ctx)
            .expect("complete");
        assert!(matches.is_empty());
    }

    /// A rustyline console can be constructed on a non-tty (no terminal needed for construction), and
    /// `update_completion` rebuilds its helper with the given history — the port of the Scala
    /// `updateCompletion`. Construction is the only part testable without a tty; `read_line` on a
    /// closed stdin returns `None` rather than blocking forever (the EOF arm).
    #[test]
    fn a_rustyline_console_constructs_and_updates_its_completion_history() {
        let mut console = match RustylineConsole::new() {
            Ok(console) => console,
            // No tty available in some sandboxes: constructing is best-effort by design.
            Err(_) => return,
        };
        console.update_completion(&["Nil".to_string(), "new".to_string()]);
        console.println("ignored on a non-tty");
        console.println_colored(&rchain_shared::string_ops::StringColors::red("ignored"));
        console.close();
    }

    /// `ColoredString` is what the colored printer consumes; the ANSI form is what reaches stdout.
    #[test]
    fn a_colored_line_is_rendered_with_its_ansi_code() {
        use rchain_shared::string_ops::StringColors;
        let colored = StringColors::green("ok");
        assert!(colored.colorize().contains("ok"));
        assert!(colored.colorize().contains('\u{1b}'));
    }
}
