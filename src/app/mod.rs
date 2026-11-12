//! Main window: connection, buckets, folders and the file preview.

mod browser;
mod connection;
mod dialogs;
mod preview_panel;
mod tasks;
mod widgets;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui::{self, Margin, ViewportCommand};

use crate::format::format_size;
use crate::geotiff::{self, FnSource, Style, TiffError, TiffPreview};
use crate::i18n::{self, Language};
use crate::preview::{self, IMAGE_BYTE_LIMIT, PREVIEW_BYTE_LIMIT, Preview};
use crate::profiles;
use crate::s3::{
    BucketInfo, CancelFlag, Connected, ConnectionSettings, MAX_LISTED, ObjectEntry, S3Error, S3Result, Session,
    SignatureVersion, validate_settings,
};
use crate::settings::Settings;
use crate::theme::{self, Fonts, Theme, ThemeStore};
use crate::tr;
use crate::transfer::{self, UploadPlan};
use crate::update;
pub(crate) use dialogs::{Confirmed, Dialog};
use preview_panel::ImageView;
use tasks::{Channel, Progress, Tasks};

/// Clicks right after opening a folder are ignored, so a double click does not open two levels.
const NAV_GUARD: Duration = Duration::from_millis(400);

pub const REGION_CHOICES: &[&str] = &[
    "us-east-1",
    "RegionOne",
    "eu-central-1",
    "eu-central-2",
    "eu-west-1",
    "eu-west-2",
    "eu-west-3",
    "eu-north-1",
    "eu-south-1",
    "eu-south-2",
    "us-east-2",
    "us-west-1",
    "us-west-2",
    "ca-central-1",
    "ap-southeast-1",
    "ap-southeast-2",
    "ap-northeast-1",
    "ap-northeast-2",
    "ap-south-1",
    "sa-east-1",
];

#[derive(Default)]
pub struct Form {
    pub endpoint: String,
    pub access_key: String,
    pub secret_key: String,
    pub region: String,
    pub bucket: String,
    pub signature: SignatureVersion,
    pub path_style: bool,
    pub verify_ssl: bool,
    pub show_secret: bool,
}

impl Form {
    fn validate(&self) -> S3Result<ConnectionSettings> {
        validate_settings(
            &self.endpoint,
            &self.access_key,
            &self.secret_key,
            &self.region,
            &self.bucket,
            self.path_style,
            self.verify_ssl,
            self.signature,
        )
    }

    fn fill(&mut self, settings: &ConnectionSettings) {
        self.endpoint = settings.endpoint_url.clone();
        self.access_key = settings.access_key.clone();
        self.secret_key = settings.secret_key.clone();
        self.region = settings.region.clone();
        self.bucket = settings.direct_bucket.clone();
        self.signature = settings.signature_version;
        self.path_style = settings.path_style;
        self.verify_ssl = settings.verify_ssl;
    }
}

/// One line of the file list; `None` is the ".." entry.
pub struct Row {
    pub entry: Option<ObjectEntry>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortColumn {
    Name,
    Size,
    Modified,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TransferKind {
    Download,
    Upload,
    /// Counts objects instead of bytes.
    Delete,
}

pub struct Transfer {
    kind: TransferKind,
    progress: Arc<Progress>,
    cancel: CancelFlag,
}

pub enum PreviewContent {
    Message(String),
    Text(Box<Preview>),
    Image { preview: Box<Preview>, texture: egui::TextureHandle, view: ImageView },
    Tiff { tiff: Box<TiffPreview>, texture: egui::TextureHandle, style: Style, view: ImageView },
}

pub struct PreviewState {
    pub entry: Option<ObjectEntry>,
    pub title: String,
    pub badge: String,
    pub meta: String,
    pub notice: String,
    pub content: PreviewContent,
    inflight: Option<String>,
    cancel: Option<CancelFlag>,
    pub editing: bool,
    pub edit_text: String,
    pub save_type: String,
}

impl Default for PreviewState {
    fn default() -> Self {
        PreviewState {
            entry: None,
            title: tr!("preview"),
            badge: String::new(),
            meta: String::new(),
            notice: String::new(),
            content: PreviewContent::Message(tr!("preview_idle")),
            inflight: None,
            cancel: None,
            editing: false,
            edit_text: String::new(),
            save_type: String::new(),
        }
    }
}

impl PreviewState {
    /// Text that the copy button copies.
    pub fn text(&self) -> Option<&str> {
        let PreviewContent::Text(preview) = &self.content else { return None };
        let text = if self.editing { self.edit_text.as_str() } else { preview.text.as_str() };
        (!text.is_empty()).then_some(text)
    }

    pub fn dirty(&self) -> bool {
        match &self.content {
            PreviewContent::Text(preview) => self.editing && self.edit_text != preview.text,
            _ => false,
        }
    }
}

enum PreviewResult {
    Text(Preview),
    Image(Preview),
    Tiff(Box<TiffPreview>, u64, String),
    Failed(String),
}

/// State of the optional GitHub release check.
#[derive(Default)]
pub struct UpdateState {
    pub checking: bool,
    pub available: Option<update::Release>,
    /// Result of the last check, shown in the settings menu.
    pub message: Option<String>,
}

pub struct App {
    root: PathBuf,
    settings: Settings,
    themes: ThemeStore,
    theme: Theme,
    applied_fonts: Option<Fonts>,

    form: Form,
    profiles: Vec<String>,
    profile: String,

    session: Option<Arc<Session>>,
    connecting: bool,
    epoch: u64,

    buckets: Vec<BucketInfo>,
    bucket_filter: String,
    bucket: String,
    prefix: String,
    rows: Vec<Row>,
    file_filter: String,
    selected: HashSet<String>,
    current: Option<String>,
    anchor: Option<String>,
    sort: (SortColumn, bool),
    scroll_to_current: bool,
    focus_file_filter: bool,
    /// Shown instead of the folder counts after the next listing (e.g. "3 files uploaded").
    status_after_listing: Option<String>,

    preview: PreviewState,

    status: String,
    status_until: Option<Instant>,
    banner: Option<String>,
    busy: usize,
    transfer: Option<Transfer>,
    nav_guard_until: Instant,
    dialog: Option<Dialog>,
    tasks: Tasks,
    generations: HashMap<Channel, u64>,
    window_title: String,
    update: UpdateState,
    shown_title: String,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let root = crate::settings::config_root();
        crate::settings::import_legacy(&root);
        let settings = Settings::load(&root);
        i18n::set_language(Language::from_code(&settings.language));
        let themes = ThemeStore::new(root.join("themes"));
        let mut form = Form {
            endpoint: settings.endpoint.clone(),
            access_key: settings.access_key.clone(),
            region: settings.region.clone(),
            bucket: settings.bucket.clone(),
            signature: SignatureVersion::from_code(&settings.signature).unwrap_or_default(),
            path_style: settings.path_style,
            verify_ssl: settings.verify_ssl,
            ..Form::default()
        };
        if form.region.is_empty() {
            form.region = "us-east-1".into();
        }
        let mut app = App {
            root,
            theme: Theme::builtin("light"),
            themes,
            applied_fonts: None,
            form,
            profiles: Vec::new(),
            profile: String::new(),
            session: None,
            connecting: false,
            epoch: 0,
            buckets: Vec::new(),
            bucket_filter: String::new(),
            bucket: String::new(),
            prefix: String::new(),
            rows: Vec::new(),
            file_filter: String::new(),
            selected: HashSet::new(),
            current: None,
            anchor: None,
            sort: (SortColumn::Name, true),
            scroll_to_current: false,
            focus_file_filter: false,
            status_after_listing: None,
            preview: PreviewState::default(),
            status: tr!("not_connected"),
            status_until: None,
            banner: None,
            busy: 0,
            transfer: None,
            nav_guard_until: Instant::now(),
            dialog: None,
            tasks: Tasks::new(cc.egui_ctx.clone()),
            generations: HashMap::new(),
            window_title: "S3 Viewer".into(),
            shown_title: "S3 Viewer".into(),
            update: UpdateState::default(),
            settings,
        };
        let theme_id = app.settings.theme.clone();
        app.set_theme(&cc.egui_ctx, &theme_id);
        app.reload_profiles(None);
        app.start_update_check(false);
        app
    }

    // ------------------------------------------------------------- plumbing

    fn generation(&mut self, channel: Channel) -> u64 {
        let counter = self.generations.entry(channel).or_insert(0);
        *counter += 1;
        *counter
    }

    /// Runs `work` in the background. Answers from an older connection or
    /// an older request of the same channel are dropped.
    fn submit<T: Send + 'static>(
        &mut self,
        channel: Channel,
        work: impl FnOnce() -> S3Result<T> + Send + 'static,
        ok: impl FnOnce(&mut App, T) + Send + 'static,
        err: impl FnOnce(&mut App, String) + Send + 'static,
        track_transfer: bool,
    ) {
        let epoch = self.epoch;
        let generation = self.generation(channel);
        self.busy += 1;
        self.tasks.spawn(work, move |app, result| {
            if track_transfer {
                app.transfer = None;
            }
            app.busy = app.busy.saturating_sub(1);
            if epoch != app.epoch || app.generations.get(&channel) != Some(&generation) {
                return;
            }
            match result {
                Ok(value) => ok(app, value),
                Err(error) => err(app, error.humanize()),
            }
        });
    }

    fn set_status(&mut self, text: impl Into<String>) {
        self.status = text.into();
        self.status_until = None;
    }

    fn flash_status(&mut self, text: impl Into<String>, duration: Duration) {
        self.status = text.into();
        self.status_until = Some(Instant::now() + duration);
    }

    fn set_banner(&mut self, text: Option<String>) {
        self.banner = text.filter(|text| !text.is_empty());
    }

    fn save_settings(&mut self) {
        let s = &mut self.settings;
        s.endpoint = self.form.endpoint.trim().to_owned();
        s.access_key = self.form.access_key.trim().to_owned();
        s.region = self.form.region.trim().to_owned();
        s.bucket = self.form.bucket.trim().to_owned();
        s.signature = self.form.signature.code().to_owned();
        s.path_style = self.form.path_style;
        s.verify_ssl = self.form.verify_ssl;
        s.language = i18n::language().code().to_owned();
        let _ = self.settings.save(&self.root);
    }

    fn connected(&self) -> bool {
        self.session.is_some()
    }

    // ---------------------------------------------------------------- updates

    /// Looks for a newer release in the background. Without `manual` it only
    /// runs when enabled and the last check is more than a day old; otherwise
    /// the remembered answer is used. Failures never interrupt the user.
    fn start_update_check(&mut self, manual: bool) {
        if update::disabled_by_environment() || self.update.checking {
            return;
        }
        if !manual {
            if !self.settings.update_check {
                return;
            }
            if !update::due(self.settings.last_update_check, chrono::Utc::now().timestamp()) {
                self.update.available = self
                    .settings
                    .latest_version
                    .clone()
                    .filter(|version| update::is_newer(version, update::CURRENT))
                    .map(|version| update::Release { version, url: update::RELEASES_PAGE.to_owned() });
                return;
            }
        }
        self.update.checking = true;
        self.tasks.spawn(update::fetch_latest, |app, result| {
            app.update.checking = false;
            match result {
                Ok(release) => {
                    app.settings.last_update_check = Some(chrono::Utc::now().timestamp());
                    app.settings.latest_version = Some(release.version.clone());
                    let _ = app.settings.save(&app.root);
                    if update::is_newer(&release.version, update::CURRENT) {
                        app.update.message = None;
                        app.update.available = Some(release);
                    } else {
                        app.update.message = Some(tr!("update_latest"));
                        app.update.available = None;
                    }
                }
                Err(error) => app.update.message = Some(tr!("update_failed", error = error)),
            }
        });
    }

    // ---------------------------------------------------------------- styles

    pub fn set_theme(&mut self, ctx: &egui::Context, id: &str) {
        match self.themes.load(id) {
            Ok(theme) => {
                theme::apply(ctx, &theme, self.applied_fonts.as_ref());
                self.applied_fonts = Some(theme.fonts.clone());
                self.theme = theme;
                self.settings.theme = id.to_owned();
                self.set_banner(None);
            }
            Err(error) => {
                if self.applied_fonts.is_none() {
                    theme::apply(ctx, &self.theme, None);
                    self.applied_fonts = Some(self.theme.fonts.clone());
                }
                self.set_banner(Some(tr!("theme_load_failed", name = id, error = error)));
            }
        }
    }

    fn export_theme(&mut self, ctx: &egui::Context) {
        let theme = self.theme.clone();
        match self.themes.export(&theme) {
            Ok((id, path)) => {
                self.set_theme(ctx, &id);
                let _ = self.settings.save(&self.root);
                self.set_status(tr!("theme_exported", path = path.display()));
            }
            Err(error) => self.set_banner(Some(error)),
        }
    }

    fn open_theme_folder(&mut self, ctx: &egui::Context) {
        let dir = self.themes.dir().to_path_buf();
        if let Err(error) = std::fs::create_dir_all(&dir) {
            self.set_banner(Some(error.to_string()));
            return;
        }
        ctx.open_url(egui::OpenUrl::new_tab(format!("file://{}", dir.display())));
    }

    fn toggle_language(&mut self) {
        i18n::set_language(match i18n::language() {
            Language::De => Language::En,
            Language::En => Language::De,
        });
        if !self.connected() && self.busy == 0 {
            self.set_status(tr!("not_connected"));
        }
        if self.preview.entry.is_none() {
            self.preview.title = tr!("preview");
            if matches!(self.preview.content, PreviewContent::Message(_)) {
                self.preview.content = PreviewContent::Message(tr!("preview_idle"));
            }
        }
        self.save_settings();
    }

    // -------------------------------------------------------------- profiles

    fn reload_profiles(&mut self, select: Option<&str>) {
        match profiles::list_profile_names(&self.root) {
            Ok(names) => {
                let current = select.map(str::to_owned).unwrap_or_else(|| self.profile.clone());
                self.profile = if names.contains(&current) { current } else { String::new() };
                self.profiles = names;
            }
            Err(error) => {
                self.profiles.clear();
                self.profile.clear();
                self.set_banner(Some(error));
            }
        }
    }

    fn select_profile(&mut self, name: String) {
        self.profile = name.clone();
        if name.is_empty() {
            return;
        }
        match profiles::load_profile(&self.root, &name) {
            Ok(settings) => {
                self.form.fill(&settings);
                self.set_banner(None);
            }
            Err(error) => self.set_banner(Some(error)),
        }
    }

    fn save_named_profile(&mut self) {
        let settings = match self.form.validate() {
            Ok(settings) => settings,
            Err(error) => return self.set_banner(Some(error.humanize())),
        };
        let mut name = self.profile.clone();
        if name.is_empty() {
            name = crate::s3::host_of(&settings.endpoint_url);
            if name.is_empty() {
                name = tr!("fallback_profile");
            }
        }
        self.dialog = Some(Dialog::SaveProfile { name, settings, error: None });
    }

    fn store_profile(&mut self, name: &str, settings: &ConnectionSettings, confirmed: bool) -> Result<(), String> {
        let cleaned = profiles::normalize_profile_name(name)?;
        if !confirmed && cleaned != self.profile && self.profiles.contains(&cleaned) {
            self.dialog = Some(Dialog::Confirm {
                title: tr!("replace_connection_title"),
                body: tr!("replace_connection_body", name = cleaned),
                action: Confirmed::ReplaceProfile(cleaned, settings.clone()),
            });
            return Ok(());
        }
        let saved = profiles::save_profile(&self.root, &cleaned, settings)?;
        self.reload_profiles(Some(&saved));
        self.set_banner(None);
        self.set_status(tr!("saved_connection_status", path = self.root.display()));
        Ok(())
    }

    fn delete_named_profile(&mut self) {
        if self.profile.is_empty() {
            return;
        }
        self.dialog = Some(Dialog::Confirm {
            title: tr!("remove_connection_title"),
            body: tr!("remove_connection_body", name = self.profile, path = crate::settings::display_path(&self.root)),
            action: Confirmed::RemoveProfile(self.profile.clone()),
        });
    }

    fn remove_profile(&mut self, name: &str) {
        match profiles::delete_profile(&self.root, name) {
            Ok(()) => {
                self.reload_profiles(Some(""));
                self.set_status(tr!("removed_connection", name = name));
            }
            Err(error) => self.set_banner(Some(error)),
        }
    }

    // ------------------------------------------------------------ connection

    fn connect(&mut self) {
        let settings = match self.form.validate() {
            Ok(settings) => settings,
            Err(error) => return self.set_banner(Some(error.humanize())),
        };
        self.set_banner(None);
        self.save_settings();
        self.epoch += 1;
        self.connecting = true;
        self.session = None;
        self.clear_browser();
        self.set_status(tr!("connecting"));
        let epoch = self.epoch;
        self.busy += 1;
        self.tasks.spawn(
            move || -> S3Result<(Session, Connected)> {
                let session = Session::new(settings)?;
                let connected = session.establish()?;
                Ok((session, connected))
            },
            move |app, result| {
                app.busy = app.busy.saturating_sub(1);
                if epoch != app.epoch {
                    return;
                }
                app.connecting = false;
                match result {
                    Ok((session, connected)) => app.apply_connection(session, connected),
                    Err(error) => {
                        app.set_banner(Some(error.humanize()));
                        app.set_status(tr!("not_connected"));
                    }
                }
            },
        );
    }

    fn disconnect(&mut self) {
        self.epoch += 1;
        self.session = None;
        self.connecting = false;
        if let Some(transfer) = self.transfer.take() {
            transfer.cancel.cancel();
        }
        self.clear_browser();
        self.set_banner(None);
        self.window_title = "S3 Viewer".into();
        self.set_status(tr!("not_connected"));
    }

    fn apply_connection(&mut self, session: Session, connected: Connected) {
        let host = session.host().to_owned();
        let target = session.settings.direct_bucket.clone();
        self.session = Some(Arc::new(session));
        self.buckets = connected.buckets;
        self.window_title = format!("S3 Viewer — {host}");
        if let Some(notice) = connected.notice {
            self.set_status(notice);
        } else if self.buckets.is_empty() {
            self.set_status(tr!("no_buckets"));
            self.set_banner(Some(tr!("no_bucket_list")));
        } else {
            self.set_banner(None);
            self.set_status(tr!("bucket_count", count = self.buckets.len()));
        }
        if !target.is_empty() && self.buckets.iter().any(|bucket| bucket.name == target) {
            self.open_bucket(target);
        } else if self.buckets.len() == 1 {
            let only = self.buckets[0].name.clone();
            self.open_bucket(only);
        }
    }

    fn clear_browser(&mut self) {
        self.buckets.clear();
        self.bucket.clear();
        self.prefix.clear();
        self.rows.clear();
        self.selected.clear();
        self.current = None;
        self.anchor = None;
        self.cancel_preview();
        self.preview = PreviewState::default();
    }

    // -------------------------------------------------------------- browsing

    fn open_bucket(&mut self, name: String) {
        self.bucket = name;
        self.open_prefix(String::new());
    }

    fn open_prefix(&mut self, prefix: String) {
        let Some(session) = self.session.clone() else { return };
        if self.bucket.is_empty() {
            return;
        }
        let prefix = if prefix.is_empty() || prefix.ends_with('/') { prefix } else { format!("{prefix}/") };
        self.nav_guard_until = Instant::now() + NAV_GUARD;
        self.prefix = prefix.clone();
        self.generation(Channel::Preview);
        self.cancel_preview();
        self.preview =
            PreviewState { content: PreviewContent::Message(tr!("loading_folder")), ..PreviewState::default() };
        self.file_filter.clear();
        self.fill_rows(Vec::new());
        self.set_status(tr!("loading_folder"));
        let bucket = self.bucket.clone();
        self.submit(
            Channel::List,
            move || session.list_folder(&bucket, &prefix),
            |app, (entries, capped)| app.on_listed(entries, capped),
            |app, message| {
                app.set_banner(Some(message));
                app.set_status(tr!("folder_failed"));
            },
            false,
        );
    }

    fn on_listed(&mut self, entries: Vec<ObjectEntry>, capped: bool) {
        let folders = entries.iter().filter(|entry| entry.is_folder).count();
        let files = entries.len() - folders;
        let mut message = if entries.is_empty() {
            if self.prefix.is_empty() { tr!("bucket_empty") } else { tr!("folder_empty") }
        } else {
            tr!("counts", folders = folders, files = files)
        };
        if capped {
            message += &tr!("list_capped", limit = MAX_LISTED);
        }
        self.fill_rows(entries);
        self.set_banner(None);
        let message = self.status_after_listing.take().unwrap_or(message);
        self.set_status(message);
        if self.preview.entry.is_none() {
            self.preview.content = PreviewContent::Message(tr!("preview_idle"));
        }
    }

    fn fill_rows(&mut self, entries: Vec<ObjectEntry>) {
        let mut rows: Vec<Row> = Vec::with_capacity(entries.len() + 1);
        if !self.prefix.is_empty() {
            rows.push(Row { entry: None });
        }
        rows.extend(entries.into_iter().map(|entry| Row { entry: Some(entry) }));
        self.rows = rows;
        self.sort_rows();
        self.selected.clear();
        self.current = None;
        self.anchor = None;
    }

    fn sort_rows(&mut self) {
        let (column, ascending) = self.sort;
        let start = usize::from(self.rows.first().is_some_and(|row| row.entry.is_none()));
        self.rows[start..].sort_by(|a, b| {
            let (Some(a), Some(b)) = (&a.entry, &b.entry) else { return std::cmp::Ordering::Equal };
            let folders_first = b.is_folder.cmp(&a.is_folder);
            let by_name = || a.name.to_lowercase().cmp(&b.name.to_lowercase());
            let order = match column {
                SortColumn::Name => by_name(),
                SortColumn::Size => a.size.cmp(&b.size).then_with(by_name),
                SortColumn::Modified => a.last_modified.cmp(&b.last_modified).then_with(by_name),
            };
            folders_first.then(if ascending { order } else { order.reverse() })
        });
    }

    /// Indices of rows that pass the file filter. The ".." row always stays.
    fn visible_rows(&self) -> Vec<usize> {
        let query = self.file_filter.trim().to_lowercase();
        self.rows
            .iter()
            .enumerate()
            .filter(|(_, row)| match &row.entry {
                None => true,
                Some(entry) => query.is_empty() || entry.name.to_lowercase().contains(&query),
            })
            .map(|(index, _)| index)
            .collect()
    }

    fn selected_entries(&self) -> Vec<ObjectEntry> {
        self.visible_rows()
            .into_iter()
            .filter_map(|index| self.rows[index].entry.as_ref())
            .filter(|entry| self.selected.contains(&entry.key))
            .cloned()
            .collect()
    }

    fn go_up(&mut self) {
        if self.prefix.is_empty() || Instant::now() < self.nav_guard_until {
            return;
        }
        let trimmed = self.prefix.trim_end_matches('/');
        let parent = match trimmed.rsplit_once('/') {
            Some((parent, _)) => format!("{parent}/"),
            None => String::new(),
        };
        self.open_prefix(parent);
    }

    fn refresh(&mut self) {
        let Some(session) = self.session.clone() else { return };
        if !self.bucket.is_empty() {
            self.nav_guard_until = Instant::now();
            self.open_prefix(self.prefix.clone());
            return;
        }
        self.submit(
            Channel::Buckets,
            move || session.list_buckets(),
            |app, buckets| {
                app.buckets = buckets;
                app.set_status(tr!("buckets_refreshed"));
            },
            |app, message| {
                app.set_banner(Some(message));
                app.set_status(tr!("folder_failed"));
            },
            false,
        );
    }

    fn open_crumb(&mut self, prefix: String) {
        if Instant::now() < self.nav_guard_until {
            return;
        }
        self.open_prefix(prefix);
    }

    /// A row was clicked or chosen with the keyboard.
    fn activate(&mut self, row: usize, navigate: bool) {
        if navigate && Instant::now() < self.nav_guard_until {
            return;
        }
        let Some(row) = self.rows.get(row) else { return };
        let Some(entry) = row.entry.clone() else {
            if navigate {
                self.go_up();
            }
            return;
        };
        if entry.is_folder {
            if navigate {
                self.open_prefix(entry.key);
            } else {
                self.show_message_preview(&entry.name, tr!("open_folder"));
            }
            return;
        }
        self.load_preview(entry, false);
    }

    // --------------------------------------------------------------- preview

    fn cancel_preview(&mut self) {
        if let Some(cancel) = self.preview.cancel.take() {
            cancel.cancel();
        }
        self.preview.inflight = None;
    }

    fn show_message_preview(&mut self, title: &str, message: String) {
        self.preview.title = title.to_owned();
        self.preview.content = PreviewContent::Message(message);
        self.preview.editing = false;
    }

    fn load_preview(&mut self, entry: ObjectEntry, discard_confirmed: bool) {
        self.load_preview_with(entry, discard_confirmed, false);
    }

    /// Loads the shown GeoTIFF again without the byte budget.
    fn load_full_tiff(&mut self) {
        if let Some(entry) = self.preview.entry.clone() {
            self.load_preview_with(entry, true, true);
        }
    }

    fn load_preview_with(&mut self, entry: ObjectEntry, discard_confirmed: bool, unlimited: bool) {
        let same = self.preview.entry.as_ref().is_some_and(|current| current.key == entry.key);
        if !discard_confirmed && self.preview.dirty() && !same {
            self.dialog = Some(Dialog::Confirm {
                title: tr!("discard_title"),
                body: tr!("discard_body"),
                action: Confirmed::LoadPreview(entry),
            });
            return;
        }
        let Some(session) = self.session.clone() else { return };
        // Clicking the file that is already shown keeps it (and any edits) as it is.
        let shown =
            same && self.preview.inflight.is_none() && !matches!(self.preview.content, PreviewContent::Message(_));
        if self.preview.inflight.as_deref() == Some(entry.key.as_str()) || (shown && !discard_confirmed) {
            return;
        }
        self.cancel_preview();
        let cancel = CancelFlag::new();
        self.preview = PreviewState {
            entry: Some(entry.clone()),
            title: entry.name.clone(),
            meta: entry.size.map(|size| format_size(Some(size))).unwrap_or_default(),
            content: PreviewContent::Message(tr!("loading_preview")),
            inflight: Some(entry.key.clone()),
            cancel: Some(cancel.clone()),
            ..PreviewState::default()
        };
        let bucket = self.bucket.clone();
        let key = entry.key.clone();
        let options = geotiff::Options {
            edge: self.settings.geotiff_edge.clamp(256, 8192),
            budget: if unlimited { u64::MAX } else { self.settings.geotiff_budget_mb.clamp(1, 4096) * 1024 * 1024 },
            ..geotiff::Options::default()
        };
        let ctx_key = key.clone();
        self.submit(
            Channel::Preview,
            move || fetch_preview(&session, &bucket, &key, &options, &cancel),
            move |app, result| {
                if app.preview.inflight.as_deref() != Some(ctx_key.as_str()) {
                    return;
                }
                app.preview.inflight = None;
                app.preview.cancel = None;
                app.show_preview(result);
            },
            |app, message| {
                app.preview.inflight = None;
                app.preview.cancel = None;
                app.preview.badge.clear();
                app.preview.content = PreviewContent::Message(message);
            },
            false,
        );
    }

    fn show_preview(&mut self, result: PreviewResult) {
        let ctx = self.tasks.ctx().clone();
        let state = &mut self.preview;
        state.editing = false;
        match result {
            PreviewResult::Failed(message) => {
                state.badge = "TIFF".into();
                state.content = PreviewContent::Message(message);
            }
            PreviewResult::Tiff(tiff, size, content_type) => {
                state.badge = "TIFF".into();
                state.meta = meta_text(size, &content_type);
                state.notice = tiff.notice.clone();
                let style = tiff.raster.default_style();
                let texture = load_texture(&ctx, "geotiff", &tiff.raster.render(&style));
                state.content = PreviewContent::Tiff { tiff, texture, style, view: ImageView::default() };
            }
            PreviewResult::Image(preview) | PreviewResult::Text(preview) => {
                state.title = preview.filename.clone();
                state.badge = preview.kind.badge();
                state.meta = meta_text(preview.size, &preview.content_type);
                let silent = matches!(preview.kind, preview::Kind::Binary | preview::Kind::Empty);
                state.notice = if silent { String::new() } else { preview.notice.clone() };
                if silent {
                    let message = if preview.notice.is_empty() { tr!("no_preview") } else { preview.notice.clone() };
                    state.content = PreviewContent::Message(message);
                    return;
                }
                if let Some(image) = preview.image.as_ref() {
                    let texture = load_texture(&ctx, "image", image);
                    state.content =
                        PreviewContent::Image { preview: Box::new(preview), texture, view: ImageView::default() };
                    return;
                }
                if preview.kind == preview::Kind::Image {
                    state.content = PreviewContent::Message(preview.notice.clone());
                    state.notice.clear();
                    return;
                }
                state.save_type = preview::content_type_for_save(preview.kind, &preview.content_type);
                state.content = PreviewContent::Text(Box::new(preview));
            }
        }
    }

    fn copy_preview(&mut self, ctx: &egui::Context) {
        if let Some(text) = self.preview.text() {
            ctx.copy_text(text.to_owned());
            self.flash_status(tr!("copied"), Duration::from_secs(3));
        }
    }

    fn start_editing(&mut self) {
        if let PreviewContent::Text(preview) = &self.preview.content
            && preview.editable()
        {
            self.preview.edit_text = preview.text.clone();
            self.preview.editing = true;
        }
    }

    fn save_edits(&mut self) {
        let Some(entry) = self.preview.entry.clone() else { return };
        let Some(session) = self.session.clone() else { return };
        if !self.preview.dirty() {
            return;
        }
        let text = self.preview.edit_text.clone();
        let content_type = self.preview.save_type.clone();
        let bucket = self.bucket.clone();
        let key = entry.key.clone();
        self.set_status(tr!("writing", name = entry.name));
        let written = text.clone();
        self.submit(
            Channel::SaveText,
            move || session.put_bytes(&bucket, &key, text.as_bytes(), &content_type).map(|_| key),
            move |app, key| {
                if app.preview.entry.as_ref().is_some_and(|entry| entry.key == key)
                    && let PreviewContent::Text(preview) = &mut app.preview.content
                {
                    preview.size = written.len() as u64;
                    preview.set_text(written);
                    app.preview.meta = meta_text(preview.size, &preview.content_type);
                }
                app.set_banner(None);
                app.set_status(tr!("written", key = key));
            },
            |app, message| app.set_banner(Some(message)),
            false,
        );
    }

    fn reselect_preview_entry(&mut self) {
        if let Some(entry) = &self.preview.entry {
            self.selected = HashSet::from([entry.key.clone()]);
            self.current = Some(entry.key.clone());
            self.anchor = self.current.clone();
        }
    }

    // ------------------------------------------------------------- transfers

    fn begin_transfer(&mut self, kind: TransferKind, total: u64) -> (Arc<Progress>, CancelFlag) {
        let progress = Arc::new(Progress::default());
        progress.set_total(total);
        let cancel = CancelFlag::new();
        self.transfer = Some(Transfer { kind, progress: progress.clone(), cancel: cancel.clone() });
        (progress, cancel)
    }

    fn cancel_transfer(&mut self) {
        if let Some(transfer) = &self.transfer
            && !transfer.cancel.is_cancelled()
        {
            transfer.cancel.cancel();
            self.set_status(tr!("download_cancelling"));
        }
    }

    fn on_transfer_error(&mut self, kind: TransferKind, message: String) {
        if message == tr!("download_cancelled") {
            self.set_banner(None);
            self.set_status(match kind {
                TransferKind::Download => tr!("download_cancelled"),
                TransferKind::Upload => tr!("upload_cancelled"),
                TransferKind::Delete => tr!("delete_cancelled"),
            });
            return;
        }
        self.set_banner(Some(message));
    }

    fn download_current(&mut self) {
        if let Some(entry) = self.preview.entry.clone().filter(|entry| !entry.is_folder) {
            self.download_entries(vec![entry]);
        }
    }

    fn download_selected(&mut self) {
        let entries = self.selected_entries();
        if !entries.is_empty() {
            self.download_entries(entries);
        }
    }

    fn download_entries(&mut self, entries: Vec<ObjectEntry>) {
        let Some(session) = self.session.clone() else { return };
        if entries.is_empty() || self.transfer.is_some() {
            return;
        }
        let bucket = self.bucket.clone();
        if entries.len() == 1 && !entries[0].is_folder {
            let entry = entries[0].clone();
            let Some(destination) =
                rfd::FileDialog::new().set_title(tr!("save_one")).set_file_name(&entry.name).save_file()
            else {
                return;
            };
            self.set_status(tr!("saving", name = entry.name));
            let size = entry.size.unwrap_or(0);
            let (progress, cancel) = self.begin_transfer(TransferKind::Download, size);
            self.submit(
                Channel::Download,
                move || {
                    session.download(&bucket, &entry.key, &destination, &|amount| progress.add(amount), &cancel)?;
                    Ok(destination)
                },
                |app, path| app.set_status(tr!("saved_at", path = path.display())),
                |app, message| app.on_transfer_error(TransferKind::Download, message),
                true,
            );
            return;
        }
        let Some(directory) = rfd::FileDialog::new().set_title(tr!("choose_download_dir")).pick_folder() else {
            return;
        };
        self.set_status(tr!("saving_files"));
        let prefix = self.prefix.clone();
        let (progress, cancel) = self.begin_transfer(TransferKind::Download, 0);
        let label = directory.display().to_string();
        self.submit(
            Channel::Download,
            move || download_many(&session, &bucket, &prefix, &entries, &directory, &progress, &cancel),
            move |app, (saved, capped)| {
                if saved == 0 {
                    app.set_banner(Some(tr!("no_files_to_save")));
                    app.set_status(tr!("no_files_saved"));
                    return;
                }
                app.set_banner(None);
                app.set_status(transfer::saved_message(saved, &label, capped, MAX_LISTED));
            },
            |app, message| app.on_transfer_error(TransferKind::Download, message),
            true,
        );
    }

    fn upload_files(&mut self) {
        if !self.connected() || self.bucket.is_empty() || self.transfer.is_some() {
            return;
        }
        let Some(paths) = rfd::FileDialog::new().set_title(tr!("choose_upload_files")).pick_files() else { return };
        match transfer::upload_plan(&self.prefix, &paths) {
            Ok(plan) => self.check_upload(plan),
            Err(error) => self.set_banner(Some(error)),
        }
    }

    fn upload_folder(&mut self) {
        if !self.connected() || self.bucket.is_empty() || self.transfer.is_some() {
            return;
        }
        let Some(directory) = rfd::FileDialog::new().set_title(tr!("choose_upload_folder")).pick_folder() else {
            return;
        };
        match transfer::folder_upload_plan(&self.prefix, &directory) {
            Ok(plan) => self.check_upload(plan),
            Err(error) => self.set_banner(Some(error)),
        }
    }

    /// Looks for files that the upload would replace and asks before replacing them.
    fn check_upload(&mut self, plan: UploadPlan) {
        let Some(session) = self.session.clone() else { return };
        let mut existing: HashSet<String> = self
            .rows
            .iter()
            .filter_map(|row| row.entry.as_ref())
            .filter(|entry| !entry.is_folder)
            .map(|entry| entry.key.clone())
            .collect();
        let folder_keys: HashSet<String> = self
            .rows
            .iter()
            .filter_map(|row| row.entry.as_ref())
            .filter(|entry| entry.is_folder)
            .map(|entry| entry.key.clone())
            .collect();
        let current = self.prefix.clone();
        let remote_folders: HashSet<String> = plan
            .iter()
            .filter_map(|(_, key)| key.strip_prefix(current.as_str()))
            .filter_map(|rest| rest.split_once('/').map(|(folder, _)| format!("{current}{folder}/")))
            .filter(|folder| folder_keys.contains(folder))
            .collect();
        if remote_folders.is_empty() {
            self.confirm_upload(plan, &existing);
            return;
        }
        self.set_status(tr!("checking_files"));
        let bucket = self.bucket.clone();
        self.submit(
            Channel::Check,
            move || {
                let mut keys = Vec::new();
                for folder in remote_folders {
                    keys.extend(session.list_files(&bucket, &folder)?.0.into_iter().map(|(key, _)| key));
                }
                Ok(keys)
            },
            move |app, keys| {
                existing.extend(keys);
                app.confirm_upload(plan, &existing);
            },
            |app, message| app.set_banner(Some(message)),
            false,
        );
    }

    fn confirm_upload(&mut self, plan: UploadPlan, existing: &HashSet<String>) {
        let collisions = transfer::colliding_keys(&plan, existing);
        if collisions.is_empty() {
            self.start_upload(plan);
            return;
        }
        let mut names: Vec<String> =
            collisions.iter().take(12).map(|key| key.rsplit('/').next().unwrap_or(key).to_owned()).collect();
        if collisions.len() > 12 {
            names.push(tr!("replace_more", count = collisions.len() - 12).trim_start().to_owned());
        }
        self.dialog = Some(Dialog::Confirm {
            title: tr!("replace_title"),
            body: tr!("replace_body", names = names.join("\n")),
            action: Confirmed::Upload(plan),
        });
    }

    fn delete_selected(&mut self) {
        let entries = self.selected_entries();
        if !self.connected() || entries.is_empty() || self.transfer.is_some() {
            return;
        }
        let mut names: Vec<String> = entries.iter().take(12).map(|entry| entry.name.clone()).collect();
        if entries.len() > 12 {
            names.push(tr!("replace_more", count = entries.len() - 12).trim_start().to_owned());
        }
        let mut body = tr!("delete_body", names = names.join("\n"));
        if entries.iter().any(|entry| entry.is_folder) {
            body += &tr!("delete_folders");
        }
        self.dialog = Some(Dialog::Confirm { title: tr!("delete"), body, action: Confirmed::Delete(entries) });
    }

    fn start_delete(&mut self, entries: Vec<ObjectEntry>) {
        let Some(session) = self.session.clone() else { return };
        let bucket = self.bucket.clone();
        let folder = self.prefix.clone();
        self.set_banner(None);
        let (progress, cancel) = self.begin_transfer(TransferKind::Delete, 0);
        self.submit(
            Channel::Delete,
            move || {
                let (keys, capped) = keys_for_delete(&session, &bucket, &entries)?;
                progress.set_total(keys.len().max(1) as u64);
                let count = session.delete_keys(&bucket, &keys, &|amount| progress.add(amount), &cancel)?;
                Ok((bucket, count, capped))
            },
            move |app, (bucket, count, capped)| {
                let mut message = if count == 1 { tr!("deleted_one") } else { tr!("deleted_many", count = count) };
                if capped {
                    message += &tr!("list_capped", limit = MAX_LISTED);
                }
                app.set_banner(None);
                app.set_status(message.clone());
                if app.connected() && app.bucket == bucket && app.prefix == folder {
                    app.nav_guard_until = Instant::now();
                    app.status_after_listing = Some(message);
                    app.open_prefix(folder);
                }
            },
            |app, message| app.on_transfer_error(TransferKind::Delete, message),
            true,
        );
    }

    fn ask_new_folder(&mut self) {
        if self.connected() && !self.bucket.is_empty() {
            self.dialog = Some(Dialog::NewFolder { name: String::new(), error: None });
        }
    }

    /// Creates the folder, or explains why the name does not work.
    fn create_folder(&mut self, name: &str) -> Result<(), String> {
        let key = transfer::folder_marker_key(&self.prefix, name)?;
        let folder_name = key.trim_end_matches('/').rsplit('/').next().unwrap_or_default().to_owned();
        let exists = self
            .rows
            .iter()
            .filter_map(|row| row.entry.as_ref())
            .any(|entry| entry.is_folder && entry.name == folder_name);
        if exists {
            return Err(tr!("new_folder_exists", name = folder_name));
        }
        let Some(session) = self.session.clone() else { return Ok(()) };
        let bucket = self.bucket.clone();
        let folder = self.prefix.clone();
        self.submit(
            Channel::NewFolder,
            move || session.create_folder(&bucket, &key).map(|_| bucket),
            move |app, bucket| {
                let message = tr!("new_folder_created", name = folder_name);
                app.set_banner(None);
                app.set_status(message.clone());
                if app.connected() && app.bucket == bucket && app.prefix == folder {
                    app.nav_guard_until = Instant::now();
                    app.status_after_listing = Some(message);
                    app.open_prefix(folder);
                }
            },
            |app, message| app.set_banner(Some(message)),
            false,
        );
        Ok(())
    }

    fn start_upload(&mut self, plan: UploadPlan) {
        let Some(session) = self.session.clone() else { return };
        let destination = transfer::upload_destination_label(&self.bucket, &self.prefix);
        let count = plan.len();
        let noun = if count == 1 { tr!("noun_file") } else { tr!("noun_files") };
        let total: u64 = plan.iter().map(|(path, _)| std::fs::metadata(path).map(|m| m.len()).unwrap_or(0)).sum();
        self.set_status(tr!("uploading", count = count, noun = noun, destination = destination));
        let (progress, cancel) = self.begin_transfer(TransferKind::Upload, total);
        let bucket = self.bucket.clone();
        let folder = self.prefix.clone();
        self.submit(
            Channel::Upload,
            move || {
                for (source, key) in &plan {
                    session.upload(&bucket, key, source, &|amount| progress.add(amount), &cancel)?;
                }
                Ok(bucket)
            },
            move |app, bucket| {
                app.set_banner(None);
                let message = transfer::uploaded_message(count, &destination);
                app.set_status(message.clone());
                if app.connected() && app.bucket == bucket && app.prefix == folder {
                    app.nav_guard_until = Instant::now();
                    app.status_after_listing = Some(message);
                    app.open_prefix(folder);
                }
            },
            |app, message| app.on_transfer_error(TransferKind::Upload, message),
            true,
        );
    }

    // ------------------------------------------------------------- shortcuts

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        if self.dialog.is_some() {
            return;
        }
        let typing = ctx.egui_wants_keyboard_input();
        let (f5, find, select_all, enter, backspace, up, down) = ctx.input_mut(|input| {
            (
                input.consume_key(egui::Modifiers::NONE, egui::Key::F5),
                input.consume_key(egui::Modifiers::COMMAND, egui::Key::F),
                !typing && input.consume_key(egui::Modifiers::COMMAND, egui::Key::A),
                !typing && input.key_pressed(egui::Key::Enter),
                !typing && input.key_pressed(egui::Key::Backspace),
                !typing && input.key_pressed(egui::Key::ArrowUp),
                !typing && input.key_pressed(egui::Key::ArrowDown),
            )
        });
        if f5 {
            self.refresh();
        }
        if find {
            self.focus_file_filter = true;
        }
        if self.rows.is_empty() {
            return;
        }
        if select_all {
            self.selected = self
                .visible_rows()
                .into_iter()
                .filter_map(|index| self.rows[index].entry.as_ref().map(|entry| entry.key.clone()))
                .collect();
        }
        if backspace {
            self.go_up();
        }
        if enter && let Some(index) = self.current_index() {
            self.activate(index, true);
        }
        if up || down {
            let visible = self.visible_rows();
            let position = self.current_index().and_then(|index| visible.iter().position(|&v| v == index));
            let next = match (position, down) {
                (None, _) => visible.first().copied(),
                (Some(p), true) => visible.get(p + 1).copied(),
                (Some(p), false) => p.checked_sub(1).and_then(|p| visible.get(p).copied()),
            };
            if let Some(index) = next {
                let key = self.row_key(index);
                self.selected = key.iter().cloned().collect();
                self.current = key.clone();
                self.anchor = key;
                self.scroll_to_current = true;
                self.activate(index, false);
            }
        }
    }

    /// Selection key of a row; ".." uses an empty key.
    fn row_key(&self, index: usize) -> Option<String> {
        self.rows.get(index).map(|row| row.entry.as_ref().map(|entry| entry.key.clone()).unwrap_or_default())
    }

    fn current_index(&self) -> Option<usize> {
        let current = self.current.as_ref()?;
        self.rows.iter().position(|row| row.entry.as_ref().map(|entry| &entry.key).unwrap_or(&String::new()) == current)
    }

    fn process_tasks(&mut self) {
        for callback in self.tasks.finished() {
            callback(self);
        }
        if let Some(until) = self.status_until
            && Instant::now() >= until
        {
            self.status.clear();
            self.status_until = None;
        }
    }
}

fn meta_text(size: u64, content_type: &str) -> String {
    let mut parts = vec![format_size(Some(size))];
    if !content_type.is_empty() {
        parts.push(content_type.to_owned());
    }
    parts.join(" · ")
}

fn load_texture(ctx: &egui::Context, name: &str, image: &preview::RgbaImage) -> egui::TextureHandle {
    let color = egui::ColorImage::from_rgba_unmultiplied([image.width, image.height], &image.pixels);
    let options = egui::TextureOptions {
        magnification: egui::TextureFilter::Nearest,
        minification: egui::TextureFilter::Linear,
        ..Default::default()
    };
    ctx.load_texture(name, color, options)
}

fn fetch_preview(
    session: &Arc<Session>,
    bucket: &str,
    key: &str,
    options: &geotiff::Options,
    cancel: &CancelFlag,
) -> S3Result<PreviewResult> {
    if preview::is_tiff(key, "") {
        return fetch_tiff(session, bucket, key, options, cancel);
    }
    let limit = if preview::is_image(key, "") { IMAGE_BYTE_LIMIT } else { PREVIEW_BYTE_LIMIT };
    let mut fetched = session.fetch(bucket, key, limit)?;
    if preview::is_tiff(key, &fetched.content_type) {
        return fetch_tiff(session, bucket, key, options, cancel);
    }
    let image = preview::is_image(key, &fetched.content_type);
    if image && limit < IMAGE_BYTE_LIMIT && fetched.truncated {
        fetched = session.fetch(bucket, key, IMAGE_BYTE_LIMIT)?;
    }
    if image && preview::detect_kind(key, &fetched.content_type, &fetched.data) == preview::Kind::Image {
        return Ok(PreviewResult::Image(preview::build_image_preview(
            key,
            &fetched.data,
            &fetched.content_type,
            fetched.size,
            fetched.truncated,
        )));
    }
    Ok(PreviewResult::Text(preview::build_preview(
        key,
        &fetched.data,
        &fetched.content_type,
        Some(fetched.size),
        fetched.truncated,
    )))
}

fn fetch_tiff(
    session: &Arc<Session>,
    bucket: &str,
    key: &str,
    options: &geotiff::Options,
    cancel: &CancelFlag,
) -> S3Result<PreviewResult> {
    let meta = session.head(bucket, key)?;
    let content_type = preview::content_type_base(&meta.content_type);
    let content_type = if content_type.is_empty() { "image/tiff".to_owned() } else { content_type };
    let Some(size) = meta.size else {
        return Ok(PreviewResult::Failed(tr!("tiff_failed")));
    };
    if size == 0 {
        return Ok(PreviewResult::Failed(tr!("empty_file")));
    }
    let reader = session.clone();
    let (bucket_name, key_name) = (bucket.to_owned(), key.to_owned());
    let source = FnSource {
        len: size,
        read: move |offset, len| {
            reader.read_range(&bucket_name, &key_name, offset, len).map_err(|error| error.humanize())
        },
    };
    match geotiff::load(&source, options, &|| cancel.is_cancelled()) {
        Ok(tiff) => Ok(PreviewResult::Tiff(Box::new(tiff), size, content_type)),
        Err(TiffError::Cancelled) => Err(S3Error::Cancelled),
        Err(TiffError::Unsupported(text)) => Ok(PreviewResult::Failed(text)),
        Err(TiffError::Failed(detail)) => Ok(PreviewResult::Failed(format!("{}\n\n{detail}", tr!("tiff_failed")))),
    }
}

/// Downloads files and whole folders into `directory`, keeping the structure below `prefix`.
/// Keys to delete for the selection: files as they are, folders with
/// everything below them and their own placeholder object.
fn keys_for_delete(session: &Session, bucket: &str, entries: &[ObjectEntry]) -> S3Result<(Vec<String>, bool)> {
    let mut keys = Vec::new();
    let mut capped = false;
    for entry in entries {
        if entry.is_folder {
            let (found, entry_capped) = session.list_object_keys(bucket, &entry.key)?;
            capped |= entry_capped;
            keys.extend(found);
            keys.push(if entry.key.ends_with('/') { entry.key.clone() } else { format!("{}/", entry.key) });
        } else {
            keys.push(entry.key.clone());
        }
    }
    Ok((keys, capped))
}

fn download_many(
    session: &Session,
    bucket: &str,
    prefix: &str,
    entries: &[ObjectEntry],
    directory: &Path,
    progress: &Progress,
    cancel: &CancelFlag,
) -> S3Result<(usize, bool)> {
    cancel.check()?;
    let mut capped = false;
    let mut jobs: Vec<(String, String, u64)> = Vec::new();
    for entry in entries {
        if entry.is_folder {
            let (records, entry_capped) = session.list_files(bucket, &entry.key)?;
            capped |= entry_capped;
            jobs.extend(records.into_iter().map(|(key, size)| {
                let relative = transfer::relative_name(prefix, &key);
                (key, relative, size)
            }));
        } else {
            jobs.push((entry.key.clone(), entry.name.clone(), entry.size.unwrap_or(0)));
        }
        cancel.check()?;
    }
    progress.set_total(jobs.iter().map(|(_, _, size)| size).sum());
    let mut saved = 0;
    for (key, relative, _) in jobs {
        cancel.check()?;
        let target = transfer::destination_for(directory, &relative).map_err(S3Error::Invalid)?;
        let target = transfer::unique_path(&target);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        session.download(bucket, &key, &target, &|amount| progress.add(amount), cancel)?;
        saved += 1;
    }
    Ok((saved, capped))
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.process_tasks();
        if self.themes.poll_changed() {
            let id = self.settings.theme.clone();
            self.set_theme(&ctx, &id);
        }
        self.handle_shortcuts(&ctx);
        if self.shown_title != self.window_title {
            self.shown_title = self.window_title.clone();
            ctx.send_viewport_cmd(ViewportCommand::Title(self.window_title.clone()));
        }

        let theme = self.theme.clone();
        let background = egui::Frame::new().fill(theme.colors.background.0);

        egui::Panel::top("connection")
            .frame(background.inner_margin(Margin { left: 16, right: 16, top: 16, bottom: 4 }))
            .show(ui, |ui| {
                self.connection_ui(ui);
                if let Some(text) = self.banner.clone() {
                    ui.add_space(6.0);
                    if widgets::banner(ui, &theme, &text) {
                        self.banner = None;
                    }
                }
            });
        egui::Panel::bottom("status")
            .frame(background.inner_margin(Margin { left: 16, right: 16, top: 4, bottom: 8 }))
            .show(ui, |ui| self.status_ui(ui));
        if !self.settings.preview_detached {
            let panel = match self.settings.preview_side {
                crate::settings::PreviewSide::Right => egui::Panel::right("preview"),
                crate::settings::PreviewSide::Left => egui::Panel::left("preview"),
            };
            panel
                .frame(background.inner_margin(Margin { left: 12, right: 16, top: 8, bottom: 8 }))
                .default_size(460.0)
                .min_size(340.0)
                .show(ui, |ui| self.preview_ui(ui));
        }
        egui::Panel::left("buckets")
            .frame(background.inner_margin(Margin { left: 16, right: 8, top: 8, bottom: 8 }))
            .default_size(250.0)
            .size_range(200.0..=360.0)
            .show(ui, |ui| self.buckets_ui(ui));
        egui::CentralPanel::default()
            .frame(background.inner_margin(Margin { left: 8, right: 8, top: 8, bottom: 8 }))
            .show(ui, |ui| self.files_ui(ui));

        if self.settings.preview_detached {
            let title = match &self.preview.entry {
                Some(entry) => tr!("preview_named", name = entry.name),
                None => tr!("preview"),
            };
            ctx.show_viewport_immediate(
                egui::ViewportId::from_hash_of("preview-window"),
                egui::ViewportBuilder::default()
                    .with_title(title)
                    .with_inner_size([640.0, 780.0])
                    .with_min_inner_size([340.0, 300.0]),
                |ui, _class| {
                    egui::CentralPanel::default()
                        .frame(egui::Frame::new().fill(theme.colors.background.0).inner_margin(Margin::same(12)))
                        .show(ui, |ui| self.preview_ui(ui));
                    if ui.input(|input| input.viewport().close_requested()) {
                        self.settings.preview_detached = false;
                        let _ = self.settings.save(&self.root);
                    }
                },
            );
        }

        self.dialog_ui(&ctx);
        if self.transfer.is_some() || self.busy > 0 || self.status_until.is_some() {
            ctx.request_repaint_after(Duration::from_millis(100));
        } else {
            // Picks up edited style files even when nothing else happens.
            ctx.request_repaint_after(Duration::from_secs(1));
        }
    }

    fn save(&mut self, _storage: &mut dyn eframe::Storage) {
        self.save_settings();
    }
}
