//! `axelstra start --trust-acceptance`: shows what the todo cards will run (acceptance commands, permission modes)
//! and, for `bypass*` cards, demands an explicit confirmation before `orch::run_pinned` is called.

use anyhow::{bail, Result};
use axelstra_core::orch::{self, Options, Report, TrustInfo};
use std::io::{BufRead, IsTerminal, Write};

/// What to do with a run once the cards are known.
#[derive(Debug, PartialEq, Eq)]
pub enum Decision {
    /// No `bypass*` card: `orch::run_trusted`.
    Trusted,
    /// `bypass*` cards, explicitly confirmed: `orch::run_pinned` with this pin.
    Pinned(String),
    /// Nothing runs.
    Refuse(String),
}

/// `confirm_pin`: the `--confirm-bypass` value. `typed`: the line read from an interactive terminal, `None` when
/// stdin is not a terminal (or was not asked). The pin wins when given: a wrong pin is never rescued by a typed word.
pub fn decide(info: &TrustInfo, confirm_pin: Option<&str>, typed: Option<&str>) -> Decision {
    if !info.cards.iter().any(|c| c.bypass) {
        return Decision::Trusted;
    }
    if let Some(p) = confirm_pin {
        return if p.trim() == info.pin {
            Decision::Pinned(info.pin.clone())
        } else {
            Decision::Refuse("--confirm-bypass does not match the pin of the current todo cards (edited since?)".into())
        };
    }
    match typed {
        Some(t) if t.trim() == "bypass" => Decision::Pinned(info.pin.clone()),
        Some(_) => Decision::Refuse("bypass not confirmed".into()),
        None => Decision::Refuse(format!(
            "cards with a bypass permission_mode need confirmation: run in a terminal, or pass --confirm-bypass {}",
            info.pin
        )),
    }
}

pub fn run(opts: &Options, confirm_bypass: Option<&str>) -> Result<Report> {
    let info = orch::trust_info(&opts.repo)?;
    for c in &info.cards {
        eprintln!("card {}: agent={} permission_mode={}{}", c.id, c.agent, c.permission_mode, if c.bypass { "  [BYPASS]" } else { "" });
        if let Some(a) = &c.acceptance {
            eprintln!("  acceptance: {a}");
        }
    }
    let has_bypass = info.cards.iter().any(|c| c.bypass);
    let mut typed = None;
    if has_bypass {
        eprintln!("pin: {}", info.pin);
        if confirm_bypass.is_none() && std::io::stdin().is_terminal() {
            eprint!("These cards run agents with permission checks off. Type 'bypass' to continue: ");
            std::io::stderr().flush().ok();
            let mut line = String::new();
            std::io::stdin().lock().read_line(&mut line)?;
            typed = Some(line);
        }
    }
    match decide(&info, confirm_bypass, typed.as_deref()) {
        Decision::Trusted => orch::run_trusted(opts),
        Decision::Pinned(pin) => orch::run_pinned(opts, &pin),
        Decision::Refuse(why) => bail!("{why}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axelstra_core::orch::TrustCard;

    fn info(bypass: bool) -> TrustInfo {
        let card = |id: &str, b: bool| TrustCard {
            id: id.into(),
            agent: "claude".into(),
            permission_mode: if b { "bypassPermissions".into() } else { "default".into() },
            bypass: b,
            acceptance: None,
        };
        let mut cards = vec![card("a", false)];
        if bypass {
            cards.push(card("b", true));
        }
        TrustInfo { cards, pin: "PIN123".into() }
    }

    #[test]
    fn no_bypass_cards_run_as_before() {
        assert_eq!(decide(&info(false), None, None), Decision::Trusted);
        assert_eq!(decide(&info(false), Some("wrong"), Some("no")), Decision::Trusted);
    }

    #[test]
    fn bypass_without_tty_or_pin_is_refused() {
        match decide(&info(true), None, None) {
            Decision::Refuse(m) => assert!(m.contains("PIN123")),
            d => panic!("{d:?}"),
        }
    }

    #[test]
    fn wrong_pin_is_refused_even_with_the_word_typed() {
        assert!(matches!(decide(&info(true), Some("nope"), Some("bypass")), Decision::Refuse(_)));
    }

    #[test]
    fn right_pin_is_accepted() {
        assert_eq!(decide(&info(true), Some("PIN123"), None), Decision::Pinned("PIN123".into()));
    }

    #[test]
    fn typed_word_must_be_exactly_bypass() {
        assert_eq!(decide(&info(true), None, Some("bypass\n")), Decision::Pinned("PIN123".into()));
        assert!(matches!(decide(&info(true), None, Some("yes\n")), Decision::Refuse(_)));
        assert!(matches!(decide(&info(true), None, Some("")), Decision::Refuse(_)));
    }
}
