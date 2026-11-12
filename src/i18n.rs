//! German and English strings for the interface.

use std::sync::atomic::{AtomicBool, Ordering};

static ENGLISH: AtomicBool = AtomicBool::new(false);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Language {
    De,
    En,
}

impl Language {
    pub fn code(self) -> &'static str {
        match self {
            Language::De => "de",
            Language::En => "en",
        }
    }

    pub fn from_code(code: &str) -> Self {
        if code == "en" { Language::En } else { Language::De }
    }
}

pub fn language() -> Language {
    if ENGLISH.load(Ordering::Relaxed) { Language::En } else { Language::De }
}

pub fn set_language(language: Language) {
    ENGLISH.store(language == Language::En, Ordering::Relaxed);
}

/// Looks up a message for the current language. Unknown ids come back unchanged.
pub fn tr(id: &str) -> String {
    let english = ENGLISH.load(Ordering::Relaxed);
    match STRINGS.binary_search_by(|(key, _, _)| key.cmp(&id)) {
        Ok(index) => {
            let (_, de, en) = STRINGS[index];
            if english { en } else { de }.to_owned()
        }
        Err(_) => id.to_owned(),
    }
}

/// Looks up a message and fills `{name}` placeholders.
pub fn tr_with(id: &str, values: &[(&str, &dyn std::fmt::Display)]) -> String {
    let mut text = tr(id);
    for (name, value) in values {
        text = text.replace(&format!("{{{name}}}"), &value.to_string());
    }
    text
}

/// `tr!("id")` or `tr!("id", name = value, ...)`.
#[macro_export]
macro_rules! tr {
    ($id:expr) => {
        $crate::i18n::tr($id)
    };
    ($id:expr, $($name:ident = $value:expr),+ $(,)?) => {
        $crate::i18n::tr_with($id, &[$((stringify!($name), &$value as &dyn std::fmt::Display)),+])
    };
}

/// Sorted by id so lookups can use binary search. A unit test checks the order.
static STRINGS: &[(&str, &str, &str)] = &[
    ("access_key", "Zugriffsschlüssel", "Access key"),
    ("access_ph", "S3 Access Key", "S3 access key"),
    ("bad_filename", "Ungültiger Dateiname: {name}", "Invalid file name: {name}"),
    ("bad_folder", "Dieser Ordner kann so nicht hochgeladen werden.", "This folder cannot be uploaded like this."),
    ("badge_binary", "Datei", "File"),
    ("badge_empty", "Leer", "Empty"),
    ("badge_image", "Bild", "Image"),
    ("bucket", "Bucket", "Bucket"),
    ("bucket_count", "Verbunden. {count} Buckets.", "Connected. {count} buckets."),
    ("bucket_empty", "Dieser Bucket ist leer.", "This bucket is empty."),
    ("bucket_ph", "optional", "optional"),
    (
        "bucket_tip",
        "Nur nötig, wenn die Rechte keine Bucket-Liste erlauben.",
        "Only needed when the credentials cannot list buckets.",
    ),
    ("buckets", "Buckets", "Buckets"),
    ("buckets_refreshed", "Buckets aktualisiert", "Buckets refreshed"),
    ("cancel", "Abbrechen", "Cancel"),
    ("cancel_download", "Abbrechen", "Cancel"),
    ("checking_files", "Prüfe vorhandene Dateien…", "Checking existing files…"),
    ("choose_download_dir", "Ordner zum Speichern wählen", "Choose a folder to save into"),
    ("choose_upload_files", "Dateien zum Hochladen auswählen", "Choose files to upload"),
    ("choose_upload_folder", "Ordner zum Hochladen auswählen", "Choose a folder to upload"),
    ("col_modified", "Geändert", "Modified"),
    ("col_name", "Name", "Name"),
    ("col_size", "Größe", "Size"),
    ("column", "Spalte {index}", "Column {index}"),
    ("connect", "Verbinden", "Connect"),
    ("connecting", "Verbinde…", "Connecting…"),
    ("connection", "Verbindung", "Connection"),
    ("copied", "Vorschau in die Zwischenablage kopiert", "Preview copied to the clipboard"),
    ("copy", "Text kopieren", "Copy text"),
    ("counts", "{folders} Ordner, {files} Dateien", "{folders} folders, {files} files"),
    ("created", "Erstellt {when}", "Created {when}"),
    ("delete", "Löschen", "Delete"),
    (
        "delete_body",
        "Diese Einträge werden unwiderruflich gelöscht:\n{names}",
        "These items will be permanently deleted:\n{names}",
    ),
    (
        "delete_cancelled",
        "Löschen abgebrochen. Bereits gelöschte Objekte bleiben gelöscht.",
        "Deletion cancelled. Objects already deleted stay deleted.",
    ),
    (
        "delete_folders",
        "\nOrdner werden zusammen mit allem Inhalt gelöscht.",
        "\nFolders are deleted together with everything inside.",
    ),
    ("delete_profile", "Entfernen", "Remove"),
    ("delete_profile_tip", "Die ausgewählte gespeicherte Verbindung löschen", "Delete the selected saved connection"),
    (
        "delete_tip",
        "Löscht die markierten Dateien und Ordner. Ein Ordner wird mit seinem Inhalt gelöscht. Mehrere mit Strg+Klick oder Umschalt+Klick auswählen.",
        "Deletes the selected files and folders. A folder is deleted with its contents. Ctrl+click or Shift+click selects more than one.",
    ),
    ("deleted_many", "{count} Objekte gelöscht", "{count} objects deleted"),
    ("deleted_one", "1 Objekt gelöscht", "1 object deleted"),
    (
        "direct_bucket_notice",
        "Die Bucket-Liste ist mit diesen Rechten nicht abrufbar. Der eingetragene Bucket wurde direkt geöffnet.",
        "The bucket list is not available with these permissions. The entered bucket was opened directly.",
    ),
    (
        "discard_body",
        "Der Text wurde geändert und noch nicht in den Bucket geschrieben. Änderungen verwerfen?",
        "The text was changed and has not been written to the bucket. Discard the changes?",
    ),
    ("discard_title", "Ungespeicherte Änderungen", "Unsaved changes"),
    ("disconnect", "Trennen", "Disconnect"),
    ("dock_left", "Vorschau links andocken", "Dock the preview on the left"),
    ("dock_right", "Vorschau rechts andocken", "Dock the preview on the right"),
    ("download", "Herunterladen", "Download"),
    ("download_cancelled", "Download abgebrochen", "Download cancelled"),
    ("download_cancelling", "Breche Download ab…", "Cancelling download…"),
    ("download_file", "Diese Datei speichern", "Save this file"),
    (
        "download_file_tip",
        "Speichert die Datei, die in der Vorschau geöffnet ist.",
        "Saves the file that is open in the preview.",
    ),
    (
        "download_tip",
        "Speichert die markierten Dateien. Mehrere mit Strg+Klick oder Umschalt+Klick auswählen. Ein markierter Ordner wird mit seinem Inhalt gespeichert.",
        "Saves the selected files. Ctrl+click or Shift+click selects more than one. A selected folder is saved with its contents.",
    ),
    ("edit", "Bearbeiten", "Edit"),
    (
        "edit_tip",
        "Text in der Vorschau bearbeiten und danach in den Bucket schreiben.",
        "Edit the text in the preview and write it back to the bucket.",
    ),
    ("empty_file", "Die Datei ist leer.", "The file is empty."),
    ("empty_filename", "Leerer Dateiname.", "Empty file name."),
    ("endpoint", "Endpunkt", "Endpoint"),
    ("endpoint_ph", "https://s3.cloud.example.com", "https://s3.cloud.example.com"),
    (
        "endpoint_tip",
        "S3-API-Adresse der OpenStack-Umgebung, inklusive https://",
        "S3 API address of the OpenStack environment, including https://",
    ),
    ("err_access_denied", "Kein Zugriff auf diese Ressource.", "Access to this resource was denied."),
    ("err_access_disabled", "Der Zugriff ist für diesen Account deaktiviert.", "Access is disabled for this account."),
    ("err_access_key", "Der Zugriffsschlüssel wurde abgelehnt.", "The access key was rejected."),
    (
        "err_argument",
        "Der Endpunkt hat die Anfrage abgelehnt. Bei OpenStack Pfad-Stil und Signaturversion prüfen.",
        "The endpoint rejected the request. For OpenStack, check path style and signature version.",
    ),
    (
        "err_auth_header",
        "Die Anmeldung wurde abgelehnt. Oft stimmt die Region nicht.",
        "Sign-in was rejected. The region is often wrong.",
    ),
    ("err_bad_bucket", "Der Bucket-Name ist ungültig.", "The bucket name is invalid."),
    (
        "err_bucket_name",
        "Der Bucket-Name darf keine Leerzeichen oder Schrägstriche enthalten.",
        "The bucket name must not contain spaces or slashes.",
    ),
    ("err_clock", "Die Uhrzeit dieses Rechners weicht zu stark ab.", "This computer's clock is too far off."),
    ("err_endpoint_missing", "Der Endpunkt fehlt.", "The endpoint is missing."),
    (
        "err_endpoint_url",
        "Der Endpunkt muss eine http- oder https-URL sein.",
        "The endpoint must be an http or https URL.",
    ),
    ("err_failed", "Die Anfrage ist fehlgeschlagen ({code}).", "The request failed ({code})."),
    ("err_forbidden", "Kein Zugriff ({code}).", "Access denied ({code})."),
    (
        "err_keys_required",
        "Zugriffsschlüssel und geheimer Schlüssel werden benötigt.",
        "Access key and secret key are required.",
    ),
    ("err_no_bucket", "Der Bucket wurde nicht gefunden.", "The bucket was not found."),
    ("err_no_key", "Die Datei wurde nicht gefunden.", "The file was not found."),
    (
        "err_not_implemented",
        "Der Endpunkt unterstützt diese Operation nicht ({code}).",
        "The endpoint does not support this operation ({code}).",
    ),
    (
        "err_redirect",
        "Der Endpunkt leitet woanders hin. Prüfe Region und Endpunkt-URL.",
        "The endpoint redirects elsewhere. Check the region and endpoint URL.",
    ),
    (
        "err_sig_mismatch",
        "Die Signatur passt nicht. Prüfe den geheimen Schlüssel, die Region und die Signaturversion (s3v4 oder s3).",
        "The signature does not match. Check the secret key, the region and the signature version (s3v4 or s3).",
    ),
    ("err_timeout", "Der Endpunkt hat nicht rechtzeitig geantwortet.", "The endpoint did not respond in time."),
    (
        "err_tls",
        "TLS-Zertifikat konnte nicht geprüft werden. Bei einem selbstsignierten Zertifikat die Option „TLS-Zertifikat prüfen“ ausschalten.",
        "The TLS certificate could not be verified. For a self-signed certificate, turn off “Verify TLS certificate”.",
    ),
    (
        "err_unreachable",
        "Der Endpunkt ist nicht erreichbar. Prüfe die URL, den Port und das Netzwerk.",
        "The endpoint is unreachable. Check the URL, the port and the network.",
    ),
    ("err_with_code", "{message} ({code})", "{message} ({code})"),
    ("fallback_profile", "Verbindung", "Connection"),
    ("files", "Dateien", "Files"),
    (
        "files_placeholder",
        "Nach der Verbindung links einen Bucket wählen.\nOrdner öffnen sich per Doppelklick. Textdateien, CSV, JSON, YAML, XML,\nBilder und GeoTIFFs erscheinen in der Dateivorschau.",
        "After connecting, choose a bucket on the left.\nFolders open with a double-click. Text files, CSV, JSON, YAML, XML,\nimages and GeoTIFFs appear in the preview.",
    ),
    ("filter_buckets", "Bucket-Name filtern", "Filter bucket name"),
    (
        "filter_buckets_tip",
        "Zeigt nur Buckets, deren Name diesen Text enthält.",
        "Shows only buckets whose name contains this text.",
    ),
    ("filter_files", "Dateiname filtern", "Filter file name"),
    ("filter_files_regex", "Regulärer Ausdruck, z. B. \\.tiff?$", "Regular expression, e.g. \\.tiff?$"),
    (
        "filter_files_tip",
        "Zeigt nur Dateien und Ordner, deren Name diesen Text enthält.",
        "Shows only files and folders whose name contains this text.",
    ),
    ("filter_regex_invalid", "Ungültiger Ausdruck", "Invalid expression"),
    (
        "filter_regex_tip",
        "Zeigt nur Dateien und Ordner, deren Name zum regulären Ausdruck passt. ^ und $ binden an Anfang und Ende, z. B. ^tile_\\d+\\.png$ oder 2024|2025.",
        "Shows only files and folders whose name matches the regular expression. ^ and $ anchor to start and end, e.g. ^tile_\\d+\\.png$ or 2024|2025.",
    ),
    (
        "filter_regex_toggle",
        "Filter als regulären Ausdruck verwenden (Groß-/Kleinschreibung egal)",
        "Use the filter as a regular expression (case-insensitive)",
    ),
    ("fold_collapse", "Verbindung nach oben einklappen", "Collapse the connection panel"),
    ("fold_expand", "Verbindung ausklappen", "Expand the connection panel"),
    ("folder_empty", "Dieser Ordner ist leer.", "This folder is empty."),
    ("folder_empty_local", "Der Ordner enthält keine Dateien.", "The folder contains no files."),
    ("folder_failed", "Ordner konnte nicht geladen werden", "Could not load the folder"),
    ("geo_band", "Kanal", "Band"),
    ("geo_bands", "Kanäle", "Bands"),
    ("geo_bounds", "Ausdehnung", "Extent"),
    ("geo_center", "Mitte (WGS84)", "Center (WGS84)"),
    ("geo_colormap", "Farbskala", "Colour map"),
    ("geo_compression", "Kompression", "Compression"),
    ("geo_crs", "Koordinatensystem", "Coordinate system"),
    ("geo_dtype", "Datentyp", "Data type"),
    ("geo_fetched", "Geladen", "Fetched"),
    ("geo_info", "GeoTIFF-Details", "GeoTIFF details"),
    ("geo_layout", "Aufbau", "Layout"),
    ("geo_level", "Vorschau aus", "Preview from"),
    ("geo_mode_gray", "Einzelkanal", "Single band"),
    ("geo_nodata", "NoData", "NoData"),
    ("geo_open_map", "Auf Karte zeigen", "Show on map"),
    (
        "geo_open_map_tip",
        "Öffnet die Bildmitte in OpenStreetMap im Browser.",
        "Opens the image centre in OpenStreetMap in the browser.",
    ),
    ("geo_origin", "Ursprung", "Origin"),
    ("geo_overviews", "Übersichten", "Overviews"),
    ("geo_overviews_none", "keine", "none"),
    ("geo_pixel", "Pixel", "Pixel"),
    ("geo_pixel_size", "Pixelgröße", "Pixel size"),
    ("geo_size", "Größe", "Size"),
    ("geo_stretch", "Streckung", "Stretch"),
    ("geo_stretch_minmax", "Min–Max", "Min–max"),
    ("geo_stretch_none", "Keine", "None"),
    ("geo_stretch_percent", "2–98 %", "2–98 %"),
    ("geo_stripped", "Streifen, {rows} Zeilen", "Strips, {rows} rows"),
    ("geo_tiled", "Kacheln {width}×{height}", "Tiles {width}×{height}"),
    ("geo_value", "Wert", "Value"),
    (
        "geo_zoom_hint",
        "Mausrad zoomt, Ziehen verschiebt, Doppelklick passt ein.",
        "Scroll to zoom, drag to pan, double-click to fit.",
    ),
    ("hide_secret", "Verbergen", "Hide"),
    (
        "hint",
        "Der Endpunkt ist die S3-API-Adresse, nicht die Keystone-URL. Zugangsdaten sind die S3-Schlüssel (EC2 Credentials), nicht das Login-Passwort. Der geheime Schlüssel wird nicht gespeichert.",
        "The endpoint is the S3 API address, not the Keystone URL. Credentials are the S3 keys (EC2 credentials), not the login password. The secret key is not stored unless you save a named connection.",
    ),
    (
        "image_failed",
        "Das Bild konnte nicht gelesen werden. Die Datei kann heruntergeladen werden.",
        "The image could not be read. The file can be downloaded.",
    ),
    ("image_info", "Bild {width}×{height}", "Image {width}×{height}"),
    (
        "image_too_big",
        "Das Bild ist größer als {limit} MB und wird nicht als Vorschau geladen. Die Datei kann heruntergeladen werden.",
        "The image is larger than {limit} MB and is not loaded as a preview. The file can be downloaded.",
    ),
    (
        "json_raw",
        "JSON konnte nicht formatiert werden und wird roh angezeigt.",
        "JSON could not be formatted and is shown raw.",
    ),
    ("language", "Sprache: Deutsch. Klick schaltet auf Englisch.", "Language: English. Click switches to German."),
    ("list_capped", ". Anzeige auf {limit} Einträge begrenzt.", ". Showing at most {limit} entries."),
    ("loading_folder", "Lade Ordner…", "Loading folder…"),
    ("loading_preview", "Lade Vorschau…", "Loading preview…"),
    ("new_folder", "Neuer Ordner", "New folder"),
    ("new_folder_created", "Ordner „{name}“ angelegt", "Created folder “{name}”"),
    ("new_folder_exists", "Der Ordner „{name}“ ist schon da.", "The folder “{name}” already exists."),
    (
        "new_folder_invalid",
        "Der Ordnername darf nicht leer sein und keine Schrägstriche enthalten.",
        "The folder name must not be empty or contain slashes.",
    ),
    ("new_folder_label", "Name", "Name"),
    (
        "new_folder_tip",
        "Legt im geöffneten Bucket und Ordner einen neuen Ordner an.",
        "Creates a new folder in the open bucket and folder.",
    ),
    ("no", "Nein", "No"),
    (
        "no_bucket_list",
        "Die Verbindung steht, aber es wurden keine Buckets gelistet. Trag den Bucket-Namen ein und verbinde erneut, falls die Liste gesperrt ist.",
        "Connected, but no buckets were listed. Enter the bucket name and connect again if listing is denied.",
    ),
    ("no_buckets", "Verbunden. Keine Buckets sichtbar.", "Connected. No buckets visible."),
    ("no_files_saved", "Keine Dateien gespeichert", "No files saved"),
    ("no_files_to_save", "Keine Dateien zum Speichern.", "No files to save."),
    ("no_preview", "Keine Vorschau.", "No preview."),
    (
        "no_text_preview",
        "Für diesen Dateityp gibt es keine Textvorschau. Die Datei kann heruntergeladen werden.",
        "There is no text preview for this file type. The file can be downloaded.",
    ),
    ("no_tiff", "Keine TIFF-Vorschau.", "No TIFF preview."),
    ("not_a_folder", "Das ist kein Ordner.", "That is not a folder."),
    ("not_connected", "Nicht verbunden", "Not connected"),
    ("noun_file", "Datei", "file"),
    ("noun_files", "Dateien", "files"),
    ("ok", "OK", "OK"),
    ("open_folder", "Doppelklick öffnet den Ordner.", "Double-click opens the folder."),
    ("parent_folder", "Übergeordneter Ordner", "Parent folder"),
    ("path_style", "Pfad-Stil", "Path style"),
    (
        "path_tip",
        "Für OpenStack aktiv lassen. Der Bucket steht dann im URL-Pfad und nicht in der Domain.",
        "Leave this on for OpenStack. The bucket is then in the URL path, not the domain.",
    ),
    ("pop_in", "Vorschau ins Hauptfenster zurückholen", "Put the preview back into the main window"),
    ("pop_out", "Vorschau in eigenem Fenster öffnen", "Open the preview in its own window"),
    ("preview", "Dateivorschau", "Preview"),
    (
        "preview_idle",
        "Wähle eine Textdatei, CSV, JSON, YAML, XML, ein Bild oder ein GeoTIFF aus.\nDer Inhalt erscheint hier.",
        "Choose a text file, CSV, JSON, YAML, XML, an image or a GeoTIFF.\nThe contents appear here.",
    ),
    ("preview_named", "Dateivorschau — {name}", "Preview — {name}"),
    (
        "preview_truncated",
        "Die Vorschau zeigt nur den Anfang der Datei (1 MB).",
        "The preview shows only the start of the file (1 MB).",
    ),
    (
        "profile_bad_name",
        "Der Name darf keine Schrägstriche enthalten und höchstens 80 Zeichen lang sein.",
        "The name must not contain slashes and can be at most 80 characters.",
    ),
    ("profile_incomplete", "Die Verbindung „{name}“ ist unvollständig.", "The connection “{name}” is incomplete."),
    ("profile_missing", "Die Verbindung „{name}“ ist nicht gespeichert.", "The connection “{name}” is not saved."),
    ("profile_needs_name", "Die Verbindung braucht einen Namen.", "The connection needs a name."),
    ("profile_tip", "Gespeicherte Verbindung aus {path} laden", "Load a saved connection from {path}"),
    (
        "profile_unreadable",
        "Die gespeicherten Verbindungen konnten nicht gelesen werden.",
        "The saved connections could not be read.",
    ),
    ("refresh", "Aktualisieren (F5)", "Refresh (F5)"),
    ("region", "Region", "Region"),
    (
        "region_tip",
        "Region aus der Liste wählen oder eintragen. Der Name geht in die Signatur ein. Bei OpenStack oft RegionOne, sonst häufig us-east-1.",
        "Choose a region from the list or type one. The name is part of the signature. On OpenStack often RegionOne, otherwise usually us-east-1.",
    ),
    ("remove_connection_body", "„{name}“ aus {path} entfernen?", "Remove “{name}” from {path}?"),
    ("remove_connection_title", "Verbindung entfernen", "Remove connection"),
    ("removed_connection", "„{name}“ entfernt", "Removed “{name}”"),
    (
        "replace_body",
        "Diese Dateien gibt es schon und sie werden ersetzt:\n{names}",
        "These files already exist and will be replaced:\n{names}",
    ),
    (
        "replace_connection_body",
        "„{name}“ ist schon gespeichert. Überschreiben?",
        "“{name}” is already saved. Overwrite it?",
    ),
    ("replace_connection_title", "Verbindung ersetzen", "Replace connection"),
    ("replace_more", "\n… und {count} weitere", "\n… and {count} more"),
    ("replace_title", "Dateien ersetzen", "Replace files"),
    ("save_connection_label", "Name", "Name"),
    ("save_connection_title", "Verbindung speichern", "Save connection"),
    ("save_edits", "Änderungen speichern", "Save changes"),
    (
        "save_edits_tip",
        "Schreibt den geänderten Text zurück in den Bucket und ersetzt die Datei.",
        "Writes the edited text back to the bucket and replaces the file.",
    ),
    ("save_one", "Datei speichern", "Save file"),
    ("save_profile", "Speichern", "Save"),
    (
        "save_profile_tip",
        "Endpunkt und Zugangsdaten unter einem Namen in {path} speichern",
        "Save endpoint and credentials under a name in {path}",
    ),
    ("saved_at", "Gespeichert: {path}", "Saved: {path}"),
    ("saved_capped", ". Es wurden höchstens {limit} Dateien berücksichtigt.", ". At most {limit} files were included."),
    ("saved_connection", "Gespeicherte Verbindung", "Saved connection"),
    ("saved_connection_status", "Verbindung gespeichert in {path}", "Connection saved in {path}"),
    ("saved_many", "{count} Dateien gespeichert in {directory}", "{count} files saved in {directory}"),
    ("saved_one", "1 Datei gespeichert in {directory}", "1 file saved in {directory}"),
    ("saving", "Speichere {name}…", "Saving {name}…"),
    ("saving_files", "Speichere Dateien…", "Saving files…"),
    ("secret_key", "Geheimer Schlüssel", "Secret key"),
    ("secret_ph", "S3 Secret Key", "S3 secret key"),
    (
        "secret_tip",
        "Wird nur im Speicher gehalten und nicht gespeichert.",
        "Kept in memory only and not saved, unless you save a named connection.",
    ),
    ("settings_menu", "Einstellungen", "Settings"),
    ("show_secret", "Zeigen", "Show"),
    ("signature", "Signatur", "Signature"),
    (
        "signature_tip",
        "Ältere OpenStack-Installationen brauchen manchmal SigV2.",
        "Older OpenStack installations sometimes need SigV2.",
    ),
    ("stop_editing", "Bearbeitung beenden", "Stop editing"),
    ("summary_empty", "Endpunkt und Zugangsdaten", "Endpoint and credentials"),
    ("table_limited", "Die Tabelle zeigt die ersten 2000 Zeilen.", "The table shows the first 2000 rows."),
    (
        "table_too_wide",
        "Die Datei wird als Text gezeigt, weil sie zu viele Spalten für die Tabelle hat.",
        "The file is shown as text because it has too many columns for the table.",
    ),
    ("theme", "Stil", "Style"),
    ("theme_export", "Aktuellen Stil als Vorlage speichern", "Save the current style as a template"),
    (
        "theme_exported",
        "Vorlage gespeichert: {path}. Datei bearbeiten, die Änderungen erscheinen sofort.",
        "Template saved: {path}. Edit the file, changes appear right away.",
    ),
    ("theme_folder", "Stil-Ordner öffnen", "Open the styles folder"),
    (
        "theme_load_failed",
        "Stil „{name}“ konnte nicht geladen werden: {error}",
        "Style “{name}” could not be loaded: {error}",
    ),
    (
        "theme_tip",
        "Erscheinungsbild wählen. Eigene Stile liegen als TOML-Dateien in {path}.",
        "Choose the appearance. Your own styles are TOML files in {path}.",
    ),
    (
        "tiff_compression",
        "Die TIFF-Kompression „{name}“ wird für die Vorschau nicht unterstützt.",
        "The TIFF compression “{name}” is not supported for previews.",
    ),
    (
        "tiff_failed",
        "Die TIFF-Vorschau konnte nicht erzeugt werden. Die Datei kann heruntergeladen werden.",
        "The TIFF preview could not be created. The file can be downloaded.",
    ),
    ("tiff_full", "Vorschau aus voller Auflösung.", "Preview from full resolution."),
    ("tiff_load_all", "Vollständig laden ({size})", "Load completely ({size})"),
    (
        "tiff_load_all_tip",
        "Lädt alle Blöcke dieser Auflösungsstufe für eine genaue Vorschau.",
        "Loads every block of this resolution level for an exact preview.",
    ),
    (
        "tiff_overview",
        "Vorschau aus Übersicht {level} ({width}×{height}).",
        "Preview from overview {level} ({width}×{height}).",
    ),
    ("tiff_page", " Seite 1 von {pages}.", " Page 1 of {pages}."),
    (
        "tiff_sampled",
        "Die Datei hat keine Übersichten. Die Vorschau ist aus {used} von {total} Blöcken zusammengesetzt und daher grob. Mit Übersichten (gdaladdo, COG) wird sie genau und schnell.",
        "The file has no overviews. The preview is assembled from {used} of {total} blocks and is therefore coarse. With overviews (gdaladdo, COG) it becomes exact and fast.",
    ),
    ("transfer_delete", "Löschen", "Delete"),
    ("transfer_download", "Download", "Download"),
    ("transfer_upload", "Upload", "Upload"),
    ("up", "Nach oben (Rücktaste)", "Up (Backspace)"),
    ("update_available", "Version {version} verfügbar", "Version {version} available"),
    (
        "update_available_tip",
        "Auf GitHub ist eine neuere Version erschienen (installiert: {current}). Klick öffnet die Download-Seite.",
        "A newer version was released on GitHub (installed: {current}). Click to open the download page.",
    ),
    ("update_check_now", "Jetzt prüfen", "Check now"),
    ("update_check_setting", "Beim Start nach neuer Version suchen", "Check for a new version at start"),
    (
        "update_check_tip",
        "Fragt höchstens einmal am Tag bei GitHub nach der neuesten Version. Es wird nichts heruntergeladen oder installiert. Ohne Verbindung zu GitHub arbeitet das Programm ganz normal.",
        "Asks GitHub for the latest version at most once a day. Nothing is downloaded or installed. Without access to GitHub the program works as usual.",
    ),
    ("update_checking", "Prüfe…", "Checking…"),
    ("update_current", "Installiert: Version {version}", "Installed: version {version}"),
    ("update_disabled_env", "Durch S3VIEWER_UPDATE_CHECK=0 abgeschaltet.", "Turned off by S3VIEWER_UPDATE_CHECK=0."),
    ("update_failed", "GitHub nicht erreichbar: {error}", "Could not reach GitHub: {error}"),
    ("update_latest", "Das ist die neueste Version.", "This is the latest version."),
    ("upload", "Hochladen", "Upload"),
    ("upload_bad_path", "„{path}“ kann so nicht hochgeladen werden.", "“{path}” cannot be uploaded like this."),
    ("upload_cancelled", "Upload abgebrochen", "Upload cancelled"),
    ("upload_duplicate", "Mehrere ausgewählte Dateien heißen „{name}“.", "Several selected files are named “{name}”."),
    ("upload_folder", "Ordner hochladen", "Upload folder"),
    (
        "upload_folder_tip",
        "Lädt einen Ordner mit allen Unterordnern in den geöffneten Bucket und Ordner hoch.",
        "Uploads a folder and its subfolders into the open bucket and folder.",
    ),
    ("upload_none", "Keine Dateien ausgewählt.", "No files selected."),
    (
        "upload_tip",
        "Lädt Dateien in den geöffneten Bucket und Ordner hoch.",
        "Uploads files into the open bucket and folder.",
    ),
    ("uploaded_many", "{count} Dateien hochgeladen nach {destination}", "{count} files uploaded to {destination}"),
    ("uploaded_one", "1 Datei hochgeladen nach {destination}", "1 file uploaded to {destination}"),
    ("uploading", "Lade {count} {noun} nach {destination} hoch…", "Uploading {count} {noun} to {destination}…"),
    (
        "verify_tip",
        "Ausschalten, wenn der Endpunkt ein selbstsigniertes Zertifikat verwendet.",
        "Turn off when the endpoint uses a self-signed certificate.",
    ),
    ("verify_tls", "TLS-Zertifikat prüfen", "Verify TLS certificate"),
    ("writing", "Schreibe {name}…", "Writing {name}…"),
    ("written", "In den Bucket geschrieben: {key}", "Written to the bucket: {key}"),
    (
        "xml_raw",
        "XML konnte nicht formatiert werden und wird roh angezeigt.",
        "XML could not be formatted and is shown raw.",
    ),
    ("yes", "Ja", "Yes"),
];

#[cfg(test)]
pub(crate) static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn table_is_sorted_and_unique() {
        for pair in STRINGS.windows(2) {
            assert!(pair[0].0 < pair[1].0, "{} must come before {}", pair[1].0, pair[0].0);
        }
    }

    #[test]
    fn switches_language_and_fills_values() {
        let _guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        set_language(Language::En);
        assert_eq!(tr!("counts", folders = 2, files = 3), "2 folders, 3 files");
        set_language(Language::De);
        assert_eq!(tr!("counts", folders = 2, files = 3), "2 Ordner, 3 Dateien");
        assert_eq!(tr("missing_id"), "missing_id");
    }
}
