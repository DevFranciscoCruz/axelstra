//! Texts of the native dialogs. The host shows these itself (a page must not be able to answer them), so it cannot use the
//! frontend dictionaries: it reads the language the UI is set to (`axelstra.locale` in the `ui` map of the daemon's
//! settings) and picks the sentence here. Same three languages as the app; anything else is pt-PT, the source language.
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lang {
    Pt,
    En,
    PtBr,
}

/// The UI language from a `settings.get` answer.
pub fn lang_from_settings(settings: &Value) -> Lang {
    match settings["ui"]["axelstra.locale"].as_str() {
        Some("en") => Lang::En,
        Some("pt-BR") => Lang::PtBr,
        _ => Lang::Pt,
    }
}

pub struct Confirm {
    pub title: String,
    pub text: String,
    pub ok: &'static str,
    pub cancel: &'static str,
}

/// Button labels of every confirmation (the OS default would follow the system language, not the app's).
fn buttons(l: Lang) -> (&'static str, &'static str) {
    match l {
        Lang::En => ("OK", "Cancel"),
        Lang::Pt | Lang::PtBr => ("OK", "Cancelar"),
    }
}

fn confirm(l: Lang, title: &str, text: String) -> Confirm {
    let (ok, cancel) = buttons(l);
    Confirm { title: title.to_string(), text, ok, cancel }
}

pub fn add_host(l: Lang, name: &str, spec: &str) -> Confirm {
    match l {
        Lang::En => confirm(
            l,
            "Add remote host",
            format!("Add the remote host \"{name}\"?\n\nAxelstra will run `ssh`/`wsl.exe` for {spec} and connect to the daemon running there."),
        ),
        Lang::Pt => confirm(
            l,
            "Adicionar anfitrião remoto",
            format!("Adicionar o anfitrião remoto \"{name}\"?\n\nO Axelstra vai executar `ssh`/`wsl.exe` para {spec} e ligar-se ao daemon que lá corre."),
        ),
        Lang::PtBr => confirm(
            l,
            "Adicionar host remoto",
            format!("Adicionar o host remoto \"{name}\"?\n\nO Axelstra vai executar `ssh`/`wsl.exe` para {spec} e se conectar ao daemon que está rodando lá."),
        ),
    }
}

pub fn remove_host(l: Lang, id: &str) -> Confirm {
    match l {
        Lang::En => confirm(l, "Remove remote host", format!("Remove the host \"{id}\"? What runs there keeps running.")),
        Lang::Pt => confirm(l, "Remover anfitrião remoto", format!("Remover o anfitrião \"{id}\"? O que corre lá continua a correr.")),
        Lang::PtBr => confirm(l, "Remover host remoto", format!("Remover o host \"{id}\"? O que está rodando lá continua rodando.")),
    }
}

pub fn add_remote_project(l: Lang, path: &str, host: &str) -> Confirm {
    match l {
        Lang::En => confirm(l, "Add remote project", format!("Add {path} on {host} as a project?")),
        Lang::Pt => confirm(l, "Adicionar projeto remoto", format!("Adicionar {path} em {host} como projeto?")),
        Lang::PtBr => confirm(l, "Adicionar projeto remoto", format!("Adicionar {path} em {host} como projeto?")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn language_comes_from_the_ui_settings() {
        assert_eq!(lang_from_settings(&json!({"ui": {"axelstra.locale": "en"}})), Lang::En);
        assert_eq!(lang_from_settings(&json!({"ui": {"axelstra.locale": "pt-BR"}})), Lang::PtBr);
        assert_eq!(lang_from_settings(&json!({"ui": {"axelstra.locale": "pt-PT"}})), Lang::Pt);
        assert_eq!(lang_from_settings(&json!({"ui": {}})), Lang::Pt);
        assert_eq!(lang_from_settings(&json!({})), Lang::Pt);
        assert_eq!(lang_from_settings(&json!({"ui": {"axelstra.locale": "xx"}})), Lang::Pt);
    }

    #[test]
    fn every_dialog_exists_in_every_language_and_keeps_its_values() {
        for l in [Lang::Pt, Lang::En, Lang::PtBr] {
            for c in [add_host(l, "lab", "me@box"), remove_host(l, "lab"), add_remote_project(l, "/srv/p", "lab")] {
                assert!(!c.title.is_empty() && !c.ok.is_empty() && !c.cancel.is_empty());
                assert!(c.text.contains("lab"), "{}", c.text);
            }
            assert!(add_host(l, "lab", "me@box").text.contains("me@box"));
            assert!(add_remote_project(l, "/srv/p", "lab").text.contains("/srv/p"));
        }
        // the three languages really differ where the words differ
        assert_ne!(add_host(Lang::En, "x", "y").title, add_host(Lang::Pt, "x", "y").title);
        assert_ne!(add_host(Lang::PtBr, "x", "y").title, add_host(Lang::Pt, "x", "y").title);
        assert_eq!(add_host(Lang::En, "x", "y").cancel, "Cancel");
    }
}
