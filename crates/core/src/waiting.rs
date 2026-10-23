//! Detects an agent that stopped to ask the user something (a permission prompt, a y/n question).
//! Works on the tail of the terminal output; no parsing of the agent's own protocol.

/// Drops ANSI escape sequences (CSI, OSC) and carriage returns, keeping printable text.
pub fn strip_ansi(bytes: &[u8]) -> String {
    let s = String::from_utf8_lossy(bytes);
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars().peekable();
    while let Some(c) = it.next() {
        match c {
            '\x1b' => match it.peek() {
                Some('[') => {
                    it.next();
                    for n in it.by_ref() {
                        if ('@'..='~').contains(&n) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    it.next();
                    while let Some(n) = it.next() {
                        if n == '\x07' || (n == '\x1b' && it.peek() == Some(&'\\')) {
                            if n == '\x1b' {
                                it.next();
                            }
                            break;
                        }
                    }
                }
                _ => {
                    it.next();
                }
            },
            '\r' => {}
            c if c.is_control() && c != '\n' && c != '\t' => {}
            c => out.push(c),
        }
    }
    out
}

const MARKERS: &[&str] = &[
    "do you want to proceed",
    "do you want to make this edit",
    "do you want to create",
    "do you want to allow",
    "allow this",
    "(y/n)",
    "[y/n]",
    "[y/n/",
    "(yes/no)",
    "press enter to continue",
    "esc to cancel",
    "approve this",
    "waiting for your",
];

/// True when the last lines of the screen look like a question waiting for an answer.
pub fn needs_input(tail: &[u8]) -> bool {
    let text = strip_ansi(tail).to_lowercase();
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    let last: String = lines[lines.len().saturating_sub(12)..].join("\n");
    MARKERS.iter().any(|m| last.contains(m)) || (last.contains("❯ 1.") && last.contains("2."))
}

/// True when the screen's last lines (as rendered, see `Session::screen_lines`) contain a free-form question
/// the agent put to the user ("Uso X ou Y?"), which has no fixed marker. The TUI's own hints don't count.
pub fn asks_question(lines: &[String]) -> bool {
    let from = lines.len().saturating_sub(8);
    lines[from..].iter().any(|l| {
        let t = l.trim().trim_matches(|c: char| "│┃|>* ".contains(c)).trim();
        t.chars().count() >= 8 && t.ends_with('?') && !t.contains("for shortcuts") && !t.starts_with("? ")
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn free_form_question_is_detected() {
        let l = |s: &str| s.to_string();
        assert!(asks_question(&[l("Analisei o código."), l("● Uso a opção A ou a B?"), l("╭──╮"), l("│ > │")]));
        assert!(!asks_question(&[l("Tudo feito."), l("  ? for shortcuts")]));
        assert!(!asks_question(&[l("Terminei.")]));
    }

    #[test]
    fn claude_permission_prompt() {
        let screen =
            "\x1b[1mBash command\x1b[0m\n  npm install\n\n Do you want to proceed?\n \x1b[36m❯ 1. Yes\x1b[0m\n   2. No\n\n Esc to cancel";
        assert!(needs_input(screen.as_bytes()));
    }

    #[test]
    fn plain_question() {
        assert!(needs_input(b"Overwrite file? (y/n) "));
    }

    #[test]
    fn working_output_is_not_a_prompt() {
        assert!(!needs_input(b"\x1b[2KReading src/main.rs\nEditing 3 files\n"));
    }

    #[test]
    fn old_prompt_scrolled_away_is_ignored() {
        let mut s = String::from("Do you want to proceed?\n");
        for i in 0..30 {
            s.push_str(&format!("line {i}\n"));
        }
        assert!(!needs_input(s.as_bytes()));
    }
}
