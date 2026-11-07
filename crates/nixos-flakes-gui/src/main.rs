//! `nixos-flakes-gui` : interface graphique de gestion des flakes.

mod anim;
mod app;
mod cli;
mod demo;
mod keys;
mod model;
mod palette;
mod settings;
mod tasks;
mod theme;
mod toast;
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
        .style(App::style)
        // Inter (SIL OFL 1.1, voir assets/fonts/OFL.txt) : instances statiques réduites au latin.
        .font(include_bytes!("../assets/fonts/Inter-400.ttf").as_slice())
        .font(include_bytes!("../assets/fonts/Inter-500.ttf").as_slice())
        .font(include_bytes!("../assets/fonts/Inter-600.ttf").as_slice())
        .default_font(widgets::INTER)
        .settings(iced::Settings {
            id: Some("nixos-flakes-gui".to_string()),
            antialiasing: true,
            ..iced::Settings::default()
        })
        .run()
}
