# pashari

<img src="assets/icon.svg" width="96" alt="pashari icon">

Windows 向けのスクリーンショット / 画面録画ソフト。

対応 OS: Windows 10 / 11

## ライセンス

CC0-1.0（`LICENSE` ファイル参照）。

## upstream ([yozba/pashari](https://github.com/yozba/pashari)) からの変更点
バイブコーディングのレビュー負荷を考慮してPRは今のところ控えています
- このレポジトリもCC0ライセンスなので利用したいものは自由に再利用できます

変更点
- 録画後に自動で再エンコードして圧縮するオプションが追加
  - 同程度の画質で約40倍のサイズ圧縮を確認[^1]
- 録画フォーマットにJPEG XLアニメーションが追加
- 選択範囲をOCRする機能が追加
- スクショ時にエクスプローラーを開くかを設定画面から選択可能
- 全画面スクリーンショットおよび全画面録画開始のショートカットが追加
- 全画面スクリーンショット時とOCR実行時にトーストポップアップを画面右下に表示
- その他細かいUI/機能改善

### prerequisite
変更点のうち一部機能についてそれぞれ以下の実行ファイルがパスに通っている必要があります
- 録画の再エンコード: `ffmpeg.exe`
  - ダウンロード: https://ffmpeg.org/download.html#build-windows
- JPEG XLアニメーションで録画: `cjxl.exe`
  - ダウンロード: https://github.com/libjxl/libjxl/releases -> jxl-platform-arch.7z -> platform-arch\bin\cjxl.exe
  - [ライセンスの関係](https://github.com/kkofu/pashari/issues/2)で直接エンコード可能なライブラリが使えないため、libjxlのCLIツールを介して変換しています

[^1]: v1.0.5 (upstream) で録った静止動画 (Video bitrate=Low (8Mpbs)、実測ビットレート24.51Mbps、総フレーム数151、サイズ14.7MB) とv1.0.5-1c8c143 (このfork) で録った静止動画 (Video bitrate=Low (8Mpbs)、Encoder=AMF、qvbr_quality_level=26、圧縮後ビットレート580kbps、総フレーム数152、サイズ358KB) の比較
