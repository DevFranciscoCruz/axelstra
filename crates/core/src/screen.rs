//! What a terminal actually shows. Raw PTY output of a TUI agent is a stream of cursor moves and partial
//! redraws: stripping the escape codes leaves glued words and repeated fragments. Replaying the bytes through a
//! terminal emulator gives the real lines instead.

/// The last `lines` non-empty lines of the screen (history included) after replaying `bytes` on a terminal of
/// `cols` x `rows`.
pub fn last_lines(bytes: &[u8], cols: u16, rows: u16, lines: usize) -> Vec<String> {
    let (cols, rows) = (cols.clamp(20, 400), rows.clamp(5, 200));
    let mut parser = vt100::Parser::new(rows, cols, 5000);
    parser.process(bytes);
    let sc = parser.screen_mut();
    sc.set_scrollback(usize::MAX);
    let history = sc.scrollback(); // clamped to what exists
    let page = rows as usize;
    let mut out: Vec<String> = Vec::new();
    // pages from the oldest view to the live one; `next` is the first absolute line not yet taken
    let mut offset = history;
    let mut next = 0usize;
    loop {
        sc.set_scrollback(offset);
        let top = history - offset; // absolute index of the first visible row
        for (i, row) in sc.rows(0, cols).enumerate() {
            if top + i >= next {
                out.push(row.trim_end().to_string());
            }
        }
        next = top + page;
        if offset == 0 {
            break;
        }
        offset = offset.saturating_sub(page);
    }
    let end = out.iter().rposition(|l| !l.is_empty()).map_or(0, |i| i + 1);
    out.truncate(end);
    let kept: Vec<String> = out.into_iter().filter(|l| !l.is_empty()).collect();
    kept[kept.len().saturating_sub(lines)..].to_vec()
}

/// The agent's last reply as plain text, out of a screen full of TUI chrome (banner, tool-run lines, prompt box,
/// status bar). Looks for the last `●` message block and stops at the first piece of chrome after it. `None` when
/// the screen has no such block (not a Claude-style TUI), so the caller can fall back to the raw lines.
pub fn last_reply(lines: &[String]) -> Option<String> {
    let is_chrome = |l: &str| {
        let t = l.trim_start();
        t.starts_with('─') || t.starts_with('✻') || t.starts_with('✢') || t.starts_with('✶') || t.starts_with('*') && t.contains("for ")
            || t.starts_with('>') || t.starts_with('❯') || t.starts_with('⏵') || t.starts_with("Model:") || t.starts_with("? for shortcuts")
    };
    let is_tool_call = |t: &str| {
        let name = t.split('(').next().unwrap_or("");
        t.ends_with(')') && !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == ':')
    };
    let start = lines.iter().rposition(|l| {
        l.trim_start().strip_prefix('●').is_some_and(|rest| {
            let rest = rest.trim();
            !rest.is_empty() && !is_tool_call(rest)
        })
    })?;
    let mut out: Vec<String> = Vec::new();
    for (i, l) in lines[start..].iter().enumerate() {
        if i > 0 && is_chrome(l) {
            break;
        }
        let t = l.trim_end();
        out.push(if i == 0 { t.trim_start().trim_start_matches('●').trim_start().to_string() } else { t.to_string() });
    }
    // continuation lines are indented by the bullet width; drop that uniform indent
    let indent = out.iter().skip(1).filter(|l| !l.trim().is_empty()).map(|l| l.len() - l.trim_start().len()).min().unwrap_or(0).min(2);
    let text: Vec<String> = out.into_iter().enumerate().map(|(i, l)| if i == 0 { l } else { l.chars().skip(indent).collect() }).collect();
    Some(text.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::{last_lines, last_reply};

    #[test]
    fn reply_is_the_last_message_block_only() {
        let l: Vec<String> = [
            "Claude Code v2.1.308",
            "● Splitting the handlers.",
            "Ran 4 shell commands",
            "● Both gate failures are fixed.",
            "  - Test assertions: done.",
            "  To finish, allow the amend.",
            "✻ Brewed for 2m 43s · done",
            "──────────",
            "> amend the commit",
            "Model: Sonnet 5.5",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        assert_eq!(
            last_reply(&l).unwrap(),
            "Both gate failures are fixed.\n- Test assertions: done.\nTo finish, allow the amend."
        );
    }

    #[test]
    fn no_bullet_means_no_reply() {
        assert!(last_reply(&["plain shell output".to_string()]).is_none());
    }

    #[test]
    fn cursor_moves_do_not_glue_words() {
        // "Your message" drawn with a cursor-forward instead of a space, then a redraw of the line
        let bytes = b"Your\x1b[1Cmessage\x1b[1Cwas\x1b[1Ccut\r\nsecond line\r\n";
        assert_eq!(last_lines(bytes, 80, 24, 10), vec!["Your message was cut", "second line"]);
    }

    #[test]
    fn a_redrawn_line_shows_once() {
        let bytes = b"working 1\rworking 2\rworking 3\r\ndone\r\n";
        assert_eq!(last_lines(bytes, 80, 24, 10), vec!["working 3", "done"]);
    }

    #[test]
    fn history_beyond_one_screen_is_kept() {
        let mut bytes = Vec::new();
        for i in 1..=60 {
            bytes.extend(format!("line {i}\r\n").bytes());
        }
        let got = last_lines(&bytes, 80, 10, 40);
        assert_eq!(got.len(), 40);
        assert_eq!(got.first().map(String::as_str), Some("line 21"));
        assert_eq!(got.last().map(String::as_str), Some("line 60"));
    }
}
