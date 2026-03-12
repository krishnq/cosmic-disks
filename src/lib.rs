// SPDX-License-Identifier: GPL-3.0

pub mod actions;
pub mod app;
pub mod util;
pub mod cli;
pub mod config;
pub mod handlers;
pub mod i18n;
pub mod message;
pub mod ui;

pub fn run() -> cosmic::iced::Result {
    env_logger::init();

    if std::env::args_os().len() > 1 {
        let cli = <cli::Cli as clap::Parser>::parse();
        let rt = tokio::runtime::Runtime::new().expect("failed to build tokio runtime");
        if let Err(e) = rt.block_on(cli::run(cli)) {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
        return Ok(());
    }

    let requested_languages = i18n_embed::DesktopLanguageRequester::requested_languages();
    i18n::init(&requested_languages);

    let settings = cosmic::app::Settings::default().size_limits(
        cosmic::iced::Limits::NONE
            .min_width(600.0)
            .min_height(400.0),
    );

    cosmic::app::run::<app::AppModel>(settings, ())
}
