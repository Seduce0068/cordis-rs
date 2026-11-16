# ADR-001: Cordis 互換ランタイムを Rust で再実装する

- ステータス: accepted
- 日付: 2026-11-17
- 決定者: Seduce0068

## 背景

DSH を GPUI で再実装するにあたり、最大の障壁はプラグインです。公式プラグイン
332 パッケージはすべて MIT で公開されていますが、Cordis（Node.js 上の
サービス／イベント／effect レジストリ）に強く依存しています。

## 検討した選択肢

1. Cordis 相当を Rust で再実装し、公式プラグインを移植する
2. Node.js ランタイムを埋め込み、既存 TypeScript プラグインをそのまま動かす
3. WASM などの新規 ABI を定義し、プラグインを書き直す

## 決定

選択肢 1 を採用しました。`src/context.rs` / `src/events.rs` / `src/plugin.rs` に
サービスレジストリ、5 つの dispatch モード、reversible effect、Fiber の
ライフサイクルを実装しています。

## 根拠

- ライセンス面の障壁はない（上流 MIT を継承）。
- Node.js 埋め込みは依存とデバッグの複雑さが残る。
- 新規 ABI は既存プラグインの資産を活かせない。

## 意図的な差分

- JS Proxy による `ctx.foo` アクセスは `ctx.get::<T>()` / `require()` に置換。
- `serial` / `bail` は `Option<R>` 戻り値。`Some(false)` のみ upstream と異なる。
- Schemastery 完全互換、YAML Loader の `!!js` 式評価、HMR は対象外。
- 軽量 YAML パーサは依存追加を避けるため自作（serde_yaml は deprecated、
  yaml-rust2 は保守中だが Cargo.toml 凍結方針のため不採用）。

## 結果

- 全 31 テスト通過、CI green。
- `dsh-command-compact` / `dsh-command-goal` / `dsh-tool-todo` /
  `dsh-command-feedback` を移植。
- `ctx.tools` / `ctx.sessions` / `ctx.commands` のコアサービスを提供。
- GPUI 側で `todos` projection を描画する経路を確立。

## 未対応

`ctx.llm` / `ctx.agentLoop` の完全実装、公式プラグイン 332 個の全移植、
`!!js` 式評価、HMR。
