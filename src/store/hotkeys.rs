//! The hotkey config file (`%APPDATA%\pashari\hotkeys.toml`).
//!
//! Bundles global hotkeys and in-app local shortcuts. Split into its own
//! file since it's different in nature from the other settings
//! (`super::Config`) — dedicated to the settings GUI's Hotkeys tab.

use std::path::PathBuf;
use std::sync::{LazyLock, Mutex};

use serde::{Deserialize, Serialize};

/// The hotkey config. Unset fields fall back to defaults.
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct HotkeyConfig {
    /// The global hotkey that starts region selection (e.g. "Ctrl+Shift+2"). Multiple allowed.
    #[serde(deserialize_with = "string_or_list")]
    pub hotkey: Vec<String>,
    /// Global hotkey that saves the whole virtual desktop as a PNG.
    #[serde(deserialize_with = "string_or_list")]
    pub hotkey_full_screenshot: Vec<String>,
    /// Global hotkey that starts recording the whole virtual desktop.
    #[serde(deserialize_with = "string_or_list")]
    pub hotkey_full_record: Vec<String>,

    // --- In-app local shortcuts (each element parsed by
    // `localkey::parse`; changeable via the settings GUI's Hotkeys tab.
    // Multiple allowed. Escape and Delete/Backspace are excluded and fixed).
    /// Undo (shared by the region-selection overlay and editor).
    #[serde(deserialize_with = "string_or_list")]
    pub hotkey_undo: Vec<String>,
    /// Redo (shared by the region-selection overlay and editor).
    #[serde(deserialize_with = "string_or_list")]
    pub hotkey_redo: Vec<String>,
    /// Reuse the last region as-is (region selection).
    #[serde(deserialize_with = "string_or_list")]
    pub hotkey_reuse_region: Vec<String>,
    /// Clear the selection to redraw it (region selection).
    #[serde(deserialize_with = "string_or_list")]
    pub hotkey_clear_selection: Vec<String>,
    /// Save as, choosing the destination via a dialog (region selection).
    #[serde(deserialize_with = "string_or_list")]
    pub hotkey_save_as: Vec<String>,
    /// Hand off to an external editor (region selection; no-op if unset).
    #[serde(deserialize_with = "string_or_list")]
    pub hotkey_edit_external: Vec<String>,
    /// End the capture (region selection; same as Esc).
    #[serde(deserialize_with = "string_or_list")]
    pub hotkey_quit: Vec<String>,
    /// Action menu: Save (region selection).
    #[serde(deserialize_with = "string_or_list")]
    pub hotkey_menu_save: Vec<String>,
    /// Action menu: Copy (region selection).
    #[serde(deserialize_with = "string_or_list")]
    pub hotkey_menu_copy: Vec<String>,
    /// Action menu: Edit (region selection).
    #[serde(deserialize_with = "string_or_list")]
    pub hotkey_menu_edit: Vec<String>,
    /// Action menu: Upload (region selection).
    #[serde(deserialize_with = "string_or_list")]
    pub hotkey_menu_upload: Vec<String>,
    /// Action menu: Record video (region selection).
    #[serde(deserialize_with = "string_or_list")]
    pub hotkey_menu_record: Vec<String>,
    /// Action menu: OCR the selection.
    #[serde(deserialize_with = "string_or_list")]
    pub hotkey_menu_ocr: Vec<String>,
    /// Reset zoom/pan to the initial view (editor).
    #[serde(deserialize_with = "string_or_list")]
    pub hotkey_editor_reset_zoom: Vec<String>,
    /// Tool switch: Select (editor). Unlike paste/save/copy, this is
    /// changeable (shown in the Hotkeys tab's Editor group).
    #[serde(deserialize_with = "string_or_list")]
    pub hotkey_editor_tool_select: Vec<String>,
    /// Tool switch: Arrow (editor).
    #[serde(deserialize_with = "string_or_list")]
    pub hotkey_editor_tool_arrow: Vec<String>,
    /// Tool switch: Polyline (editor).
    #[serde(deserialize_with = "string_or_list")]
    pub hotkey_editor_tool_polyline: Vec<String>,
    /// Tool switch: Draw (formerly Freehand, editor).
    #[serde(deserialize_with = "string_or_list")]
    pub hotkey_editor_tool_draw: Vec<String>,
    /// Tool switch: Rect (editor).
    #[serde(deserialize_with = "string_or_list")]
    pub hotkey_editor_tool_rect: Vec<String>,
    /// Tool switch: Ellipse (editor).
    #[serde(deserialize_with = "string_or_list")]
    pub hotkey_editor_tool_ellipse: Vec<String>,
    /// Tool switch: Text (editor).
    #[serde(deserialize_with = "string_or_list")]
    pub hotkey_editor_tool_text: Vec<String>,
    /// Tool switch: NumberMarker (editor).
    #[serde(deserialize_with = "string_or_list")]
    pub hotkey_editor_tool_number_marker: Vec<String>,
}

impl Default for HotkeyConfig {
    fn default() -> Self {
        Self {
            hotkey: vec!["Ctrl+Shift+2".into()],
            hotkey_full_screenshot: vec!["Ctrl+Shift+3".into()],
            hotkey_full_record: vec!["Ctrl+Shift+4".into()],
            hotkey_undo: vec!["Ctrl+Z".into()],
            hotkey_redo: vec!["Ctrl+Shift+Z".into()],
            hotkey_reuse_region: vec!["R".into()],
            hotkey_clear_selection: vec!["X".into()],
            hotkey_save_as: vec!["Shift+S".into()],
            hotkey_edit_external: vec!["Shift+E".into()],
            hotkey_quit: vec!["Q".into()],
            hotkey_menu_save: vec!["S".into()],
            hotkey_menu_copy: vec!["C".into()],
            hotkey_menu_edit: vec!["E".into()],
            hotkey_menu_upload: vec!["U".into()],
            hotkey_menu_record: vec!["V".into()],
            hotkey_menu_ocr: vec!["O".into()],
            hotkey_editor_reset_zoom: vec!["Ctrl+0".into()],
            hotkey_editor_tool_select: vec!["V".into()],
            hotkey_editor_tool_arrow: vec!["A".into()],
            hotkey_editor_tool_polyline: vec!["L".into()],
            hotkey_editor_tool_draw: vec!["D".into()],
            hotkey_editor_tool_rect: vec!["R".into()],
            hotkey_editor_tool_ellipse: vec!["C".into()],
            hotkey_editor_tool_text: vec!["T".into()],
            hotkey_editor_tool_number_marker: vec!["N".into()],
        }
    }
}

/// A backward-compatible deserializer for `hotkey*` fields, reading a
/// `hotkeys.toml` saved in either the old format (a single scalar string)
/// or the new format (an array of strings) — the former converts to a 1-element `Vec`.
fn string_or_list<'de, D>(d: D) -> Result<Vec<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OneOrMany {
        One(String),
        Many(Vec<String>),
    }
    Ok(match OneOrMany::deserialize(d)? {
        OneOrMany::One(s) => vec![s],
        OneOrMany::Many(v) => v,
    })
}

static CONFIG: LazyLock<Mutex<HotkeyConfig>> =
    LazyLock::new(|| Mutex::new(HotkeyConfig::default()));

/// Loads the config file (generates a template if missing). Called from `super::init()`.
pub(crate) fn init() {
    *CONFIG.lock().unwrap() = load_or_create();
}

/// Returns a clone of the current config (e.g. for the GUI's initial display).
pub fn snapshot() -> HotkeyConfig {
    CONFIG.lock().unwrap().clone()
}

/// Updates the config and writes it out as toml (called from the GUI's Save).
pub fn set_and_save(cfg: HotkeyConfig) {
    if let Some(path) = config_path() {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Err(e) = std::fs::write(&path, render_toml(&cfg)) {
            eprintln!("ホットキー設定の保存に失敗: {e}");
        }
    }
    *CONFIG.lock().unwrap() = cfg;
}

/// The config file's path (`%APPDATA%\pashari\hotkeys.toml`).
fn config_path() -> Option<PathBuf> {
    let appdata = std::env::var("APPDATA").ok()?;
    Some(PathBuf::from(appdata).join("pashari").join("hotkeys.toml"))
}

fn load_or_create() -> HotkeyConfig {
    let Some(path) = config_path() else {
        return HotkeyConfig::default();
    };

    if let Ok(text) = std::fs::read_to_string(&path) {
        match toml::from_str::<HotkeyConfig>(&text) {
            Ok(cfg) => return cfg,
            Err(e) => {
                eprintln!("ホットキー設定ファイルの解釈に失敗（既定を使用）: {e}");
                return HotkeyConfig::default();
            }
        }
    }

    // Writes out a template if missing (continues with defaults on failure).
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if std::fs::write(&path, render_toml(&HotkeyConfig::default())).is_ok() {
        println!("ホットキー設定ファイルを作成しました: {}", path.display());
    }
    HotkeyConfig::default()
}

/// Generates commented toml (shared by first-run generation and GUI saves).
fn render_toml(c: &HotkeyConfig) -> String {
    format!(
        r#"# pashari ホットキー設定ファイル。トレイ → Settings → Hotkeys タブの GUI から
# 変更できます（手編集も可）。変更は GUI の Save で即反映、手編集の場合は
# 再起動で反映されます。

# 領域選択を起動するグローバルホットキー（複数登録可）。
# 修飾子: Ctrl / Shift / Alt / Super(Win)。キー: 数字 / 英字 / F1..F12 / NumpadSubtract など。
# 例: ["Ctrl+Shift+2", "Alt+PrintScreen"]
hotkey = {}
hotkey_full_screenshot = {}
hotkey_full_record = {}

# アプリ内のローカルショートカット（Undo/Redo・領域選択・エディタ。複数登録可）。
# 設定GUIの Hotkeys タブから変更してください。手編集する場合は
# "R" や "Ctrl+Z" のように、Ctrl/Shift/Alt の組み合わせ＋英数字1文字で書きます。
hotkey_undo = {}
hotkey_redo = {}
hotkey_reuse_region = {}
hotkey_clear_selection = {}
hotkey_save_as = {}
hotkey_edit_external = {}
hotkey_quit = {}
hotkey_menu_save = {}
hotkey_menu_copy = {}
hotkey_menu_edit = {}
hotkey_menu_upload = {}
hotkey_menu_record = {}
hotkey_menu_ocr = {}
hotkey_editor_reset_zoom = {}
hotkey_editor_tool_select = {}
hotkey_editor_tool_arrow = {}
hotkey_editor_tool_polyline = {}
hotkey_editor_tool_draw = {}
hotkey_editor_tool_rect = {}
hotkey_editor_tool_ellipse = {}
hotkey_editor_tool_text = {}
hotkey_editor_tool_number_marker = {}
"#,
        render_string_list(&c.hotkey),
        render_string_list(&c.hotkey_full_screenshot),
        render_string_list(&c.hotkey_full_record),
        render_string_list(&c.hotkey_undo),
        render_string_list(&c.hotkey_redo),
        render_string_list(&c.hotkey_reuse_region),
        render_string_list(&c.hotkey_clear_selection),
        render_string_list(&c.hotkey_save_as),
        render_string_list(&c.hotkey_edit_external),
        render_string_list(&c.hotkey_quit),
        render_string_list(&c.hotkey_menu_save),
        render_string_list(&c.hotkey_menu_copy),
        render_string_list(&c.hotkey_menu_edit),
        render_string_list(&c.hotkey_menu_upload),
        render_string_list(&c.hotkey_menu_record),
        render_string_list(&c.hotkey_menu_ocr),
        render_string_list(&c.hotkey_editor_reset_zoom),
        render_string_list(&c.hotkey_editor_tool_select),
        render_string_list(&c.hotkey_editor_tool_arrow),
        render_string_list(&c.hotkey_editor_tool_polyline),
        render_string_list(&c.hotkey_editor_tool_draw),
        render_string_list(&c.hotkey_editor_tool_rect),
        render_string_list(&c.hotkey_editor_tool_ellipse),
        render_string_list(&c.hotkey_editor_tool_text),
        render_string_list(&c.hotkey_editor_tool_number_marker),
    )
}

/// Formats a `Vec` of strings as a toml array literal (`["a", "b"]`).
fn render_string_list(v: &[String]) -> String {
    let items: Vec<String> = v
        .iter()
        .map(|s| format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"")))
        .collect();
    format!("[{}]", items.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hotkey_fields_accept_old_scalar_string_format() {
        // Config files written by older versions (before multi-key
        // support) use scalar strings (`hotkey_undo = "Ctrl+Z"`). Verifies
        // these read as a 1-element `Vec`.
        let text = r#"
hotkey = "Ctrl+Shift+2"
hotkey_undo = "Ctrl+Z"
hotkey_redo = "Ctrl+Shift+Z"
hotkey_reuse_region = "R"
hotkey_clear_selection = "X"
hotkey_quit = "Q"
hotkey_menu_save = "S"
hotkey_menu_copy = "C"
hotkey_menu_edit = "E"
hotkey_menu_upload = "U"
hotkey_menu_record = "V"
hotkey_editor_reset_zoom = "Ctrl+0"
"#;
        let parsed: HotkeyConfig =
            toml::from_str(text).expect("旧形式のスカラー文字列でも解釈できるはず");
        assert_eq!(parsed.hotkey, vec!["Ctrl+Shift+2".to_string()]);
        assert_eq!(parsed.hotkey_undo, vec!["Ctrl+Z".to_string()]);
        assert_eq!(parsed.hotkey_editor_reset_zoom, vec!["Ctrl+0".to_string()]);
    }

    #[test]
    fn render_toml_round_trips_multiple_hotkey_bindings() {
        let cfg = HotkeyConfig {
            hotkey: vec!["Ctrl+Shift+2".into(), "F9".into()],
            hotkey_undo: vec!["Ctrl+Z".into(), "Ctrl+Shift+U".into()],
            ..HotkeyConfig::default()
        };
        let text = render_toml(&cfg);
        let parsed: HotkeyConfig = toml::from_str(&text).expect("生成した toml が解釈できるはず");
        assert_eq!(
            parsed.hotkey,
            vec!["Ctrl+Shift+2".to_string(), "F9".to_string()]
        );
        assert_eq!(
            parsed.hotkey_undo,
            vec!["Ctrl+Z".to_string(), "Ctrl+Shift+U".to_string()]
        );
    }

    #[test]
    fn render_toml_round_trips_editor_tool_shortcut_defaults() {
        let text = render_toml(&HotkeyConfig::default());
        let parsed: HotkeyConfig = toml::from_str(&text).expect("生成した toml が解釈できるはず");
        assert_eq!(parsed.hotkey_editor_tool_select, vec!["V".to_string()]);
        assert_eq!(parsed.hotkey_editor_tool_arrow, vec!["A".to_string()]);
        assert_eq!(parsed.hotkey_editor_tool_polyline, vec!["L".to_string()]);
        assert_eq!(parsed.hotkey_editor_tool_draw, vec!["D".to_string()]);
        assert_eq!(parsed.hotkey_editor_tool_rect, vec!["R".to_string()]);
        assert_eq!(parsed.hotkey_editor_tool_ellipse, vec!["C".to_string()]);
        assert_eq!(parsed.hotkey_editor_tool_text, vec!["T".to_string()]);
        assert_eq!(
            parsed.hotkey_editor_tool_number_marker,
            vec!["N".to_string()]
        );
    }

    #[test]
    fn hotkey_fields_missing_from_toml_fall_back_to_defaults() {
        // When a field is entirely absent (container-level
        // #[serde(default)]), it should take HotkeyConfig::default()'s
        // value, not Vec::default() (empty).
        let text = "";
        let parsed: HotkeyConfig = toml::from_str(text).expect("空の toml でも解釈できるはず");
        assert_eq!(parsed.hotkey, HotkeyConfig::default().hotkey);
        assert_eq!(parsed.hotkey_undo, HotkeyConfig::default().hotkey_undo);
    }
}
