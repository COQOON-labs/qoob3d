//! QOOB3D – parametric CAD for FDM printing.

use clap::Parser as _;
use qoob3d_app::{app, cli, project_render, service_cli, smoke};
use qoob3d_project::{client, init, mcp, service, worker};

fn main() -> anyhow::Result<()> {
    // Logs go to stderr: stdout carries the worker protocol and CLI envelopes.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,wgpu_core=warn,wgpu_hal=warn,naga=warn".into()),
        )
        .init();

    let cli = cli::Cli::parse();
    match &cli.command {
        Some(cli::Command::Serve { dir }) => return service::serve(dir),
        Some(cli::Command::Worker) => return worker::run(),
        Some(cli::Command::Bench { dir }) => std::process::exit(service::bench(dir)),
        Some(cli::Command::Init { dir }) => return init::run(dir),
        Some(cli::Command::RenderProject {
            dir,
            out,
            view,
            views,
            size,
            parts,
            source,
            region,
        }) => {
            let one = [*view];
            let views = if views.is_empty() { &one[..] } else { views };
            let views: Vec<_> = views.iter().map(|v| (*v).into()).collect();
            let what = match source {
                Some(file) => project_render::Scene::Script(file.clone()),
                None => project_render::Scene::Parts(parts.clone()),
            };
            let frame = region
                .map(|r| qoob3d_core::Aabb { min: [r[0], r[1], r[2]], max: [r[3], r[4], r[5]] });
            return project_render::run(dir, out, &views, *size, &what, frame);
        }
        Some(cli::Command::Mcp { dir }) => return mcp::run(dir.as_deref()),
        Some(cli::Command::Project(command)) => std::process::exit(service_cli::run(command)),
        None => {}
    }

    // WA-OP-01: a folder that is no project ends the app before any window.
    let project = match cli.project.as_deref().map(client::ProjectDir::open).transpose() {
        Ok(project) => project,
        Err(err) => {
            eprintln!("error: {err:#}");
            std::process::exit(2);
        }
    };

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
                demo: smoke.is_some(),
                smoke,
                project_dir: project.map(|p| p.dir().to_path_buf()),
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
