//! `nixos-flakes-gui` : interface graphique de gestion des flakes.

mod app;
mod cli;
mod model;
mod settings;
mod tasks;
mod theme;
mod tray;
mod view;
mod widgets;

use app::App;

fn main() -> iced::Result {
    let cli = match cli::parse(std::env::args().skip(1)) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("nixos-flakes-gui : {e}\n\n{}", cli::USAGE);
            std::process::exit(2);
        }
    };
    if cli.help {
        println!("{}", cli::USAGE);
        return Ok(());
    }
    if cli.version {
        println!("nixos-flakes-gui {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }

    iced::daemon(move || app::boot(cli.clone()), App::update, App::view)
        .title(App::title)
        .subscription(App::subscription)
        .theme(App::theme)
        .settings(iced::Settings {
            id: Some("nixos-flakes-gui".to_string()),
            antialiasing: true,
            ..iced::Settings::default()
        })
        .run()
}
