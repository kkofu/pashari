// Prevents a console window from flashing briefly when the exe is
// launched directly in a release build. Debug builds (e.g. `cargo run`)
// still show println!/eprintln! as before.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod capture;
mod editor;
mod export;
mod hotkey;
mod localkey;
mod overlay;
mod session;
mod settings;
mod shell;
mod single_instance;
mod startup;
mod store;
mod ui;
mod update;
mod upload;

use winit::event_loop::{ControlFlow, EventLoop};

fn main() {
    store::init();

    // The `editor <png>` subcommand: opens the annotation editor as a
    // separate process (multiple instances allowed).
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() == Some("editor") {
        editor::run_standalone(args.next());
        return;
    }

    // Prevents duplicate launches of the resident app. If already
    // running, sends it a signal to show Settings and exits immediately.
    if single_instance::acquire_or_notify() {
        println!("pashari は既に起動中です（Settings を表示します）");
        return;
    }

    // The resident app: a single event loop drives hotkey waiting and the capture flow.
    let event_loop = match EventLoop::<app::UserEvent>::with_user_event().build() {
        Ok(el) => el,
        Err(e) => {
            eprintln!("EventLoop の生成に失敗: {e}");
            std::process::exit(1);
        }
    };
    event_loop.set_control_flow(ControlFlow::Wait);

    // Waits for another process's duplicate-launch signal and requests showing Settings.
    let proxy = event_loop.create_proxy();
    single_instance::watch({
        let proxy = proxy.clone();
        move || {
            let _ = proxy.send_event(app::UserEvent::ShowSettings);
        }
    });

    // The update-check background thread also returns its result via
    // this same proxy (App holds it and clones it when needed).
    let mut app = app::App::new(proxy);
    if let Err(e) = event_loop.run_app(&mut app) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}
