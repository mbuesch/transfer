use crate::app::App;
use dioxus::desktop::{Config, WindowBuilder};

#[cfg(not(target_os = "android"))]
use clap::Parser;

mod app;
mod device_name;
mod l10n;
mod pick_file;
mod protocol;

#[cfg(not(target_os = "android"))]
#[derive(Parser)]
#[command(version, about = "LAN file transfer tool")]
struct Args {
    /// Enable IPv4 support.
    ///
    /// By default both --ipv4 and --ipv6 are enabled, unless one of them is explicitly specified.
    /// Specifying --ipv4 or --ipv6 will disable the other protocol.
    #[arg(long, short = '4')]
    #[cfg(feature = "ipv4")]
    ipv4: bool,

    /// Enable IPv6 support.
    ///
    /// By default both --ipv4 and --ipv6 are enabled, unless one of them is explicitly specified.
    /// Specifying --ipv4 or --ipv6 will disable the other protocol.
    #[arg(long, short = '6')]
    #[cfg(feature = "ipv6")]
    ipv6: bool,

    /// Language to use: en, de (default: auto)
    #[arg(long, value_name = "LANG")]
    lang: Option<String>,
}

#[cfg(not(target_os = "android"))]
fn load_window_icon() -> Option<dioxus::desktop::tao::window::Icon> {
    let bytes = include_bytes!("../assets/icon.png");
    let decoder = png::Decoder::new(std::io::Cursor::new(bytes.as_slice()));
    let mut reader = decoder.read_info().ok()?;
    let size = reader.output_buffer_size()?;
    let mut buf = vec![0u8; size];
    let info = reader.next_frame(&mut buf).ok()?;
    let rgba = match info.color_type {
        png::ColorType::Rgba => buf[..info.buffer_size()].to_vec(),
        png::ColorType::Rgb => buf[..info.buffer_size()]
            .chunks(3)
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect(),
        _ => return None,
    };
    dioxus::desktop::tao::window::Icon::from_rgba(rgba, info.width, info.height).ok()
}

fn main() {
    env_logger::init();

    #[cfg(not(target_os = "android"))]
    {
        use crate::protocol::packets::IpSupport;

        let args = Args::parse();

        if args.ipv4 && !args.ipv6 {
            IpSupport::V4.set();
        } else if args.ipv6 && !args.ipv4 {
            IpSupport::V6.set();
        }

        if let Some(lang_str) = args.lang {
            match lang_str.to_lowercase().as_str() {
                "auto" => (),
                "de" => l10n::Language::set_forced(l10n::Language::De),
                "en" => l10n::Language::set_forced(l10n::Language::En),
                _ => l10n::Language::set_forced(l10n::Language::En),
            }
        }
    }

    let window = WindowBuilder::new()
        .with_always_on_top(false)
        .with_title("File Transfer");

    #[cfg(not(target_os = "android"))]
    let window = window.with_window_icon(load_window_icon());

    let config = Config::new().with_window(window).with_menu(None);

    dioxus::LaunchBuilder::desktop()
        .with_cfg(config)
        .launch(App);
}
