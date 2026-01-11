# yurecollect

[KusaReMKN/yure](https://github.com/kusaremkn/yure) で配信されているデータをリアルタイムに受信し、より長い期間のデータを保持、再配信します。

# yurecollect

上流の WebSocket に接続し、受信した JSON データを標準出力へ出しつつ、メモリに保持し、ローカルの Web フロントエンドでリアルタイム可視化（uPlot）します。TLS（`wss://`）にも対応しています。

## 主な機能

- WebSocket 接続（`ws://`/`wss://`）: 上流のエンドポイントへ接続してテキストフレームを受信
- 標準出力: 受信した内容をそのまま出力
- メモリ保持: 約数 GB（コード既定値は 3GB）のリングバッファで最新を維持
- Web UI 提供: ローカル HTTP サーバ（ポート 3000）でフロントエンド提供
- リアルタイム可視化: uPlot による加速度（x/y/z）時系列をライブ更新
- ユーザーエージェント別表示: `userAgent` × 軸（ax/ay/az）の組で別系列化（色分け）
- 軸制御: 横軸は「現在時刻」を右端に固定、未来時刻のサンプルは無視（左端は自動）
- ログ表示: グラフ下に最新 ~10 行を上から順に表示（高さ制限あり）

## 要件

- Rust 1.88 以上
- ネットワークアクセス（上流 WebSocket への接続）

## ビルド

```bash
cargo build --release
```

## 実行

WebSocket URL は引数または環境変数 `WS_URL` で指定できます。

```bash
# 引数で指定
cargo run --release -- wss://example.com/your/ws

# 環境変数で指定
WS_URL=wss://example.com/your/ws cargo run --release
```

起動後、Web UI は `http://localhost:3000/` でアクセスできます。

### エンドポイント

- `GET /api/messages?limit=N`: メモリ保持中の最新メッセージ配列を返却
- `WS /ws`: 受信メッセージをリアルタイム配信
- `/`: フロントエンド（uPlot）

### 受信データ例

```json
{
	"t": 1768117058365,
	"userAgent": "yuredroid 1.4.2 on Xiaomi 2201117TG",
	"x": -0.005534179508686066,
	"y": 0.005334913730621338,
	"z": 0.00000762939453125,
	"yureId": "EReERYeurRE"
}
```

- 時刻 `t` は UNIX ミリ秒として受け取り、グラフでは秒へ変換して使用します。
- `userAgent` が異なる場合は別系列（各軸ごと）で表示されます。

## メモリ保持について

- 既定値は約 3GB（`MAX_BUFFER_BYTES`）です。調整したい場合は [src/main.rs](src/main.rs) の定数を変更してください。
- 上限を超える場合は古いメッセージから破棄して空き領域を確保します。

## Docker で利用

ローカルビルド:

```bash
docker build -t ghcr.io/yude/yurecollect .
```

実行例（引数指定 or 環境変数指定）:

```bash
docker run --rm -p 3000:3000 ghcr.io/<OWNER>/<REPO>:local wss://unstable.kusaremkn.com/yure/

docker run --rm -p 3000:3000 -e WS_URL=wss://unstable.kusaremkn.com/yure/ ghcr.io/<OWNER>/<REPO>:local
```

## GitHub Actions / GHCR（マルチアーキテクチャ）

- push や tag（`v*`）で、`linux/amd64, linux/arm64` のマルチアーチイメージを GHCR に公開します。
- ワークフロー: [.github/workflows/docker.yml](.github/workflows/docker.yml)
- 取得先: `ghcr.io/<OWNER>/<REPO>:latest` やタグ固有の参照

## トラブルシューティングのヒント

- `wss://` 接続に失敗する場合は証明書のルート（公開 CA）に注意してください。
- Web UI が真っ白な場合はブラウザキャッシュをクリア、またはローカル直アクセス（`http://localhost:3000/`）を試してください。
- 受信頻度が非常に高い環境では、UI側で描画更新をスロットリングしています（既定 100ms）。必要に応じて調整できます。

## ライセンス

MIT
