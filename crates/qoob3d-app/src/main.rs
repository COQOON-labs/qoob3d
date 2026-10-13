//! QOOB3D – parametric CAD for FDM printing.

use clap::Parser as _;
use qoob3d_app::{app, cli, headless_render, smoke};

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,wgpu_core=warn,wgpu_hal=warn,naga=warn".into()),
        )
        .init();

    let cli = cli::Cli::parse();

    if let Some(out) = &cli.render {
        let view = headless_render::RenderView {
            preset: cli.view.into(),
            orthographic: cli.ortho,
            zoom: cli.zoom,
            printer: cli.printer.clone(),
            build_volume: cli.volume,
        };
        return headless_render::run(out, view, cli.size);
    }

    let (smoke, smoke_result) = if cli.smoke_test {
        let (test, result) = smoke::SmokeTest::start();
        (Some(test), Some(result))
    } else {
        (None, None)
    };

    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("QOOB3D")
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([800.0, 500.0]),
        renderer: eframe::Renderer::Wgpu,
        ..Default::default()
    };
    eframe::run_native(
        "QOOB3D",
        options,
        Box::new(move |cc| Ok(Box::new(app::QoobApp::new(cc, smoke)?))),
    )
    .map_err(|e| anyhow::anyhow!("{e}"))?;

    if let Some(result) = smoke_result {
        let outcome = result.lock().expect("smoke result lock poisoned").take();
        match outcome {
            Some(Ok(())) => println!("smoke: PASS"),
            Some(Err(msg)) => {
                eprintln!("smoke: FAIL  {msg}");
                std::process::exit(1);
            }
            None => {
                eprintln!("smoke: FAIL  Fenster wurde geschlossen, bevor der Test fertig war");
                std::process::exit(1);
            }
        }
    }
    Ok(())
}
