use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::path::Path;
use std::time::Duration;

use zdc_dev::{build_once, compile, Assets, Options, Settings, Site, StartupError, DEFAULT_PORT};

#[test]
fn development_options_default_to_loopback_and_the_documented_port() {
    let options = Options::new("examples/hello.zd");

    assert_eq!(options.file, Path::new("examples/hello.zd"));
    assert_eq!(options.host, IpAddr::V4(Ipv4Addr::LOCALHOST));
    assert_eq!(options.port, DEFAULT_PORT);
    assert_eq!(options.addr(), SocketAddr::from(([127, 0, 0, 1], 4321)));
    assert!(options.poll > Duration::ZERO);
}

#[test]
fn development_address_tracks_host_and_port_overrides() {
    let mut options = Options::new("app.zd");
    options.host = IpAddr::V6(Ipv6Addr::LOCALHOST);
    options.port = 9876;

    assert_eq!(options.addr(), SocketAddr::new(options.host, 9876));
}

#[test]
fn unreadable_startup_errors_name_the_file_and_recovery() {
    let error = StartupError::Unreadable {
        path: "missing/app.zd".into(),
        source: io::Error::new(io::ErrorKind::NotFound, "not found"),
    };
    let report = error.report();

    assert!(report.contains("Could not read missing/app.zd"));
    assert!(report.contains("not found"));
    assert!(report.contains("takes the path to a `.zd` file"));
    assert_eq!(error.to_string(), report.trim_end());
}

#[test]
fn bind_startup_errors_name_the_address_and_port_flag() {
    let addr = SocketAddr::from(([127, 0, 0, 1], 4321));
    let error = StartupError::Bind {
        addr,
        source: io::Error::new(io::ErrorKind::AddrInUse, "address in use"),
    };
    let report = error.report();

    assert!(report.contains("Could not listen on 127.0.0.1:4321"));
    assert!(report.contains("address in use"));
    assert!(report.contains("--port"));
    assert_eq!(error.to_string(), report.trim_end());
}

#[test]
fn assets_are_sorted_replaceable_and_keep_their_content_type() {
    let mut assets = Assets::default();
    assets.insert("/z.txt", "old");
    assets.insert("/a.js", "export {};");
    assets.insert("/z.txt", "new");

    assert_eq!(assets.paths().collect::<Vec<_>>(), ["/a.js", "/z.txt"]);
    assert_eq!(assets.get("/z.txt").unwrap().body, b"new");
    assert_eq!(
        assets.get("/a.js").unwrap().content_type,
        "text/javascript; charset=utf-8"
    );
}

#[test]
fn asset_lookup_normalizes_queries_fragments_and_route_directories() {
    let mut assets = Assets::default();
    assets.insert("/index.html", "home");
    assets.insert("/blog/post/index.html", "post");
    assets.insert("/404/index.html", "missing");

    assert_eq!(assets.get("/?cache=1#top").unwrap().body, b"home");
    assert_eq!(assets.get("/blog/post/?cache=1").unwrap().body, b"post");
    assert_eq!(assets.not_found().unwrap().body, b"missing");
}

#[test]
fn unknown_asset_extensions_are_served_as_downloadable_bytes() {
    let mut assets = Assets::default();
    assets.insert("/archive.custom", [0_u8, 1, 2]);

    let asset = assets.get("/archive.custom").unwrap();
    assert_eq!(asset.content_type, "application/octet-stream");
    assert_eq!(asset.body, [0, 1, 2]);
}

#[test]
fn site_accessors_expose_exactly_one_ready_or_broken_surface() {
    let broken = Site::Broken {
        source_path: "bad.zd".into(),
        report: "compile failed".into(),
    };

    assert!(!broken.is_ready());
    assert_eq!(broken.assets(), None);
    assert_eq!(broken.report(), Some("compile failed"));
}

#[test]
fn build_once_is_the_same_pipeline_as_compile() {
    let file = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/hello.zd");
    let settings = Settings::default();

    assert_eq!(build_once(&file, &settings), compile(&file, &settings));
}
