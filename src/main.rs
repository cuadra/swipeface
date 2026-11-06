//! An elegant, keyboard-driven font viewer.
//!
//! * `swipeface FILE` shows a specimen of a single font file.
//! * `swipeface` browses every installed font with vim motions.

mod app;
mod catalog;
mod palette;
mod view;

use app::App;

use iced::window;
use std::path::PathBuf;

const USAGE: &str = "\
Usage: swipeface [FILE]

  FILE   A TrueType/OpenType font or collection (.ttf, .otf, .ttc, .otc) to
         view. Without it, every installed font is listed for browsing.

Press ? inside the application for the list of keys.";

fn main() -> iced::Result {
    let file = match std::env::args_os().nth(1) {
        Some(arg) if arg == "-h" || arg == "--help" => {
            println!("{USAGE}");
            return Ok(());
        }
        Some(arg) if arg == "-V" || arg == "--version" => {
            println!("swipeface {}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        Some(arg) => Some(local_path(PathBuf::from(arg))),
        None => None,
    };

    iced::application(move || App::boot(file.clone()), App::update, view::view)
        .title(App::title)
        .theme(App::theme)
        .subscription(App::subscription)
        .window(window::Settings {
            size: iced::Size::new(1280.0, 860.0),
            min_size: Some(iced::Size::new(560.0, 360.0)),
            platform_specific: window::settings::PlatformSpecific {
                application_id: "swipeface".to_owned(),
                ..Default::default()
            },
            ..Default::default()
        })
        .antialiasing(true)
        .run()
}

/// File managers may hand over a `file://` URI instead of a path.
fn local_path(path: PathBuf) -> PathBuf {
    let Some(uri) = path.to_str().and_then(|s| s.strip_prefix("file://")) else {
        return path;
    };

    let bytes = uri.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;

    while index < bytes.len() {
        let escape = (bytes[index] == b'%')
            .then(|| uri.get(index + 1..index + 3))
            .flatten()
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());

        match escape {
            Some(byte) => {
                decoded.push(byte);
                index += 3;
            }
            None => {
                decoded.push(bytes[index]);
                index += 1;
            }
        }
    }

    PathBuf::from(String::from_utf8_lossy(&decoded).into_owned())
}
