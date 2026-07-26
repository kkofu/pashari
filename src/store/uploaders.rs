//! The custom uploader config file (`%APPDATA%\pashari\uploaders.toml`).
//!
//! Kept separate from the other settings (`super::Config`) since it's a
//! variable-length list containing secret tokens.

use std::path::PathBuf;
use std::sync::{LazyLock, Mutex};

use serde::{Deserialize, Serialize};

/// A single custom uploader profile (a generic HTTP-callback uploader:
/// sends via multipart/form-data and extracts one key from the JSON
/// response — dot-separated for nesting — as the result URL). Multiple
/// profiles can be enabled at once; Upload sends to every `enabled` one.
///
/// `#[serde(default)]`: so loading an old config file (a saved profile
/// missing the `enabled` field) fills any missing item with
/// `Default::default()` (`false` for `enabled`).
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct UploaderProfile {
    /// Display name (shown in the Upload tab's list).
    pub name: String,
    /// The POST destination URL.
    pub url: String,
    /// The multipart field name carrying the image.
    pub file_field: String,
    /// The multipart field name carrying the token. If empty, the token field isn't sent.
    pub token_field: String,
    /// The token itself (a secret value).
    pub token: String,
    /// The key used to extract the result URL from the JSON response
    /// (dot-separated for nesting, e.g. "data.link").
    pub response_field: String,
    /// If true, this profile is also sent to on Upload (multiple can be enabled at once).
    pub enabled: bool,
}

/// The list of registered custom uploaders (edited in the settings GUI's
/// Upload tab). Upload sends to every `enabled` one (multiple can be enabled at once).
#[derive(Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct UploaderConfig {
    pub uploaders: Vec<UploaderProfile>,
}

impl Default for UploaderConfig {
    fn default() -> Self {
        Self {
            uploaders: vec![UploaderProfile {
                name: "Gyazo".into(),
                url: "https://upload.gyazo.com/api/upload".into(),
                file_field: "imagedata".into(),
                token_field: "access_token".into(),
                token: String::new(),
                response_field: "permalink_url".into(),
                enabled: true,
            }],
        }
    }
}

static CONFIG: LazyLock<Mutex<UploaderConfig>> =
    LazyLock::new(|| Mutex::new(UploaderConfig::default()));

/// Loads the config file (generates a template if missing). Called from `super::init()`.
pub(crate) fn init() {
    *CONFIG.lock().unwrap() = load_or_create();
}

/// Returns a clone of the current config (e.g. for the GUI's initial display).
pub fn snapshot() -> UploaderConfig {
    CONFIG.lock().unwrap().clone()
}

/// Updates the config and writes it out as toml (called from the GUI's Save).
pub fn set_and_save(cfg: UploaderConfig) {
    if let Some(path) = config_path() {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Err(e) = std::fs::write(&path, render_toml(&cfg)) {
            eprintln!("アップローダー設定の保存に失敗: {e}");
        }
    }
    *CONFIG.lock().unwrap() = cfg;
}

/// The config file's path (`%APPDATA%\pashari\uploaders.toml`).
fn config_path() -> Option<PathBuf> {
    let appdata = std::env::var("APPDATA").ok()?;
    Some(
        PathBuf::from(appdata)
            .join("pashari")
            .join("uploaders.toml"),
    )
}

fn load_or_create() -> UploaderConfig {
    let Some(path) = config_path() else {
        return UploaderConfig::default();
    };

    if let Ok(text) = std::fs::read_to_string(&path) {
        match toml::from_str::<UploaderConfig>(&text) {
            Ok(cfg) => return cfg,
            Err(e) => {
                eprintln!("アップローダー設定ファイルの解釈に失敗（既定を使用）: {e}");
                return UploaderConfig::default();
            }
        }
    }

    // Writes out a template if missing (continues with defaults on failure).
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if std::fs::write(&path, render_toml(&UploaderConfig::default())).is_ok() {
        println!(
            "アップローダー設定ファイルを作成しました: {}",
            path.display()
        );
    }
    UploaderConfig::default()
}

/// Generates commented toml (shared by first-run generation and GUI saves).
fn render_toml(c: &UploaderConfig) -> String {
    render_uploaders_toml(&c.uploaders)
}

/// Serializes `uploaders` as an `[[uploaders]]` array-of-tables.
fn render_uploaders_toml(uploaders: &[UploaderProfile]) -> String {
    if uploaders.is_empty() {
        // Writes an explicit empty array. Omitting it entirely would
        // drop the `uploaders` key, and the container-level
        // `#[serde(default)]` would then fill it back in with
        // `UploaderConfig::default()`'s uploaders (the seeded profile).
        return "# pashari アップローダー設定ファイル。トレイ → Settings → Upload タブの\n\
                # GUI から変更できます（手編集も可）。\nuploaders = []\n"
            .to_string();
    }
    let mut out = String::from(
        "# pashari アップローダー設定ファイル。トレイ → Settings → Upload タブの GUI から\n\
         # 変更できます（手編集も可）。\n\
         #\n\
         # カスタムアップローダー（任意の HTTP エンドポイントへ画像を送るしくみ）。\n\
         # 画像は multipart/form-data で file_field 名のフィールドに乗せて送る。\n\
         # token_field が空でなければ token をそのフィールド名で追加する。\n\
         # レスポンスJSONの response_field（ドット区切りでネスト可）から結果URLを取り出す。\n\
         # enabled な（複数可）プロファイルすべてへ Upload 時に同時送信する。\n",
    );
    for u in uploaders {
        out.push_str(&format!(
            "\n[[uploaders]]\nname = \"{}\"\nurl = '{}'\nfile_field = \"{}\"\ntoken_field = \"{}\"\ntoken = '{}'\nresponse_field = \"{}\"\nenabled = {}\n",
            u.name, u.url, u.file_field, u.token_field, u.token, u.response_field, u.enabled
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_toml_round_trips_default_uploaders() {
        let original = UploaderConfig::default();
        let text = render_toml(&original);
        let parsed: UploaderConfig = toml::from_str(&text).expect("生成した toml が解釈できるはず");

        assert_eq!(parsed.uploaders.len(), original.uploaders.len());
        for (a, b) in parsed.uploaders.iter().zip(original.uploaders.iter()) {
            assert_eq!(a.name, b.name);
            assert_eq!(a.url, b.url);
            assert_eq!(a.file_field, b.file_field);
            assert_eq!(a.token_field, b.token_field);
            assert_eq!(a.token, b.token);
            assert_eq!(a.response_field, b.response_field);
            assert_eq!(a.enabled, b.enabled);
        }
    }

    #[test]
    fn render_toml_round_trips_multiple_uploaders() {
        let mut cfg = UploaderConfig::default();
        cfg.uploaders.push(UploaderProfile {
            name: "My S3".into(),
            url: "https://example.test/upload".into(),
            file_field: "file".into(),
            token_field: String::new(),
            token: String::new(),
            response_field: "data.link".into(),
            enabled: true,
        });
        let text = render_toml(&cfg);
        let parsed: UploaderConfig = toml::from_str(&text).expect("生成した toml が解釈できるはず");

        assert_eq!(parsed.uploaders.len(), 2);
        assert_eq!(parsed.uploaders[1].name, "My S3");
        assert_eq!(parsed.uploaders[1].response_field, "data.link");
        assert_eq!(parsed.uploaders[1].token_field, "");
        assert!(parsed.uploaders[0].enabled); // the seeded Gyazo profile
        assert!(parsed.uploaders[1].enabled); // the added second profile
    }

    #[test]
    fn render_toml_defaults_enabled_to_false_when_field_is_missing() {
        // Backward compatibility when loading an old config file (a saved profile missing `enabled`).
        let text = r#"
[[uploaders]]
name = "Old profile"
url = 'https://example.test/upload'
file_field = "file"
token_field = ""
token = ""
response_field = "url"
"#;
        let parsed: UploaderConfig = toml::from_str(text).expect("enabled 無しでも解釈できるはず");
        assert_eq!(parsed.uploaders.len(), 1);
        assert!(!parsed.uploaders[0].enabled);
    }

    #[test]
    fn render_toml_handles_zero_uploaders() {
        let mut cfg = UploaderConfig::default();
        cfg.uploaders.clear();
        let text = render_toml(&cfg);
        let parsed: UploaderConfig = toml::from_str(&text).expect("生成した toml が解釈できるはず");
        assert!(parsed.uploaders.is_empty());
    }
}
