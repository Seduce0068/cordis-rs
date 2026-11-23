---
type: Reference
title: ADR-001 Cordis 互換ランタイムを Rust で再実装する
description: Cordis プラグイン互換ランタイムを Rust で再実装する決定と意図的差分
tags: [cordis-rs, adr, runtime, reimplementation]
resource: https://github.com/Seduce0068/cordis-rs
generated: 2026-11-17T00:00:00+09:00
verified: 2026-11-22T07:53:02+09:00
status: accepted
---

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

- ライセンス面の障壁はない（上流 Cordis は MIT。本リポジトリは AGPL-3.0-or-later で再配布し、上流通知は THIRD_PARTY_NOTICES.md に保持）。
- Node.js 埋め込みは依存とデバッグの複雑さが残る。
- 新規 ABI は既存プラグインの資産を活かせない。

## 意図的な差分

- JS Proxy による `ctx.foo` アクセスは `ctx.get::<T>()` / `require()` に置換。
- `serial` / `bail` は `Option<R>` 戻り値。`Some(false)` も非 bail で upstream と同一（差は表現のみ）。
- Schemastery の完全な DSL とファイル監視 HMR は対象外。Config 検証と
  YAML Loader は独自実装し、`!!js` は変数・補間・比較・算術・論理・三項・
  member access・配列／オブジェクト／テンプレートリテラルをサポート
  （任意の JavaScript 評価は行わない）。
- 軽量 YAML パーサは依存追加を避けるため自作（serde_yaml は deprecated、
  yaml-rust2 は保守中だが Cargo.toml 凍結方針のため不採用）。

## 結果

- 全 333 テスト通過（統合 329 ＋ unit 4）、CI green（タグ `v0.1.0` 〜 `v0.261.0`）。
- コアサービス 38 個を提供（`tools` / `sessions` / `llm` / `agentLoop` /
  `fs` / `jobs` / `shell` / `goals` / `systemPrompt` / `compaction` /
  `approval` / `storage` / `agents` / `otel` / `retry` / `sessionTitles` / `schedule` / `planMode` / `credentials` / `timeoutPolicy` / `pluginInventory` / `timeContext` / `sessionReferences` / `fileReferences` / `fileSearch` / `tmuxContext` / `repeatReminder` ほか）。
- 再利用 port 63 個を移植（tool-fs / tool-bash / tool-todo / tool-subagent /
  skill-filesystem / message-feedback / session-persistence-jsonl /
  llm-deepseek / feedback-producer / command-compact / session-title-all-prompts-llm / tool-schedule / tool-exit-plan-mode / agent-instructions / harness-dev-context / tool-goal / fs-observation-policy / tool-session-list / tool-str-replace-editor / time-context / tmux-context / tool-jobs / timeout-policy / compaction-* / fs-patch / mcp-resources / hooks-claude-code / session-turn-outline / deepseek-llm-api-extensions / session-stats / session-log-deepseek / native-tool-executor / command-builtins / command-parse / mcp-templates / hooks-codex / duration / tool-pairing / text-search ほか）。
- library port として `hook-protocol`（matcher / outcome merge）を追加。
- 上流 `snapshots/sdk/text-turn/cordis.yml` を Rust ランタイムで読み込み、
  port 存在分のみ mount し `failed` / `skipped_unknown` をレポートする部分 mount
  経路を確立（6 エントリすべての mount ではない、swarm review R7-DC16）。
- GPUI 側で todo / fs / agent / prompt / 実 composition / plugin トグル /
  ACP ストリームを描画。

## 未対応

- 公式プラグイン 332 個の全移植（現状 63 port）。
- Schemastery DSL の完全互換とファイル監視 HMR。
- OTLP 実送信（`ctx.otel` は in-memory レコードのみ）。
- モデル側残高不足により ACP ストリーミング本文の実描画は未検証。
