//! nexus-menu-gui — a window for finding installed applications that have no
//! menu entry and creating one. It runs the same code as the command line
//! (the `nexus-menu` library); this file only connects it to a window.

// Without this a release build would open a console window on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod args;
mod service;

use std::process::ExitCode;

use nexus_menu::engine::{Generated, InstallResult, PrefixInfo, RemoveResult};
use service::{ScanView, Service};
use tauri::State;

// Commands without `async` run on the main thread; `(async)` runs a plain
// function on a worker thread instead, which a scan (a few seconds) needs so
// the window does not freeze.

#[tauri::command(async)]
fn scan(service: State<'_, Service>) -> Result<ScanView, String> {
    service.scan()
}

#[tauri::command(async)]
fn install(
    service: State<'_, Service>,
    ids: Vec<String>,
    force: bool,
    dry_run: bool,
) -> Result<Vec<InstallResult>, String> {
    service.install(&ids, force, dry_run)
}

#[tauri::command(async)]
fn list_generated(service: State<'_, Service>) -> Vec<Generated> {
    service.list_generated()
}

#[tauri::command(async)]
fn remove(service: State<'_, Service>, ids: Vec<String>) -> Vec<RemoveResult> {
    service.remove(&ids)
}

#[tauri::command(async)]
fn prefixes(service: State<'_, Service>) -> Vec<PrefixInfo> {
    service.prefixes()
}

fn main() -> ExitCode {
    let options = match args::parse(std::env::args().skip(1)) {
        Ok(args::Parsed::Run(options)) => options,
        Ok(args::Parsed::Version) => {
            println!("nexus-menu-gui {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        Ok(args::Parsed::Help) => {
            print!("{}", args::USAGE);
            return ExitCode::SUCCESS;
        }
        Err(message) => {
            eprintln!("error: {message}\n\n{}", args::USAGE);
            return ExitCode::from(2);
        }
    };

    let result = tauri::Builder::default()
        .manage(Service::new(options))
        .invoke_handler(tauri::generate_handler![
            scan,
            install,
            list_generated,
            remove,
            prefixes
        ])
        .run(tauri::generate_context!());

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::FAILURE
        }
    }
}
