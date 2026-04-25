use std::sync::Arc;

use anyhow::{Context, Result, anyhow};
use eframe::egui::{self, FontData, FontDefinitions, FontFamily};
use fontdb::{Database, Family, Query};

const PROPORTIONAL_FONT_NAME: &str = "system-proportional";
const MONOSPACE_FONT_NAME: &str = "system-monospace";

pub fn install(ctx: &egui::Context) -> Result<()> {
    let mut db = Database::new();
    db.load_system_fonts();

    let proportional =
        find_font_data(&db, proportional_candidates()).context("find system proportional font")?;
    let monospace = find_font_data(&db, monospace_candidates()).context("find system mono font")?;

    let mut fonts = FontDefinitions::default();
    insert_primary_font(
        &mut fonts,
        PROPORTIONAL_FONT_NAME,
        proportional,
        FontFamily::Proportional,
    );
    insert_primary_font(
        &mut fonts,
        MONOSPACE_FONT_NAME,
        monospace,
        FontFamily::Monospace,
    );

    ctx.set_fonts(fonts);
    Ok(())
}

fn insert_primary_font(
    fonts: &mut FontDefinitions,
    name: &str,
    font: FontData,
    family: FontFamily,
) {
    fonts.font_data.insert(name.to_owned(), Arc::new(font));
    fonts
        .families
        .entry(family)
        .or_default()
        .insert(0, name.to_owned());
}

fn find_font_data(db: &Database, candidates: &'static [Family<'static>]) -> Result<FontData> {
    let query = Query {
        families: candidates,
        ..Default::default()
    };
    let id = db.query(&query).context("query font database")?;
    db.with_face_data(id, |data, index| {
        let mut font = FontData::from_owned(data.to_vec());
        font.index = index;
        font
    })
    .ok_or_else(|| anyhow!("read matched font data"))
}

#[cfg(target_os = "windows")]
fn proportional_candidates() -> &'static [Family<'static>] {
    &[
        Family::Name("Segoe UI Variable Text"),
        Family::Name("Segoe UI"),
        Family::SansSerif,
    ]
}

#[cfg(target_os = "macos")]
fn proportional_candidates() -> &'static [Family<'static>] {
    &[
        Family::Name(".AppleSystemUIFont"),
        Family::Name("SF Pro Text"),
        Family::Name("Helvetica Neue"),
        Family::SansSerif,
    ]
}

#[cfg(all(unix, not(target_os = "macos")))]
fn proportional_candidates() -> &'static [Family<'static>] {
    &[
        Family::Name("Cantarell"),
        Family::Name("Ubuntu"),
        Family::Name("Noto Sans"),
        Family::Name("DejaVu Sans"),
        Family::SansSerif,
    ]
}

#[cfg(not(any(target_os = "windows", target_os = "macos", unix)))]
fn proportional_candidates() -> &'static [Family<'static>] {
    &[Family::SansSerif]
}

#[cfg(target_os = "windows")]
fn monospace_candidates() -> &'static [Family<'static>] {
    &[
        Family::Name("Cascadia Mono"),
        Family::Name("Consolas"),
        Family::Name("Courier New"),
        Family::Monospace,
    ]
}

#[cfg(target_os = "macos")]
fn monospace_candidates() -> &'static [Family<'static>] {
    &[
        Family::Name("SF Mono"),
        Family::Name("Menlo"),
        Family::Name("Monaco"),
        Family::Monospace,
    ]
}

#[cfg(all(unix, not(target_os = "macos")))]
fn monospace_candidates() -> &'static [Family<'static>] {
    &[
        Family::Name("Ubuntu Mono"),
        Family::Name("Noto Sans Mono"),
        Family::Name("DejaVu Sans Mono"),
        Family::Monospace,
    ]
}

#[cfg(not(any(target_os = "windows", target_os = "macos", unix)))]
fn monospace_candidates() -> &'static [Family<'static>] {
    &[Family::Monospace]
}
