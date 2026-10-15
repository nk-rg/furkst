//! The fonts each person picks in Preferences, from those installed on the computer. egui's built-in fonts stay
//! behind them as fallbacks, so symbols and emoji still show whatever is chosen.

use std::sync::{Arc, OnceLock};

use egui::{FontData, FontDefinitions, FontFamily};

/// Every font installed on the computer, for all users or just this one. Reading them takes a moment (each font file
/// is opened), so it happens once, the first time a choice needs them.
fn database() -> &'static fontdb::Database {
    static DATABASE: OnceLock<fontdb::Database> = OnceLock::new();
    DATABASE.get_or_init(|| {
        let mut database = fontdb::Database::new();
        database.load_system_fonts();
        database
    })
}

/// The names of the installed font families, sorted, without repeats: all of them, and the monospaced ones.
pub struct Families {
    pub all: Vec<String>,
    pub monospaced: Vec<String>,
}

pub fn families() -> &'static Families {
    static FAMILIES: OnceLock<Families> = OnceLock::new();
    FAMILIES.get_or_init(|| {
        let sorted = |monospaced_only: bool| {
            let mut names: Vec<String> = database()
                .faces()
                .filter(|face| face.monospaced || !monospaced_only)
                .filter_map(|face| face.families.first().map(|(name, _)| name.clone()))
                .collect();
            names.sort_by_key(|name| name.to_lowercase());
            names.dedup();
            names
        };
        Families { all: sorted(false), monospaced: sorted(true) }
    })
}

/// Uses these installed font families from now on, for the interface and for code; `None`, or a font that has since
/// been uninstalled, keeps the built-in one.
pub fn apply(ctx: &egui::Context, interface: Option<&str>, code: Option<&str>) {
    let mut fonts = FontDefinitions::default();
    for (choice, family) in [(interface, FontFamily::Proportional), (code, FontFamily::Monospace)] {
        let Some(data) = choice.and_then(load) else { continue };
        let key = format!("{family:?}: {}", choice.unwrap_or_default());
        fonts.font_data.insert(key.clone(), Arc::new(data));
        fonts.families.entry(family).or_default().insert(0, key);
    }
    ctx.set_fonts(fonts);
}

/// Like [`apply`], without holding up startup: the installed fonts take a moment to read, so the window opens with the
/// built-in fonts and switches once the chosen ones are found.
pub fn apply_in_background(ctx: &egui::Context, interface: Option<String>, code: Option<String>) {
    if interface.is_none() && code.is_none() {
        return;
    }
    let ctx = ctx.clone();
    std::thread::spawn(move || {
        apply(&ctx, interface.as_deref(), code.as_deref());
        ctx.request_repaint();
    });
}

/// The regular face of an installed font family.
fn load(family: &str) -> Option<FontData> {
    let database = database();
    let query = fontdb::Query { families: &[fontdb::Family::Name(family)], ..Default::default() };
    let id = database.query(&query)?;
    let index = database.face(id)?.index;
    let bytes = database.with_face_data(id, |data, _| data.to_vec())?;
    Some(FontData { index, ..FontData::from_owned(bytes) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "reads this computer's fonts"]
    fn reads_installed_fonts() {
        let start = std::time::Instant::now();
        let families = families();
        println!("{} families, {} monospaced, in {:?}", families.all.len(), families.monospaced.len(), start.elapsed());
        println!("monospaced: {}", families.monospaced.join(", "));
        assert!(load("Cascadia Code").is_some());
    }
}
