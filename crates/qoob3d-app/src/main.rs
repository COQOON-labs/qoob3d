//! QOOB3D – parametric CAD for FDM printing.

mod app;
mod cli;
mod headless_render;
mod navigation;
mod smoke;

use clap::Parser as _;

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,wgpu_core=warn,wgpu_hal=warn,naga=warn".into()),
        )
        .init();

    let cli = cli::Cli::parse();

    if let Some(out) = &cli.render {
        return headless_render::run(out, cli.view.into(), cli.size);
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
