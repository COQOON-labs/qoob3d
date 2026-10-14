//! QOOB3D – parametric CAD for FDM printing.

use clap::Parser as _;
use qoob3d_app::{app, batch, cli, headless_render, smoke};

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,wgpu_core=warn,wgpu_hal=warn,naga=warn".into()),
        )
        .init();

    let cli = cli::Cli::parse();

    if cli.render.is_some() || cli.export.is_some() || cli.report {
        return headless(&cli);
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
        Box::new(move |cc| {
            let options = app::StartOptions {
                // The smoke test needs a body to orbit around (pivot check).
                demo: cli.demo || smoke.is_some(),
                smoke,
                open: cli.file.clone(),
            };
            Ok(Box::new(app::QoobApp::new(cc, options)?))
        }),
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
                eprintln!("smoke: FAIL  window was closed before the test finished");
                std::process::exit(1);
            }
        }
    }
    Ok(())
}

/// `--render`, `--export`, `--report` without a window (ADR-0013, ADR-0033).
fn headless(cli: &cli::Cli) -> anyhow::Result<()> {
    let mut kernel = qoob3d_kernel::Kernel::spawn(|| {});
    let mut ok = true;
    let mut scene = None;
    if let Some(file) = &cli.file {
        let doc = std::sync::Arc::new(batch::load(file)?);
        let result = batch::rebuild(&mut kernel, &doc)?;
        let report = batch::report(&doc, &result);
        ok = report.ok;
        if cli.report {
            println!("{}", serde_json::to_string_pretty(&report)?);
        }
        for e in &report.errors {
            eprintln!("error in {}: {}", e.name, e.error);
        }
        if let Some(out) = &cli.export {
            let n = batch::export(&mut kernel, &doc, out)?;
            eprintln!("exported {n} bodies to {}", out.display());
        }
        scene = Some(batch::scene(&result));
    }
    if let Some(out) = &cli.render {
        let view = headless_render::RenderView {
            preset: cli.view.into(),
            orthographic: cli.ortho,
            zoom: cli.zoom,
            printer: cli.printer.clone(),
            build_volume: cli.volume,
        };
        headless_render::run(out, view, cli.size, scene)?;
    }
    if !ok {
        std::process::exit(1);
    }
    Ok(())
}
