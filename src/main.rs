mod cli;
mod dbus;
mod direct;
mod error;
mod process;
mod schema;
mod state;

#[cfg(test)]
#[path = "../tests/support/mod.rs"]
mod test_support;

use clap::Parser;
use cli::{Cli, Command};
use error::{AppError, ErrorOutput};
use serde::Serialize;
use state::{Paths, ProviderSummary, StateReader};
use std::process::ExitCode;
use std::time::{Duration, Instant};

#[derive(Serialize)]
struct ProviderListOutput {
    app: &'static str,
    providers: Vec<ProviderSummary>,
    warnings: Vec<String>,
}

#[derive(Serialize)]
struct CurrentOutput {
    app: &'static str,
    provider: Option<ProviderSummary>,
    warnings: Vec<String>,
}

#[derive(Serialize)]
struct SwitchOutput {
    app: &'static str,
    provider: ProviderSummary,
    backend: &'static str,
    changed: bool,
    warnings: Vec<String>,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(&cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            if cli.json {
                let output = ErrorOutput::from(&error);
                eprintln!(
                    "{}",
                    serde_json::to_string(&output).expect("error output is serializable")
                );
            } else {
                eprintln!("error: {error}");
            }
            error.exit_code()
        }
    }
}

fn run(cli: &Cli) -> Result<(), AppError> {
    let reader = StateReader::open(Paths::discover()?)?;
    for warning in reader.warnings() {
        if cli.json {
            let kind = warning
                .split_once(':')
                .map_or("compatibility_warning", |(kind, _)| kind);
            eprintln!(
                "{}",
                serde_json::json!({"warning": {
                    "kind": kind, "message": warning
                }})
            );
        } else {
            eprintln!("warning: {warning}");
        }
    }
    let gui_running = process::is_gui_running();
    match &cli.command {
        Command::Codex(args) if args.list => print_list(&reader, cli.json),
        Command::Codex(args) if args.current => print_current(&reader, cli.json),
        Command::Codex(args) => {
            let provider = if let Some(name) = args.name.as_deref() {
                reader.by_name(name)?
            } else if let Some(id) = args.id.as_deref() {
                reader.by_id(id)?
            } else {
                unreachable!("clap requires one Codex action")
            };
            switch_provider(&reader, provider, gui_running, cli.json)
        }
    }
}

fn switch_provider(
    reader: &StateReader,
    provider: ProviderSummary,
    gui_running: bool,
    json: bool,
) -> Result<(), AppError> {
    if reader.has_converged_to(&provider.id)? {
        return print_switch(
            SwitchOutput {
                app: "codex",
                provider,
                backend: "noop",
                changed: false,
                warnings: reader.warnings(),
            },
            json,
        );
    }

    let providers = reader.providers()?;
    let use_gui = gui_running || reader.requires_installed_gui();
    let (backend, mut warnings, tray) = if use_gui {
        let tray = if gui_running {
            dbus::TrayClient::connect()?
        } else {
            start_gui_and_connect()?
        };
        reader.ensure_schema_unchanged()?;
        tray.activate(&providers, &provider.id)?;
        ("dbus", Vec::new(), Some(tray))
    } else {
        if process::is_gui_running() {
            return Err(AppError::GuiUnavailable(
                "CC Switch started while preparing the direct switch; retry the command".into(),
            ));
        }
        ("direct", direct::switch(reader, &provider.id)?, None)
    };

    warnings.extend(reader.warnings());

    wait_for_convergence(reader, tray.as_ref(), &providers, &provider.id)?;
    let provider = reader.by_id(&provider.id)?;
    print_switch(
        SwitchOutput {
            app: "codex",
            provider,
            backend,
            changed: true,
            warnings,
        },
        json,
    )
}

fn wait_for_convergence(
    reader: &StateReader,
    tray: Option<&dbus::TrayClient>,
    providers: &[ProviderSummary],
    provider_id: &str,
) -> Result<(), AppError> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let disk_ready = reader.has_converged_to(provider_id)?;
        let tray_ready = match tray {
            Some(tray) => tray.is_selected(providers, provider_id)?,
            None => true,
        };
        if disk_ready && tray_ready {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(AppError::Timeout);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn start_gui_and_connect() -> Result<dbus::TrayClient, AppError> {
    let mut child = process::start_gui()?;
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match dbus::TrayClient::connect() {
            Ok(tray) => {
                // Keep the GUI running to own proxy state and subsequent switches.
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
                return Ok(tray);
            }
            Err(error) if Instant::now() >= deadline => return Err(error),
            Err(_) => {}
        }
        if let Some(status) = child.try_wait().map_err(|error| {
            AppError::GuiUnavailable(format!("cannot check CC Switch startup: {error}"))
        })? {
            return Err(AppError::GuiUnavailable(format!(
                "CC Switch exited before registering its tray interface ({status})"
            )));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn print_switch(output: SwitchOutput, json: bool) -> Result<(), AppError> {
    if json {
        println!(
            "{}",
            serde_json::to_string(&output).map_err(|error| AppError::State(error.to_string()))?
        );
    } else if output.changed {
        println!(
            "Switched Codex to {} ({}) via {}.",
            output.provider.name, output.provider.id, output.backend
        );
        for warning in output.warnings.iter().filter(|warning| {
            !warning.starts_with("unverified_schema:")
                && !warning.starts_with("unverified_settings:")
        }) {
            eprintln!("warning: {warning}");
        }
    } else {
        println!(
            "Codex already uses {} ({}).",
            output.provider.name, output.provider.id
        );
    }
    Ok(())
}

fn print_list(reader: &StateReader, json: bool) -> Result<(), AppError> {
    let providers = reader.providers()?;
    if json {
        println!(
            "{}",
            serde_json::to_string(&ProviderListOutput {
                app: "codex",
                providers,
                warnings: reader.warnings(),
            })
            .map_err(|error| AppError::State(error.to_string()))?
        );
        return Ok(());
    }

    if providers.is_empty() {
        println!("No Codex providers configured.");
        return Ok(());
    }
    for provider in providers {
        let marker = if provider.current { "*" } else { " " };
        println!("{marker} {}\t{}", provider.name, provider.id);
    }
    Ok(())
}

fn print_current(reader: &StateReader, json: bool) -> Result<(), AppError> {
    let provider = reader.current()?;
    if json {
        println!(
            "{}",
            serde_json::to_string(&CurrentOutput {
                app: "codex",
                provider,
                warnings: reader.warnings()
            })
            .map_err(|error| AppError::State(error.to_string()))?
        );
        return Ok(());
    }

    match provider {
        Some(provider) => println!("{}\t{}", provider.name, provider.id),
        None => println!("No current Codex provider."),
    }
    Ok(())
}
