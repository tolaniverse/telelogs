use anyhow::Result;
use clap::Parser;
use gpui_kit::*;

mod assets;
mod client;
mod json;
mod theme;
mod ui;
mod workspace;

#[derive(Parser)]
#[command(version, about = "Telelogs desktop app")]
struct Args {
    /// telelog-server gRPC endpoint.
    #[arg(long, env = "TELELOG_SERVER", default_value = "http://127.0.0.1:7070")]
    server: String,
}

fn main() -> Result<()> {
    let args = Args::parse();

    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    let handle = runtime.handle().clone();
    // Keep the runtime alive on its own thread for the life of the process.
    std::thread::spawn(move || runtime.block_on(std::future::pending::<()>()));

    gpui_kit::application().with_assets(assets::Assets).run(move |cx| {
        gpui_kit::init(cx);
        theme::load_fonts(cx).expect("bundled fonts are valid");
        theme::apply(true, cx);
        cx.bind_keys([
            KeyBinding::new("secondary-k", workspace::ToggleCommandPalette, None),
            KeyBinding::new("secondary-j", workspace::ToggleJsonView, None),
        ]);

        let options = WindowOptions {
            window_bounds: Some(WindowBounds::centered(size(px(1320.), px(840.)), cx)),
            window_min_size: Some(size(px(960.), px(600.))),
            titlebar: Some(TitlebarOptions {
                title: Some("telelogs".into()),
                appears_transparent: true,
                traffic_light_position: Some(point(px(14.), px(13.))),
            }),
            ..Default::default()
        };
        gpui_kit::open_window(options, cx, |window, cx| {
            let (client, events) = client::spawn(&handle, args.server.clone());
            cx.new(|cx| workspace::Workspace::new(args.server.clone(), client, events, window, cx))
        })
        .expect("failed to open window");
        cx.activate(true);
    });
    Ok(())
}
