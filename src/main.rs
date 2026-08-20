mod cli;
mod dbus;
mod direct;
mod error;
mod process;
mod state;

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
}

#[derive(Serialize)]
struct CurrentOutput {
    app: &'static str,
    provider: Option<ProviderSummary>,
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
                warnings: Vec::new(),
            },
            json,
        );
    }

    let providers = reader.providers()?;
    let (backend, warnings, tray) = if gui_running {
        let tray = dbus::TrayClient::connect()?;
        tray.activate(&providers, &provider.id)?;
        ("dbus", Vec::new(), Some(tray))
    } else {
        if process::is_gui_running() {
            return Err(AppError::GuiUnavailable(
                "CC Switch started while preparing the direct switch; retry the command".into(),
            ));
        }
        ("direct", direct::switch(&provider.id)?, None)
    };

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
            Some(tray) => tray.is_selected(providers, provider_id).unwrap_or(false),
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
        for warning in output.warnings {
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
                providers
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
                provider
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
