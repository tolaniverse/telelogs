use anyhow::Result;
use clap::Parser;
use gpui_kit::*;

mod client;
mod log_view;
mod time;

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

    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx| {
            gpui_kit::init(cx);

            let options = WindowOptions {
                window_bounds: Some(WindowBounds::centered(size(px(1200.), px(760.)), cx)),
                titlebar: Some(TitlebarOptions {
                    title: Some("Telelogs".into()),
                    ..Default::default()
                }),
                ..Default::default()
            };
            gpui_kit::open_window(options, cx, |window, cx| {
                let events = client::spawn_tail(&handle, args.server.clone());
                cx.new(|cx| log_view::LogView::new(args.server.clone(), events, window, cx))
            })
            .expect("failed to open window");
            cx.activate(true);
        });
    Ok(())
}
