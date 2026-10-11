use anyhow::Result;
use clap::Parser;
use gpui_kit::*;

mod assets;
mod client;
mod config;
mod json;
mod servers;
mod theme;
mod time_range;
mod ui;
mod workspace;

#[derive(Parser)]
#[command(version, about = "Telelogs desktop app")]
struct Args {
    /// telelog-server gRPC endpoint (http:// or https://). Without it, the app opens the server
    /// used last, or http://127.0.0.1:7070.
    #[arg(long, env = "TELELOG_SERVER")]
    server: Option<String>,

    /// Token the server expects (see `telelog-server gen-token`).
    #[arg(long, env = "TELELOG_TOKEN", hide_env_values = true)]
    token: Option<String>,

    /// PEM CA certificate to trust, for an https:// server with a self-signed certificate.
    #[arg(long, env = "TELELOG_CA_CERT")]
    ca_cert: Option<std::path::PathBuf>,
}

const DEFAULT_SERVER: &str = "http://127.0.0.1:7070";

fn main() -> Result<()> {
    let args = Args::parse();
    let servers = servers::Servers::load();
    let token = args.token.filter(|t| !t.is_empty());
    let given = |server: String| {
        servers::Open::unsaved(client::Connection {
            server,
            token: token.clone(),
            ca_cert: args.ca_cert.clone(),
        })
    };
    // A saved server named by its URL opens as itself, unless a token or CA given alongside
    // it says to connect differently.
    let open = match args.server {
        Some(url) => match servers.find_url(&url) {
            Some(saved) if token.is_none() && args.ca_cert.is_none() => saved.open(),
            _ => given(url),
        },
        None => servers
            .initial()
            .map_or_else(|| given(DEFAULT_SERVER.into()), |saved| saved.open()),
    };

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
            cx.new(|cx| workspace::Workspace::new(open, servers, handle, window, cx))
        })
        .expect("failed to open window");
        cx.activate(true);
    });
    Ok(())
}
