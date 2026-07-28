//! exe に `assets/icon.ico` とバージョン情報（Cargo.toml の description 等）を
//! 埋め込む。Windows ターゲットのときだけ実行する（`CARGO_CFG_TARGET_OS` で判定。
//! winresource の README 推奨どおり、クロスコンパイル時も正しく判定できる）。
//!
//! Settings の About タブ（`src/settings/about.rs`）用に、直接依存の
//! crate 一覧（名前・バージョン・ライセンス）も生成する。手書きの
//! ハードコードだと `Cargo.toml` の更新時にズレるため。

use std::collections::HashMap;

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        winresource::WindowsResource::new()
            .set_icon("assets/icon.ico")
            .compile()
            .expect("exe リソースの埋め込みに失敗");
    }
    generate_used_crates();
}

/// `Cargo.toml` の `[dependencies]` から名前順の `(name, version, license)`
/// 一覧を生成し、`OUT_DIR/used_crates.rs` に書き出す（`about.rs` が
/// `include!` する）。`version` はバージョン要求文字列（例 `"0.30"`）を
/// そのまま使う（実際に解決されたバージョンではなく、`Cargo.toml` に
/// 書かれている要求のみ）。`license` は `cargo metadata` の解決結果
/// （`Cargo.lock` 経由）から取る——`Cargo.toml`/`Cargo.lock` 自体には
/// ライセンス情報が無いため。
fn generate_used_crates() {
    println!("cargo:rerun-if-changed=Cargo.toml");
    println!("cargo:rerun-if-changed=Cargo.lock");

    let manifest = std::fs::read_to_string("Cargo.toml").expect("Cargo.toml の読み込みに失敗");
    let doc: toml::Value = manifest.parse().expect("Cargo.toml の解析に失敗");
    let deps = doc
        .get("dependencies")
        .and_then(toml::Value::as_table)
        .expect("Cargo.toml に [dependencies] がありません");

    let mut names: Vec<String> = deps.keys().cloned().collect();
    names.sort();
    let licenses = fetch_licenses(&names);

    let mut entries: Vec<(String, String, String)> = deps
        .iter()
        .map(|(name, value)| {
            let version = match value {
                toml::Value::String(s) => s.clone(),
                toml::Value::Table(t) => t
                    .get("version")
                    .and_then(toml::Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                _ => String::new(),
            };
            let license = licenses
                .get(name)
                .cloned()
                .unwrap_or_else(|| "?".to_string());
            (name.clone(), version, license)
        })
        .collect();
    entries.sort();

    let mut out = String::from("pub(crate) const USED_CRATES: &[(&str, &str, &str)] = &[\n");
    for (name, version, license) in &entries {
        out.push_str(&format!("    ({name:?}, {version:?}, {license:?}),\n"));
    }
    out.push_str("];\n");

    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR が未設定");
    let dest = std::path::Path::new(&out_dir).join("used_crates.rs");
    std::fs::write(&dest, out).expect("used_crates.rs の書き込みに失敗");
}

/// `cargo metadata`（Cargo組み込みのサブコマンド、追加ツール不要）を
/// 呼んで、`names` に含まれる各パッケージのライセンス文字列を集める。
/// 同名パッケージが複数解決されている場合（推移依存の別バージョンなど）
/// は最初に見つかったものを採用する簡略化——このプロジェクトの直接依存
/// （現状21個）の範囲では実質問題にならない。
fn fetch_licenses(names: &[String]) -> HashMap<String, String> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let output = std::process::Command::new(cargo)
        .args(["metadata", "--format-version=1", "--locked"])
        .output()
        .expect("cargo metadata の実行に失敗");
    if !output.status.success() {
        panic!(
            "cargo metadata が失敗: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let json: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("cargo metadata の出力の解析に失敗");
    let packages = json
        .get("packages")
        .and_then(serde_json::Value::as_array)
        .expect("cargo metadata の出力に packages がありません");

    let mut map = HashMap::new();
    for pkg in packages {
        let Some(name) = pkg.get("name").and_then(serde_json::Value::as_str) else {
            continue;
        };
        if map.contains_key(name) || !names.iter().any(|n| n == name) {
            continue;
        }
        let license = pkg
            .get("license")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string)
            .or_else(|| {
                pkg.get("license_file")
                    .and_then(serde_json::Value::as_str)
                    .map(|_| "see LICENSE file".to_string())
            })
            .unwrap_or_else(|| "?".to_string());
        map.insert(name.to_string(), license);
    }
    map
}
