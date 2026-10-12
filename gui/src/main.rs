//! nexus-menu-gui — a window for finding installed applications that have no
//! menu entry and creating one. It runs the same code as the command line
//! (the `nexus-menu` library); this file only connects it to a window.

// Without this a release build would open a console window on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod args;
mod service;
mod updater;

use std::process::ExitCode;

use nexus_menu::engine::{Generated, InstallResult, PrefixInfo, RemoveResult};
use service::{ScanView, Service};
use tauri::State;
use updater::{Http, UpdateService, UpdateView};

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

/// Whether a newer version exists. The page cannot say where to look.
#[tauri::command(async)]
fn check_update(updates: State<'_, UpdateService<Http>>) -> Result<UpdateView, String> {
    updates.check()
}

/// Install the version `check_update` found, then start it and quit. The page
/// supplies nothing: not a URL, not a path.
#[tauri::command(async)]
fn install_update(
    app: tauri::AppHandle,
    updates: State<'_, UpdateService<Http>>,
) -> Result<(), String> {
    let (exe, version) = updates.install()?;
    let args: Vec<String> = std::env::args().skip(1).collect();
    updater::restart(&exe, &args).map_err(|e| {
        format!("Version {version} is installed, but {e}. Start the app again to use it.")
    })?;
    app.exit(0);
    Ok(())
}

fn main() -> ExitCode {
    let gui = match args::parse(std::env::args().skip(1)) {
        Ok(args::Parsed::Run(gui)) => gui,
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
        .manage(Service::new(gui.engine))
        .manage(UpdateService::shipped(gui.update_check))
        .invoke_handler(tauri::generate_handler![
            scan,
            install,
            list_generated,
            remove,
            prefixes,
            check_update,
            install_update
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
