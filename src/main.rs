#![cfg_attr(windows, windows_subsystem = "windows")]

use std::sync::Arc;

use eframe::egui;
use s3_viewer::app::App;

/// The window icon: a teal bucket, drawn at start-up so no image files are needed.
fn icon() -> egui::IconData {
    const SIZE: usize = 64;
    let mut rgba = vec![0u8; SIZE * SIZE * 4];
    let teal = [15u8, 118, 110, 255];
    let light = [204u8, 251, 241, 255];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
            // Rounded square background.
            let corner = 12.0f32;
            let dx = (corner - fx).max(fx - (SIZE as f32 - corner)).max(0.0);
            let dy = (corner - fy).max(fy - (SIZE as f32 - corner)).max(0.0);
            if dx * dx + dy * dy > corner * corner {
                continue;
            }
            let mut color = teal;
            // Bucket: a trapezoid with a rim.
            let top = 18.0;
            let bottom = 50.0;
            if (top..=bottom).contains(&fy) {
                let t = (fy - top) / (bottom - top);
                let half = 18.0 - 6.0 * t;
                if (fx - 32.0).abs() <= half {
                    color = light;
                }
            }
            if (14.0..=19.0).contains(&fy) && (fx - 32.0).abs() <= 20.0 {
                color = [255, 255, 255, 255];
            }
            rgba[(y * SIZE + x) * 4..][..4].copy_from_slice(&color);
        }
    }
    egui::IconData { rgba, width: SIZE as u32, height: SIZE as u32 }
}

fn options(renderer: eframe::Renderer) -> eframe::NativeOptions {
    eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("S3 Viewer")
            .with_app_id("s3-viewer")
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([980.0, 640.0])
            .with_icon(Arc::new(icon())),
        renderer,
        ..Default::default()
    }
}

fn run(renderer: eframe::Renderer) -> eframe::Result {
    eframe::run_native("s3viewer", options(renderer), Box::new(|cc| Ok(Box::new(App::new(cc)))))
}

fn main() -> eframe::Result {
    // S3VIEWER_RENDERER=glow forces OpenGL, for machines where wgpu misbehaves.
    let wanted = std::env::var("S3VIEWER_RENDERER").unwrap_or_default().to_lowercase();
    if wanted == "glow" || wanted == "opengl" {
        return run(eframe::Renderer::Glow);
    }
    let Err(error) = run(eframe::Renderer::Wgpu) else { return Ok(()) };
    // winit allows only one event loop per process, so try OpenGL in a fresh one.
    eprintln!("wgpu renderer failed ({error}); restarting with OpenGL");
    let restarted = std::env::current_exe().and_then(|exe| {
        std::process::Command::new(exe).args(std::env::args_os().skip(1)).env("S3VIEWER_RENDERER", "glow").status()
    });
    match restarted {
        Ok(status) => std::process::exit(status.code().unwrap_or(1)),
        Err(_) => Err(error),
    }
}
