use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Child, Command, Output, Stdio};
use std::sync::{Arc, Mutex};

use rusqlite::Connection;
use serde_json::Value as Json;
use tempfile::TempDir;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Structure, Value};

type Layout = (i32, HashMap<String, OwnedValue>, Vec<OwnedValue>);

struct Fixture {
    home: TempDir,
    bus: Child,
    address: String,
}

impl Fixture {
    fn new(schema: i32) -> Self {
        let home = tempfile::tempdir().unwrap();
        let app_dir = home.path().join(".cc-switch");
        std::fs::create_dir(&app_dir).unwrap();
        let db = Connection::open(app_dir.join("cc-switch.db")).unwrap();
        db.execute_batch(
            "CREATE TABLE providers (
            id TEXT, name TEXT, app_type TEXT, is_current BOOLEAN,
            sort_index INTEGER, created_at INTEGER);
            INSERT INTO providers VALUES ('one', 'One', 'codex', 1, 1, NULL);
            INSERT INTO providers VALUES ('two', 'Two', 'codex', 0, 2, NULL);",
        )
        .unwrap();
        db.pragma_update(None, "user_version", schema).unwrap();
        std::fs::write(
            app_dir.join("settings.json"),
            r#"{"currentProviderCodex":"one"}"#,
        )
        .unwrap();

        // If the CLI starts a GUI, it must never launch the user's real installation.
        let stub = home.path().join("cc-switch");
        std::fs::write(&stub, "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(&stub, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut bus = Command::new("dbus-daemon")
            .args(["--session", "--nofork", "--print-address=1"])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut address = String::new();
        BufReader::new(bus.stdout.take().unwrap())
            .read_line(&mut address)
            .unwrap();
        Self {
            home,
            bus,
            address: address.trim().to_owned(),
        }
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_cc-switch-cli"))
            .args(args)
            .env("CC_SWITCH_TEST_HOME", self.home.path())
            .env("DBUS_SESSION_BUS_ADDRESS", &self.address)
            .env("PATH", self.home.path())
            .output()
            .unwrap()
    }

    fn serve(&self, fail: bool) -> zbus::blocking::Connection {
        zbus::blocking::connection::Builder::address(self.address.as_str())
            .unwrap()
            .name("org.kde.StatusNotifierWatcher")
            .unwrap()
            .serve_at("/StatusNotifierWatcher", Watcher)
            .unwrap()
            .serve_at("/StatusNotifierItem", Item)
            .unwrap()
            .serve_at(
                "/Menu",
                Menu {
                    app_dir: self.home.path().join(".cc-switch"),
                    selected: Arc::new(Mutex::new(10)),
                    fail,
                },
            )
            .unwrap()
            .build()
            .unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.bus.kill();
        let _ = self.bus.wait();
    }
}

struct Watcher;
#[zbus::interface(name = "org.kde.StatusNotifierWatcher")]
impl Watcher {
    #[zbus(property)]
    fn registered_status_notifier_items(&self) -> Vec<String> {
        vec!["org.kde.StatusNotifierWatcher/StatusNotifierItem".into()]
    }
}

struct Item;
#[zbus::interface(name = "org.kde.StatusNotifierItem")]
impl Item {
    #[zbus(property)]
    fn title(&self) -> String {
        "cc-switch".into()
    }
    #[zbus(property)]
    fn menu(&self) -> OwnedObjectPath {
        OwnedObjectPath::try_from("/Menu").unwrap()
    }
}

struct Menu {
    app_dir: PathBuf,
    selected: Arc<Mutex<i32>>,
    fail: bool,
}

fn node(id: i32, label: &str, checked: Option<bool>, children: Vec<OwnedValue>) -> Layout {
    let mut properties = HashMap::new();
    properties.insert(
        "label".into(),
        OwnedValue::try_from(Value::from(label)).unwrap(),
    );
    properties.insert("enabled".into(), OwnedValue::from(true));
    if let Some(checked) = checked {
        properties.insert("toggle-state".into(), OwnedValue::from(i32::from(checked)));
    }
    (id, properties, children)
}

fn child(layout: Layout) -> OwnedValue {
    OwnedValue::try_from(Value::from(Structure::from(layout))).unwrap()
}

#[zbus::interface(name = "com.canonical.dbusmenu")]
impl Menu {
    fn get_layout(&self, _parent: i32, _depth: i32, _properties: Vec<String>) -> (u32, Layout) {
        let selected = *self.selected.lock().unwrap();
        let codex = node(
            1,
            "Codex · One",
            None,
            vec![
                child(node(10, "One", Some(selected == 10), vec![])),
                child(node(11, "Two", Some(selected == 11), vec![])),
            ],
        );
        (1, node(0, "", None, vec![child(codex)]))
    }

    fn event(
        &self,
        id: i32,
        _event: &str,
        _data: Value<'_>,
        _timestamp: u32,
    ) -> zbus::fdo::Result<()> {
        if self.fail {
            return Err(zbus::fdo::Error::Failed("simulated switch failure".into()));
        }
        assert_eq!(id, 11);
        Connection::open(self.app_dir.join("cc-switch.db"))
            .unwrap()
            .execute("UPDATE providers SET is_current = (id = 'two')", [])
            .unwrap();
        std::fs::write(
            self.app_dir.join("settings.json"),
            r#"{"currentProviderCodex":"two"}"#,
        )
        .unwrap();
        *self.selected.lock().unwrap() = id;
        Ok(())
    }
}

#[test]
fn future_schema_warns_and_switches_through_installed_gui() {
    let fixture = Fixture::new(999);
    let _server = fixture.serve(false);
    let output = fixture.run(&["--json", "codex", "--id", "two"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let body: Json = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(
        (body["backend"].as_str(), body["changed"].as_bool()),
        (Some("dbus"), Some(true))
    );
    assert_eq!(body["warnings"].as_array().unwrap().len(), 1);
    let warning: Json = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(warning["warning"]["kind"], "unverified_schema");
    let db = Connection::open(fixture.home.path().join(".cc-switch/cc-switch.db")).unwrap();
    assert_eq!(
        db.query_row("PRAGMA user_version", [], |r| r.get::<_, i32>(0))
            .unwrap(),
        999
    );
}

#[test]
fn future_schema_operation_failure_is_an_error_despite_warning() {
    let fixture = Fixture::new(21);
    let _server = fixture.serve(true);
    let output = fixture.run(&["--json", "codex", "--id", "two"]);
    assert_eq!(output.status.code(), Some(5));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).unwrap();
    let records: Vec<Json> = stderr
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(records[0]["warning"]["kind"], "unverified_schema");
    assert_eq!(records[1]["error"]["kind"], "gui_error");
}

#[test]
fn unknown_schema_read_and_noop_work_without_gui() {
    let fixture = Fixture::new(19);
    for args in [
        vec!["--json", "codex", "--list"],
        vec!["--json", "codex", "--current"],
        vec!["--json", "codex", "--id", "one"],
    ] {
        let output = fixture.run(&args);
        assert!(output.status.success());
        let body: Json = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(body["warnings"].as_array().unwrap().len(), 1);
    }
    let output = fixture.run(&["codex", "--name", "One"]);
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).starts_with("warning: unverified_schema:"));
    let output = fixture.run(&["codex", "--id", "missing"]);
    assert_eq!(output.status.code(), Some(4));
}
