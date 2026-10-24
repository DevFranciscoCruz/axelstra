//! `axelstra computer`: look at the screen and drive mouse and keyboard, for agents that need to see a real
//! desktop app. Off until the user turns it on in the app. The work is done by the daemon (`computer.*` calls),
//! which holds the switch and refuses everything while it is off; this is only its command line.

use super::{Cmd, CompCmd};
use anyhow::{bail, Result};
use axelstra_daemon::client::Client;
use base64::Engine;
use serde_json::json;

pub fn run(cmd: Cmd, mut c: Client) -> Result<()> {
    let Cmd::Computer { cmd } = cmd else { unreachable!() };
    match cmd {
        CompCmd::Windows { json } => {
            let r = c.call("computer.windows", json!({}))?;
            let list = r["windows"].as_array().cloned().unwrap_or_default();
            if json {
                println!("{}", serde_json::to_string_pretty(&list)?);
            } else {
                for w in list {
                    let s = |k: &str| w[k].as_str().unwrap_or_default().to_string();
                    println!("{}\t{}\t{}x{}+{}+{}\t{}", w["id"], s("process"), w["width"], w["height"], w["x"], w["y"], s("title"));
                }
            }
        }
        CompCmd::Screenshot { window, out } => {
            let path = out.unwrap_or_else(|| std::env::temp_dir().join(format!("axelstra-shot-{}.png", std::process::id())));
            let r = c.call("computer.screenshot", json!({"window": window}))?;
            let Some(b64) = r["png"].as_str() else { bail!("the daemon sent no image") };
            std::fs::write(&path, base64::engine::general_purpose::STANDARD.decode(b64)?)?;
            println!("{} ({}x{})", path.display(), r["width"], r["height"]);
        }
        CompCmd::Click { x, y, window, button, double } => {
            c.call("computer.click", json!({"x": x, "y": y, "window": window, "button": button, "double": double}))?;
        }
        CompCmd::Type { text, window } => {
            c.call("computer.type", json!({"text": text, "window": window}))?;
        }
        CompCmd::Key { combo, window } => {
            c.call("computer.key", json!({"combo": combo, "window": window}))?;
        }
        CompCmd::Scroll { dy, dx, x, y, window } => {
            c.call("computer.scroll", json!({"dy": dy, "dx": dx, "x": x, "y": y, "window": window}))?;
        }
        CompCmd::Focus { window } => {
            c.call("computer.focus", json!({"window": window}))?;
        }
    }
    Ok(())
}
