//! Styles from TOML files. Built-in styles live in `themes/`, user styles in
//! `~/.s3-viewer/themes/`. A style may `extends` another and override parts.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use eframe::egui::{self, Color32, CornerRadius, FontFamily, FontId, Stroke, TextStyle, Vec2};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

const BUILTIN: &[(&str, &str)] = &[
    ("light", include_str!("../themes/light.toml")),
    ("dark", include_str!("../themes/dark.toml")),
    ("nord", include_str!("../themes/nord.toml")),
    ("high-contrast", include_str!("../themes/high-contrast.toml")),
];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color(pub Color32);

impl<'de> Deserialize<'de> for Color {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        parse_color(&text)
            .map(Color)
            .ok_or_else(|| serde::de::Error::custom(format!("“{text}” is not a #rrggbb colour")))
    }
}

impl Serialize for Color {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let [r, g, b, a] = self.0.to_srgba_unmultiplied();
        let text = if a == 255 { format!("#{r:02x}{g:02x}{b:02x}") } else { format!("#{r:02x}{g:02x}{b:02x}{a:02x}") };
        serializer.serialize_str(&text)
    }
}

pub fn parse_color(text: &str) -> Option<Color32> {
    let hex = text.trim().strip_prefix('#')?;
    let value = |i: usize| u8::from_str_radix(hex.get(i..i + 2)?, 16).ok();
    match hex.len() {
        6 => Some(Color32::from_rgb(value(0)?, value(2)?, value(4)?)),
        8 => Some(Color32::from_rgba_unmultiplied(value(0)?, value(2)?, value(4)?, value(6)?)),
        _ => None,
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Colors {
    pub background: Color,
    pub card: Color,
    pub text: Color,
    pub muted: Color,
    pub line: Color,
    pub input: Color,
    pub hover: Color,
    pub section: Color,
    pub accent: Color,
    pub accent_text: Color,
    pub selection: Color,
    pub selection_text: Color,
    pub header: Color,
    pub header_text: Color,
    pub stripe: Color,
    pub crumb: Color,
    pub crumb_current: Color,
    pub disabled: Color,
    pub banner_bg: Color,
    pub banner_text: Color,
    pub banner_line: Color,
    pub notice: Color,
    pub badge_bg: Color,
    pub badge_text: Color,
    pub code_bg: Color,
    pub code_text: Color,
    pub gutter_bg: Color,
    pub gutter_text: Color,
    pub image_bg: Color,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Syntax {
    pub key: Color,
    pub string: Color,
    pub number: Color,
    pub keyword: Color,
    pub comment: Color,
    pub tag: Color,
    pub attribute: Color,
    pub punct: Color,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Shape {
    pub radius: f32,
    pub card_radius: f32,
    pub spacing: f32,
    pub font_size: f32,
    pub heading_size: f32,
    pub mono_size: f32,
    pub row_height: f32,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Fonts {
    #[serde(default)]
    pub proportional: String,
    #[serde(default)]
    pub monospace: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Theme {
    pub name: String,
    pub dark: bool,
    pub colors: Colors,
    pub syntax: Syntax,
    pub shape: Shape,
    #[serde(default)]
    pub fonts: Fonts,
}

impl Theme {
    pub fn builtin(id: &str) -> Theme {
        resolve(id, None, 0).unwrap_or_else(|error| panic!("built-in style {id} is broken: {error}"))
    }

    pub fn token(&self, token: crate::preview::highlight::Token) -> Color32 {
        use crate::preview::highlight::Token;
        let syntax = &self.syntax;
        match token {
            Token::Key => syntax.key.0,
            Token::String => syntax.string.0,
            Token::Number => syntax.number.0,
            Token::Keyword => syntax.keyword.0,
            Token::Comment => syntax.comment.0,
            Token::Tag => syntax.tag.0,
            Token::Attribute => syntax.attribute.0,
            Token::Punct => syntax.punct.0,
        }
    }

    /// Full TOML of this style, as written by "save as template".
    pub fn to_toml(&self) -> String {
        let body = toml::to_string_pretty(self).unwrap_or_default();
        format!(
            "# S3 Viewer style. Edit and save; the viewer reloads it right away.\n\
             # Remove keys you do not need and add `extends = \"dark\"` (or another style)\n\
             # to inherit them instead. See themes/light.toml in the sources for notes on each key.\n\n{body}"
        )
    }
}

fn builtin_source(id: &str) -> Option<&'static str> {
    BUILTIN.iter().find(|(name, _)| *name == id).map(|(_, text)| *text)
}

fn merge(base: &mut toml::Table, overlay: toml::Table) {
    for (key, value) in overlay {
        match (base.get_mut(&key), value) {
            (Some(toml::Value::Table(target)), toml::Value::Table(source)) => merge(target, source),
            (_, value) => {
                base.insert(key, value);
            }
        }
    }
}

/// Loads a style by id: a user file in `dir` first, then the built-ins.
fn resolve_table(id: &str, dir: Option<&Path>, depth: usize, skip_user: bool) -> Result<toml::Table, String> {
    if depth > 8 {
        return Err("styles extend each other in a loop".into());
    }
    let user = dir.map(|dir| dir.join(format!("{id}.toml"))).filter(|path| !skip_user && path.is_file());
    let text = match &user {
        Some(path) => std::fs::read_to_string(path).map_err(|error| error.to_string())?,
        None => builtin_source(id).ok_or_else(|| format!("unknown style “{id}”"))?.to_owned(),
    };
    let mut table: toml::Table = text.parse().map_err(|error: toml::de::Error| error.to_string())?;
    let Some(parent) = table.remove("extends") else { return Ok(table) };
    let parent = parent.as_str().ok_or("`extends` must be a style name")?.to_owned();
    // A user file may extend the built-in style of the same name.
    let skip = parent == id && user.is_some();
    let mut base = resolve_table(&parent, dir, depth + 1, skip)?;
    merge(&mut base, table);
    Ok(base)
}

fn resolve(id: &str, dir: Option<&Path>, depth: usize) -> Result<Theme, String> {
    let table = resolve_table(id, dir, depth, false)?;
    Theme::deserialize(toml::Value::Table(table)).map_err(|error| error.to_string())
}

#[derive(Clone, Debug, PartialEq)]
pub struct ThemeEntry {
    pub id: String,
    pub name: String,
    pub user: bool,
}

/// Knows the available styles and notices when user files change.
pub struct ThemeStore {
    dir: PathBuf,
    entries: Vec<ThemeEntry>,
    stamps: Vec<(PathBuf, Option<SystemTime>, u64)>,
    last_poll: Instant,
}

impl ThemeStore {
    pub fn new(dir: PathBuf) -> Self {
        let mut store = ThemeStore { dir, entries: Vec::new(), stamps: Vec::new(), last_poll: Instant::now() };
        store.rescan();
        store
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn entries(&self) -> &[ThemeEntry] {
        &self.entries
    }

    pub fn load(&self, id: &str) -> Result<Theme, String> {
        resolve(id, Some(&self.dir), 0)
    }

    fn scan_stamps(&self) -> Vec<(PathBuf, Option<SystemTime>, u64)> {
        let mut stamps: Vec<_> = std::fs::read_dir(&self.dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "toml"))
            .map(|path| {
                let meta = std::fs::metadata(&path).ok();
                let modified = meta.as_ref().and_then(|meta| meta.modified().ok());
                let len = meta.map(|meta| meta.len()).unwrap_or(0);
                (path, modified, len)
            })
            .collect();
        stamps.sort_by(|a, b| a.0.cmp(&b.0));
        stamps
    }

    fn rescan(&mut self) {
        self.stamps = self.scan_stamps();
        let mut entries: Vec<ThemeEntry> = BUILTIN
            .iter()
            .map(|(id, _)| ThemeEntry { id: (*id).to_owned(), name: Theme::builtin(id).name, user: false })
            .collect();
        for (path, _, _) in &self.stamps {
            let Some(id) = path.file_stem().map(|stem| stem.to_string_lossy().into_owned()) else { continue };
            let name = std::fs::read_to_string(path)
                .ok()
                .and_then(|text| text.parse::<toml::Table>().ok())
                .and_then(|table| table.get("name").and_then(|v| v.as_str()).map(str::to_owned))
                .unwrap_or_else(|| id.clone());
            entries.retain(|entry| entry.id != id);
            entries.push(ThemeEntry { id, name, user: true });
        }
        self.entries = entries;
    }

    /// Checks the style folder at most once a second. True when something changed.
    pub fn poll_changed(&mut self) -> bool {
        if self.last_poll.elapsed() < Duration::from_secs(1) {
            return false;
        }
        self.last_poll = Instant::now();
        if self.scan_stamps() == self.stamps {
            return false;
        }
        self.rescan();
        true
    }

    /// Writes `theme` as a complete, editable file and returns its id and path.
    pub fn export(&mut self, theme: &Theme) -> Result<(String, PathBuf), String> {
        std::fs::create_dir_all(&self.dir).map_err(|error| error.to_string())?;
        let base: String = theme
            .name
            .to_lowercase()
            .chars()
            .map(|ch| if ch.is_ascii_alphanumeric() { ch } else { '-' })
            .collect::<String>()
            .trim_matches('-')
            .to_owned();
        let base = if base.is_empty() { "style".to_owned() } else { base };
        let (id, path) = (1..)
            .map(|n| if n == 1 { format!("my-{base}") } else { format!("my-{base}-{n}") })
            .map(|id| {
                let path = self.dir.join(format!("{id}.toml"));
                (id, path)
            })
            .find(|(_, path)| !path.exists())
            .expect("a free file name");
        let mut copy = theme.clone();
        copy.name = format!("{} (custom)", theme.name);
        std::fs::write(&path, copy.to_toml()).map_err(|error| error.to_string())?;
        self.rescan();
        Ok((id, path))
    }
}

/// Applies a style to egui. Fonts are only rebuilt when they change.
pub fn apply(ctx: &egui::Context, theme: &Theme, previous_fonts: Option<&Fonts>) {
    ctx.set_theme(if theme.dark { egui::Theme::Dark } else { egui::Theme::Light });
    ctx.all_styles_mut(|style| configure(style, theme));
    if previous_fonts != Some(&theme.fonts) {
        ctx.set_fonts(fonts(&theme.fonts));
    }
}

fn fonts(spec: &Fonts) -> egui::FontDefinitions {
    let mut fonts = egui::FontDefinitions::default();
    for (path, family, name) in [
        (&spec.proportional, FontFamily::Proportional, "custom-proportional"),
        (&spec.monospace, FontFamily::Monospace, "custom-monospace"),
    ] {
        if path.trim().is_empty() {
            continue;
        }
        if let Ok(bytes) = std::fs::read(path.trim()) {
            fonts.font_data.insert(name.to_owned(), egui::FontData::from_owned(bytes).into());
            fonts.families.entry(family).or_default().insert(0, name.to_owned());
        }
    }
    fonts
}

fn configure(style: &mut egui::Style, theme: &Theme) {
    let c = &theme.colors;
    let shape = &theme.shape;
    let radius = CornerRadius::same(shape.radius.round().clamp(0.0, 255.0) as u8);
    let mut visuals = if theme.dark { egui::Visuals::dark() } else { egui::Visuals::light() };
    visuals.panel_fill = c.background.0;
    visuals.window_fill = c.card.0;
    visuals.window_stroke = Stroke::new(1.0, c.line.0);
    visuals.window_corner_radius = CornerRadius::same(shape.card_radius.round().clamp(0.0, 255.0) as u8);
    visuals.menu_corner_radius = radius;
    visuals.extreme_bg_color = c.input.0;
    visuals.text_edit_bg_color = Some(c.input.0);
    visuals.faint_bg_color = c.stripe.0;
    visuals.code_bg_color = c.code_bg.0;
    visuals.override_text_color = Some(c.text.0);
    visuals.weak_text_color = Some(c.muted.0);
    visuals.hyperlink_color = c.crumb.0;
    visuals.warn_fg_color = c.notice.0;
    visuals.error_fg_color = c.banner_text.0;
    visuals.selection.bg_fill = c.selection.0;
    visuals.selection.stroke = Stroke::new(1.0, c.selection_text.0);
    visuals.text_cursor.stroke = Stroke::new(2.0, c.accent.0);
    visuals.striped = true;

    let widgets = &mut visuals.widgets;
    widgets.noninteractive.bg_fill = c.card.0;
    widgets.noninteractive.weak_bg_fill = c.card.0;
    widgets.noninteractive.bg_stroke = Stroke::new(1.0, c.line.0);
    widgets.noninteractive.fg_stroke = Stroke::new(1.0, c.text.0);
    widgets.noninteractive.corner_radius = radius;
    for (state, fill, border, text) in [
        (&mut widgets.inactive, c.card.0, c.line.0, c.text.0),
        (&mut widgets.hovered, c.hover.0, c.accent.0, c.text.0),
        (&mut widgets.active, c.selection.0, c.accent.0, c.selection_text.0),
        (&mut widgets.open, c.hover.0, c.accent.0, c.text.0),
    ] {
        state.bg_fill = fill;
        state.weak_bg_fill = fill;
        state.bg_stroke = Stroke::new(1.0, border);
        state.fg_stroke = Stroke::new(1.5, text);
        state.corner_radius = radius;
        state.expansion = 0.0;
    }
    // Check boxes and radio buttons draw their box with bg_fill.
    widgets.inactive.bg_fill = c.input.0;
    style.visuals = visuals;

    let spacing = &mut style.spacing;
    spacing.item_spacing = Vec2::new(shape.spacing, shape.spacing * 0.75);
    spacing.button_padding = Vec2::new(10.0, 5.0);
    spacing.interact_size.y = shape.font_size + 14.0;
    spacing.combo_height = 320.0;

    style.text_styles = [
        (TextStyle::Small, FontId::proportional(shape.font_size - 2.0)),
        (TextStyle::Body, FontId::proportional(shape.font_size)),
        (TextStyle::Button, FontId::proportional(shape.font_size)),
        (TextStyle::Heading, FontId::proportional(shape.heading_size)),
        (TextStyle::Monospace, FontId::monospace(shape.mono_size)),
    ]
    .into();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("s3viewer-themes-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn builtins_resolve() {
        for (id, _) in BUILTIN {
            let theme = Theme::builtin(id);
            assert!(!theme.name.is_empty());
        }
        let dark = Theme::builtin("dark");
        assert!(dark.dark);
        // inherited from light
        assert_eq!(dark.syntax.key.0, parse_color("#7dd3fc").unwrap());
    }

    #[test]
    fn user_style_extends_and_overrides() {
        let dir = scratch("extends");
        std::fs::write(
            dir.join("mine.toml"),
            "name = \"Mine\"\nextends = \"dark\"\n[colors]\naccent = \"#ff000080\"\n",
        )
        .unwrap();
        let store = ThemeStore::new(dir.clone());
        assert!(store.entries().iter().any(|entry| entry.id == "mine" && entry.name == "Mine" && entry.user));
        let theme = store.load("mine").unwrap();
        assert!(theme.dark);
        assert_eq!(theme.colors.accent.0, Color32::from_rgba_unmultiplied(255, 0, 0, 128));
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn user_file_can_shadow_builtin_of_same_name() {
        let dir = scratch("shadow");
        std::fs::write(dir.join("dark.toml"), "extends = \"dark\"\n[shape]\nradius = 2.0\n").unwrap();
        let theme = ThemeStore::new(dir.clone()).load("dark").unwrap();
        assert_eq!(theme.shape.radius, 2.0);
        assert!(theme.dark);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn reports_mistakes() {
        let dir = scratch("errors");
        std::fs::write(dir.join("typo.toml"), "extends = \"light\"\n[colors]\naccnet = \"#000000\"\n").unwrap();
        std::fs::write(dir.join("loop.toml"), "extends = \"loop2\"\n").unwrap();
        std::fs::write(dir.join("loop2.toml"), "extends = \"loop\"\n").unwrap();
        std::fs::write(dir.join("badcolor.toml"), "extends = \"light\"\n[colors]\ntext = \"blue\"\n").unwrap();
        let store = ThemeStore::new(dir.clone());
        assert!(store.load("typo").unwrap_err().contains("accnet"));
        assert!(store.load("loop").unwrap_err().contains("loop"));
        assert!(store.load("badcolor").unwrap_err().contains("blue"));
        assert!(store.load("missing").is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn export_round_trips() {
        let dir = scratch("export");
        let mut store = ThemeStore::new(dir.clone());
        let (id, path) = store.export(&Theme::builtin("nord")).unwrap();
        assert_eq!(id, "my-nord");
        assert!(path.is_file());
        let loaded = store.load(&id).unwrap();
        assert_eq!(loaded.colors.background, Theme::builtin("nord").colors.background);
        assert_eq!(loaded.name, "Nord (custom)");
        let (second, _) = store.export(&Theme::builtin("nord")).unwrap();
        assert_eq!(second, "my-nord-2");
        let _ = std::fs::remove_dir_all(dir);
    }
}
