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

#[cfg(test)]
mod tests {
    use super::last_lines;

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
