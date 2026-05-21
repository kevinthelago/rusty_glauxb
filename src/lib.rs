pub mod app;
pub mod console;
pub mod fonts;
pub mod geo_summary;
pub mod milsym;
pub mod tile;
pub mod directions;
pub mod download;
pub mod diag;
pub mod location;
pub mod culling;
pub mod feeds;
pub mod geo;
pub mod globe;
pub mod hud;
pub mod layers;
pub mod log_capture;
pub mod natural_earth;
pub mod pipeline;
pub mod renderer;
pub mod settings;
pub mod topo;
pub mod voice;
pub mod world_map;

use log_capture::LogBuffer;

fn init_tracing(log_buffer: &LogBuffer) {
    use tracing_subscriber::{filter::LevelFilter, layer::SubscriberExt, util::SubscriberInitExt, Layer};

    let terminal_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));

    tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().with_filter(terminal_filter))
        .with(log_capture::ConsoleLayer::new(log_buffer.clone())
            .with_filter(LevelFilter::DEBUG))
        .init();
}

pub fn run() -> anyhow::Result<()> {
    let log_buffer = log_capture::new_log_buffer();
    init_tracing(&log_buffer);

    let rt = tokio::runtime::Runtime::new()?;
    let _guard = rt.enter();

    let event_loop = winit::event_loop::EventLoop::new()?;
    event_loop.set_control_flow(winit::event_loop::ControlFlow::Poll);

    let mut app = app::App::new(rt.handle().clone(), log_buffer);
    event_loop.run_app(&mut app)?;

    Ok(())
}

#[cfg(target_os = "android")]
#[no_mangle]
fn android_main(android_app: android_activity::AndroidApp) {
    use winit::platform::android::EventLoopBuilderExtAndroid;

    let log_buffer = log_capture::new_log_buffer();
    init_tracing(&log_buffer);

    let rt = tokio::runtime::Runtime::new()
        .expect("tokio runtime creation failed");
    let _guard = rt.enter();

    let event_loop = winit::event_loop::EventLoop::builder()
        .with_android_app(android_app)
        .build()
        .expect("event loop creation failed");
    event_loop.set_control_flow(winit::event_loop::ControlFlow::Poll);

    let mut app = app::App::new(rt.handle().clone(), log_buffer);
    event_loop.run_app(&mut app).expect("app error");
}
