//! exe に `assets/icon.ico` とバージョン情報（Cargo.toml の description 等）を
//! 埋め込む。Windows ターゲットのときだけ実行する（`CARGO_CFG_TARGET_OS` で判定。
//! winresource の README 推奨どおり、クロスコンパイル時も正しく判定できる）。

fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        winresource::WindowsResource::new()
            .set_icon("assets/icon.ico")
            .compile()
            .expect("exe リソースの埋め込みに失敗");
    }
}
