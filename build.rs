//! exe に `assets/icon.ico` とバージョン情報（Cargo.toml の description 等）を
//! 埋め込む。Windows ターゲットのときだけ実行する（`CARGO_CFG_TARGET_OS` で判定。
//! winresource の README 推奨どおり、クロスコンパイル時も正しく判定できる）。
//!
//! Settings の About タブ（`src/settings/about.rs`）用に、直接依存の
//! crate 一覧（名前・バージョン）も `Cargo.toml` から生成する。手書きの
//! ハードコードだと `Cargo.toml` の更新時にズレるため。

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        winresource::WindowsResource::new()
            .set_icon("assets/icon.ico")
            .compile()
            .expect("exe リソースの埋め込みに失敗");
    }
    generate_used_crates();
}

/// `Cargo.toml` の `[dependencies]` から名前順の `(name, version)` 一覧を
/// 生成し、`OUT_DIR/used_crates.rs` に書き出す（`about.rs` が `include!`
/// する）。`version` はバージョン要求文字列（例 `"0.30"`）をそのまま使う
/// （実際に解決されたバージョンではなく、`Cargo.toml` に書かれている
/// 要求のみ——`Cargo.lock` は読まない、直接依存のみを表示する方針のため）。
fn generate_used_crates() {
    println!("cargo:rerun-if-changed=Cargo.toml");

    let manifest = std::fs::read_to_string("Cargo.toml").expect("Cargo.toml の読み込みに失敗");
    let doc: toml::Value = manifest.parse().expect("Cargo.toml の解析に失敗");
    let deps = doc
        .get("dependencies")
        .and_then(toml::Value::as_table)
        .expect("Cargo.toml に [dependencies] がありません");

    let mut entries: Vec<(String, String)> = deps
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
            (name.clone(), version)
        })
        .collect();
    entries.sort();

    let mut out = String::from("pub(crate) const USED_CRATES: &[(&str, &str)] = &[\n");
    for (name, version) in &entries {
        out.push_str(&format!("    ({name:?}, {version:?}),\n"));
    }
    out.push_str("];\n");

    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR が未設定");
    let dest = std::path::Path::new(&out_dir).join("used_crates.rs");
    std::fs::write(&dest, out).expect("used_crates.rs の書き込みに失敗");
}
