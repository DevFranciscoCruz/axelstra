
use axelstra_daemon::client::Client;
use axelstra_daemon::server;
use serde_json::json;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

fn sh(cwd: &Path, args: &[&str]) {
    assert!(Command::new("git").current_dir(cwd).args(args).status().unwrap().success(), "git {args:?}");
}

#[test]
fn cards_crud_and_git_exclusion() {
    std::thread::spawn(|| {
        std::thread::sleep(Duration::from_secs(90));
        eprintln!("watchdog: test stuck");
        std::process::exit(1);
    });

    let root = std::env::temp_dir().join(format!("axelstra-kb-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let repo = root.join("r");
    std::fs::create_dir_all(&repo).unwrap();
    sh(&repo, &["init", "-q", "-b", "main"]);
    sh(&repo, &["config", "user.email", "t@t"]);
    sh(&repo, &["config", "user.name", "t"]);
    std::fs::write(repo.join("a.txt"), "x").unwrap();
    sh(&repo, &["add", "-A"]);
    sh(&repo, &["commit", "-qm", "init"]);

    let pipe = format!("{}-kb-{}", axelstra_daemon::proto::pipe_name(), std::process::id());
    let token = "k".repeat(32);
    let (p, t, s) = (pipe.clone(), token.clone(), root.join("projects.json"));
    std::thread::spawn(move || {
        tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().unwrap()
            .block_on(server::serve_with_store(p, t, s)).unwrap();
    });
    std::thread::sleep(Duration::from_millis(300));
    let mut c = Client::connect(&pipe, &token).unwrap();
    let pid = c.call("project.add", json!({"path": &repo})).unwrap()["id"].as_str().unwrap().to_string();

    assert_eq!(c.call("kanban.list", json!({"project": pid})).unwrap()["cards"], json!([]));

    // free text becomes a card whose id comes from the title
    let a = c.call("kanban.save", json!({"project": pid, "title": "Add Login Page!", "body": "make it pretty\n\nwith **markdown**"})).unwrap();
    assert_eq!(a["meta"]["id"], "add-login-page");
    assert_eq!(a["meta"]["status"], "backlog");
    let b = c.call("kanban.save", json!({"project": pid, "title": "Add login page"})).unwrap();
    assert_eq!(b["meta"]["id"], "add-login-page-2", "ids never collide");

    // the folder is hidden from git, and the user's tracked files are untouched
    assert!(repo.join(".kanban/add-login-page.md").exists());
    let st = Command::new("git").current_dir(&repo).args(["status", "--porcelain"]).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&st.stdout).trim(), "", "kanban must not show as untracked");
    assert!(!repo.join(".gitignore").exists());

    // edit + dependency + move keep the body
    c.call("kanban.save", json!({"project": pid, "id": "add-login-page-2", "title": "Second", "body": "b2", "depends_on": ["add-login-page"], "priority": 3})).unwrap();
    c.call("kanban.move", json!({"project": pid, "id": "add-login-page-2", "status": "todo"})).unwrap();
    let board = c.call("kanban.list", json!({"project": pid})).unwrap();
    let card = board["cards"].as_array().unwrap().iter().find(|x| x["meta"]["id"] == "add-login-page-2").unwrap();
    assert_eq!(card["meta"]["status"], "todo");
    assert_eq!(card["meta"]["depends_on"], json!(["add-login-page"]));
    assert_eq!(card["body"].as_str().unwrap().trim(), "b2");
    // higher priority first
    assert_eq!(board["cards"][0]["meta"]["id"], "add-login-page-2");

    // bad input is rejected, a hand-broken card is reported instead of hiding the board
    assert!(c.call("kanban.move", json!({"project": pid, "id": "add-login-page", "status": "bogus"})).is_err());
    assert!(c.call("kanban.save", json!({"project": pid, "id": "add-login-page", "title": "x", "depends_on": ["add-login-page"]})).is_err(), "self dependency");
    assert!(c.call("kanban.delete", json!({"project": pid, "id": "..\\x"})).is_err());
    std::fs::write(repo.join(".kanban/oops.md"), "no frontmatter").unwrap();
    let board = c.call("kanban.list", json!({"project": pid})).unwrap();
    assert_eq!(board["cards"].as_array().unwrap().len(), 2);
    assert_eq!(board["broken"][0]["file"], "oops.md");

    c.call("kanban.delete", json!({"project": pid, "id": "add-login-page"})).unwrap();
    assert!(!repo.join(".kanban/add-login-page.md").exists());
}
