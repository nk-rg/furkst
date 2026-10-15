#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod avatars;
mod editors;
mod fonts;
mod icon;
mod palette;
mod platform;
mod repositories;
mod preferences;
mod settings;
mod tab;
mod tasks;
mod theme;
mod widgets;

fn main() -> eframe::Result {
    let settings = settings::Settings::load();
    // `furkst <path>…` opens those repositories in addition to the previous session.
    let paths: Vec<std::path::PathBuf> = std::env::args_os().skip(1).map(Into::into).collect();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Furkst")
            .with_inner_size([1280.0, 800.0])
            .with_min_inner_size([800.0, 500.0])
            .with_icon(icon::window_icon()),
        wgpu_options: low_memory_gpu(),
        ..Default::default()
    };
    eframe::run_native("Furkst", options, Box::new(|cc| Ok(Box::new(app::FurkstApp::new(cc, settings, &paths)))))
}

/// egui's default GPU setup, but asking the driver to favor a small memory footprint over raw speed: a 2D UI
/// never needs the large up-front allocations the default reserves (about 250 MB with some drivers).
///
/// On Windows it renders with DirectX 12 rather than letting wgpu pick Vulkan: some Vulkan drivers (Intel's among
/// them) make the whole window flicker while it redraws quickly, as it does on every mouse move over the history.
/// `WGPU_BACKEND` still overrides it.
fn low_memory_gpu() -> eframe::WgpuConfiguration {
    use eframe::egui_wgpu::{WgpuSetup, WgpuSetupCreateNew};
    use eframe::wgpu;

    let mut setup = WgpuSetupCreateNew::without_display_handle();
    if cfg!(windows) {
        setup.instance_descriptor.backends = wgpu::Backends::from_env().unwrap_or(wgpu::Backends::DX12);
    }
    let default_descriptor = std::sync::Arc::clone(&setup.device_descriptor);
    setup.device_descriptor = std::sync::Arc::new(move |adapter| wgpu::DeviceDescriptor {
        memory_hints: wgpu::MemoryHints::MemoryUsage,
        ..default_descriptor(adapter)
    });
    eframe::WgpuConfiguration { wgpu_setup: WgpuSetup::CreateNew(setup), ..eframe::NativeOptions::default().wgpu_options }
}
