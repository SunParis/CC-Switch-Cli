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

mod support;

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
        support::create_schema(&db, schema);
        db.execute_batch(
            "INSERT INTO providers (id, name, app_type, is_current, sort_index) VALUES ('one', 'One', 'codex', 1, 1);
             INSERT INTO providers (id, name, app_type, is_current, sort_index) VALUES ('two', 'Two', 'codex', 0, 2);",
        )
        .unwrap();
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

    fn add_future_settings(&self) -> Json {
        let settings = serde_json::json!({
            "currentProviderCodex": "one",
            "localMigrations": {"futureSecret": "private-test-value"}
        });
        std::fs::write(
            self.home.path().join(".cc-switch/settings.json"),
            serde_json::to_vec(&settings).unwrap(),
        )
        .unwrap();
        settings
    }

    fn settings(&self) -> Json {
        serde_json::from_slice(
            &std::fs::read(self.home.path().join(".cc-switch/settings.json")).unwrap(),
        )
        .unwrap()
    }

    fn serve(&self, fail: bool) -> MockGui {
        self.serve_layout(fail, MenuStyle::Modern)
    }

    fn serve_layout(&self, fail: bool, style: MenuStyle) -> MockGui {
        // The process detector must see a GUI even on headless CI. This harmless
        // sleep process has the expected executable name; all IPC stays on our bus.
        let mock_dir = self.home.path().join("mock");
        std::fs::create_dir(&mock_dir).unwrap();
        let mock = mock_dir.join("cc-switch");
        std::fs::copy("/bin/sleep", &mock).unwrap();
        let mut attempts = 0;
        let process = loop {
            match Command::new(&mock).arg("300").spawn() {
                Ok(process) => break process,
                // Concurrent test subprocesses can briefly inherit a copy's open
                // file descriptor before exec closes it (Linux ETXTBSY).
                Err(error) if error.raw_os_error() == Some(26) && attempts < 20 => {
                    attempts += 1;
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                Err(error) => panic!("cannot start mock GUI: {error}"),
            }
        };
        let server = zbus::blocking::connection::Builder::address(self.address.as_str())
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
                    style,
                },
            )
            .unwrap()
            .build()
            .unwrap();
        MockGui {
            _server: server,
            process,
        }
    }
}

struct MockGui {
    _server: zbus::blocking::Connection,
    process: Child,
}

impl Drop for MockGui {
    fn drop(&mut self) {
        let _ = self.process.kill();
        let _ = self.process.wait();
    }
}

#[derive(Clone, Copy)]
enum MenuStyle {
    Flat,
    Submenu,
    Modern,
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
    style: MenuStyle,
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
        let children = vec![
            child(node(10, "One", Some(selected == 10), vec![])),
            child(node(11, "Two", Some(selected == 11), vec![])),
        ];
        if matches!(self.style, MenuStyle::Flat) {
            let mut heading = node(1, "─── Codex ───", None, vec![]);
            heading.1.insert("enabled".into(), OwnedValue::from(false));
            let mut next = node(2, "─── Gemini ───", None, vec![]);
            next.1.insert("enabled".into(), OwnedValue::from(false));
            let mut flat = vec![child(heading)];
            flat.extend(children);
            flat.push(child(next));
            flat.push(child(node(99, "Two", Some(false), vec![])));
            return (1, node(0, "", None, flat));
        }
        let codex = node(
            1,
            if matches!(self.style, MenuStyle::Submenu) {
                "Codex"
            } else {
                "Codex · One"
            },
            None,
            children,
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
        let settings_path = self.app_dir.join("settings.json");
        let mut settings: Json =
            serde_json::from_slice(&std::fs::read(&settings_path).unwrap()).unwrap();
        // Earliest schema-1 releases did not write a local current-provider key.
        if settings.get("currentProviderCodex").is_some() {
            settings["currentProviderCodex"] = Json::from("two");
            std::fs::write(settings_path, serde_json::to_vec(&settings).unwrap()).unwrap();
        }
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
    let fixture = Fixture::new(24);
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
    let fixture = Fixture::new(24);
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

#[test]
fn new_settings_on_known_schema_warn_and_switch_through_gui() {
    let fixture = Fixture::new(20);
    let mut expected = fixture.add_future_settings();
    let _server = fixture.serve(false);
    let output = fixture.run(&["--json", "codex", "--id", "two"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let body: Json = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(body["backend"], "dbus");
    assert_eq!(body["changed"], true);
    assert_eq!(body["warnings"].as_array().unwrap().len(), 1);
    let warning: Json = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(warning["warning"]["kind"], "unverified_settings");
    for bytes in [&output.stdout, &output.stderr] {
        let text = String::from_utf8_lossy(bytes);
        assert!(!text.contains("futureSecret"));
        assert!(!text.contains("private-test-value"));
    }
    expected["currentProviderCodex"] = Json::from("two");
    assert_eq!(fixture.settings(), expected);
}

#[test]
fn new_settings_read_and_noop_work_without_gui() {
    let fixture = Fixture::new(20);
    let expected = fixture.add_future_settings();
    for args in [
        vec!["--json", "codex", "--list"],
        vec!["--json", "codex", "--current"],
        vec!["--json", "codex", "--id", "one"],
    ] {
        let output = fixture.run(&args);
        assert!(output.status.success());
        let body: Json = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(body["warnings"].as_array().unwrap().len(), 1);
        let warning: Json = serde_json::from_slice(&output.stderr).unwrap();
        assert_eq!(warning["warning"]["kind"], "unverified_settings");
    }
    let output = fixture.run(&["codex", "--name", "One"]);
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).starts_with("warning: unverified_settings:"));
    assert_eq!(fixture.settings(), expected);
}

#[test]
fn new_settings_gui_failure_is_an_error_without_direct_fallback() {
    let fixture = Fixture::new(20);
    let expected = fixture.add_future_settings();
    let _server = fixture.serve(true);
    let output = fixture.run(&["--json", "codex", "--id", "two"]);
    assert_eq!(output.status.code(), Some(5));
    assert!(output.stdout.is_empty());
    let stderr = String::from_utf8(output.stderr).unwrap();
    let records: Vec<Json> = stderr
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(records.len(), 2);
    assert_eq!(records[0]["warning"]["kind"], "unverified_settings");
    assert_eq!(records[1]["error"]["kind"], "gui_error");
    assert_eq!(fixture.settings(), expected);
    let db = Connection::open(fixture.home.path().join(".cc-switch/cc-switch.db")).unwrap();
    let current: String = db
        .query_row("SELECT id FROM providers WHERE is_current = 1", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(current, "one");
}

#[test]
fn every_schema_lists_reads_and_noops_without_modifying_files() {
    for schema in 1..=23 {
        let fixture = Fixture::new(schema);
        let db_path = fixture.home.path().join(".cc-switch/cc-switch.db");
        let settings_before = fixture.settings();
        let database_before = std::fs::read(&db_path).unwrap();
        for action in [vec!["--list"], vec!["--current"], vec!["--id", "one"]] {
            let mut args = vec!["--json", "codex"];
            args.extend(action);
            let output = fixture.run(&args);
            assert!(
                output.status.success(),
                "schema {schema}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(output.stderr.is_empty(), "schema {schema}");
            let body: Json = serde_json::from_slice(&output.stdout).unwrap();
            assert_eq!(body["warnings"], serde_json::json!([]));
            if args.contains(&"--id") {
                assert_eq!(body["backend"], "noop");
                assert_eq!(body["changed"], false);
            }
        }
        assert_eq!(fixture.settings(), settings_before);
        assert_eq!(std::fs::read(db_path).unwrap(), database_before);
    }
}

#[test]
fn every_schema_switches_through_its_gui_menu_and_preserves_schema() {
    for schema in 1..=23 {
        let fixture = Fixture::new(schema);
        let style = match schema {
            1..=5 => MenuStyle::Flat,
            6..=18 => MenuStyle::Submenu,
            _ => MenuStyle::Modern,
        };
        let _gui = fixture.serve_layout(false, style);
        let output = fixture.run(&["--json", "codex", "--name", "Two"]);
        assert!(
            output.status.success(),
            "schema {schema}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.stderr.is_empty());
        let body: Json = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(body["backend"], "dbus");
        assert_eq!(body["changed"], true);
        assert_eq!(fixture.settings()["currentProviderCodex"], "two");
        let db = Connection::open(fixture.home.path().join(".cc-switch/cc-switch.db")).unwrap();
        assert_eq!(
            db.query_row("PRAGMA user_version", [], |r| r.get::<_, i32>(0))
                .unwrap(),
            schema
        );
        assert_eq!(
            db.query_row(
                "SELECT value FROM settings WHERE key = 'unrelated-sentinel'",
                [],
                |r| r.get::<_, String>(0)
            )
            .unwrap(),
            "preserve-me"
        );
    }
}

#[test]
fn earliest_schema1_switches_with_database_only_current_state() {
    let fixture = Fixture::new(1);
    let settings = r#"{"showInTray":true}"#;
    let path = fixture.home.path().join(".cc-switch/settings.json");
    std::fs::write(&path, settings).unwrap();
    let _gui = fixture.serve_layout(false, MenuStyle::Flat);
    let output = fixture.run(&["--json", "codex", "--id", "two"]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(std::fs::read_to_string(path).unwrap(), settings);
    let output = fixture.run(&["--json", "codex", "--id", "two"]);
    assert!(output.status.success());
    let body: Json = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(body["backend"], "noop");
}

#[test]
fn reviewed_gui_backed_schema_still_reports_operation_failure() {
    for schema in [1, 6, 15, 19, 21, 22, 23] {
        let fixture = Fixture::new(schema);
        let _gui = fixture.serve(true);
        let output = fixture.run(&["--json", "codex", "--id", "two"]);
        assert_eq!(output.status.code(), Some(5), "schema {schema}");
        assert!(output.stdout.is_empty());
        let error: Json = serde_json::from_slice(&output.stderr).unwrap();
        assert_eq!(error["error"]["kind"], "gui_error");
        assert_eq!(fixture.settings()["currentProviderCodex"], "one");
    }
}
