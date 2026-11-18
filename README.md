# cordis-rs

[![CI](https://github.com/Seduce0068/cordis-rs/actions/workflows/ci.yml/badge.svg)](https://github.com/Seduce0068/cordis-rs/actions/workflows/ci.yml)

DSH（DeepSeek Harness）を GPUI/Rust で再実装するための第一歩として作った、
Cordis 互換プラグインランタイムの PoC です。

リポジトリ: <https://github.com/Seduce0068/cordis-rs>

## 背景

DSH の公式プラグインはすべて MIT で、ソースは
[deepseek-ai/deepseek-harness](https://github.com/deepseek-ai/deepseek-harness)
の `packages/` に公開されています。プラグインを移植するには、その土台である
Cordis（`vendor/cordis`、upstream `cordiverse/cordis` v4.0.0-rc.12）の
サービスレジストリ・イベント・ライフサイクルを Rust 側で再現する必要があります。

## 実装したもの

| 機能 | 対応 |
| --- | --- |
| Context / スコープ | ルート + 子 + `isolate`（isolate は親へのフォールバックなし） |
| サービスレジストリ | `provide` / `get` / `require`（通知駆動の待機）/ `remove_service` |
| 5 つの dispatch モード | `emit` / `parallel` / `serial` / `bail` / `waterfall`（next 一回限り + veto） |
| effect | 逆順解除・single-shot disposer・reentrancy 対策・ネスト効果 |
| Fiber | `PENDING → LOADING → ACTIVE`、依存喪失で unload、復帰で reload |
| Plugin | `Plugin` trait + `FunctionPlugin`、`inject` による依存宣言 |
| Config 検証 | 型・必須・範囲・デフォルト適用、フィールド名付きエラー |
| Loader | 軽量 YAML（`entries:` / `name:` / `insert:` / `!!js` 式）、Composition の部分 mount |
| コアサービス（38） | `tools` / `sessions` / `sessionProjections` / `commands` / `llm` / `agentLoop` / `fs` / `jobs` / `shell` / `skills` / `subagents` / `workingDirectory` / `messageFeedback` / `goals` / `systemPrompt` / `toolResultPruner` / `tokenMeter` / `compaction` / `approval` / `permissionPresets` / `sessionQuery` / `workspace` / `storage` / `agents` / `otel` / `retry` / `sessionTitles` / `schedule` / `planMode` / `credentials` / `timeoutPolicy` / `pluginInventory` / `timeContext` / `sessionReferences` / `fileReferences` / `fileSearch` / `tmuxContext` / `repeatReminder` |
| library port | `hook-protocol`（matcher と most-restrictive outcome merge、regex 対応） |
| 再利用 port（63） | `command-compact` / `command-goal` / `command-feedback` / `tool-todo` / `tool-fs` / `tool-bash` / `tool-working-directory` / `tool-subagent` / `tool-session-query` / `skill-filesystem` / `message-feedback` / `session-persistence-jsonl` / `llm-deepseek` / `feedback-producer` / `session-title-all-prompts-llm` / `tool-schedule` / `tool-exit-plan-mode` / `agent-instructions` / `harness-dev-context` / `tool-goal` / `fs-observation-policy` / `tool-session-list` / `tool-str-replace-editor` / `time-context` / `tmux-context` / `tool-jobs` / `timeout-policy` / `compaction-image-offload` / `compaction-tool-result-pruner` / `fs-patch` / `mcp-resources` / `hooks-claude-code` / `session-turn-outline` / `deepseek-llm-api-extensions` / `session-title-first-prompt-llm` / `session-stats` / `session-log-deepseek` / `native-tool-executor` / `command-builtins` / `command-parse` / `mcp-templates` / `hooks-codex` / `duration` / `tool-pairing` / `text-search` / `sandbox-policy` / `user-questions` / `fs-errno` / `session-filename` / `goal-fold` / `llm-attribution` / `llm-call-config` / `compaction-checkpoint` / `llm-message` / `user-approval` / `goal-round-driver` / `compaction-automation` / `code-language` / `attachment-admission` / `workspace-path` / `llm-resilience` / `llm-transport` / `otel-exporter` |
| GPUI 連携 | 別リポジトリ `dsh-gpui-client-poc`：todo / fs / agent / prompt / 実 composition / plugin トグル / 実 composition トグル / ACP ストリーム UI |

### 意図的な差分

- JS Proxy による `ctx.foo` アクセスは、Rust では明示的な `ctx.get::<T>()` / `require()` に置換。
- `serial` / `bail` の戻り値は `Option<R>`。`Some(0)` / `Some(String::new())` は bail。
  upstream の `false` だけは非 bail という差がある。
- Schemastery の完全な DSL とファイル監視 HMR は対象外（Config 検証と YAML Loader は独自実装済み）。
- `llm` / `tools` / `agentLoop` は retry policy に対応（`StreamChunk::Error` → bounded backoff）。
- `!!js` は変数参照・補間・比較（`===` 含む）・算術・論理・三項・member access・文字列メソッド（`includes` / `startsWith` / `endsWith`）・配列／オブジェクト／テンプレートリテラル・`process.env`・`dshHomePath` をサポート。任意の JavaScript は評価しない。
- `emit` は試験の決定性のため await する（`emit_detached` で fire-and-forget も提供）。

## 推奨エントリポイント（ドメイン別）

`plugin_ports` は多数の helper を Flat に re-export する。用途に応じて次の
エントリポイントから始めること（旧名 alias は v1.0.0 で削除予定）。

| ドメイン | まず手に取る API |
| --- | --- |
| 実行・ツール基盤 | `ToolPipeline::install_default_controls` / `ToolRegistry` / `provide_tools` |
| LLM 呼び出し | `LlmService` / `with_llm_timeout` / `with_llm_retry` / `call_config_eq` |
| 会話セッション／永続化 | `SessionStore` / `JsonlPersistence` / `provide_sessions` |
| コンテキスト／圧縮 | `CompactionEngine::maybe_compact` / `mount_compaction_automation` / `fold_goal` |
| 対話統制（human-in-the-loop） | `mount_hooks_claude_code` / `UserQuestionService` / `mount_sandbox_policy` |
| 純粋ユーティリティ | `parse_patch` / `normalize_search` / `language_for_path` / `fs_error_from_errno` |

安定性ラベル（stable / beta / phase-1 / stub / fail-open）は下記マトリクス側で管理し、この表には混在させない。

新規 library port の追加基準は [ADR-008](docs/ADR-008-library-port-policy.md) を参照。

## 安定性マトリクス（production-ready の前に読む）

**このリポジトリは PoC であり、production-ready ではない。** port 数・テスト数は
実装カバレッジを示すだけで、実モデル呼び出し・テレメトリ送信の失敗系
（timeout / retry / 部分失敗 / 再起動復元）は未検証。下記の stub / fail-open 行を
解消するまでは本番利用を想定しないこと。

分類: **stable** = API/挙動固定＋回帰テストあり、**beta** = 動くが自動配線でない、
**phase-1** = 機能一部・未配線を明記、**stub** = 外部 I/O を模擬のみ、
**fail-open** = 統制が計算されるが強制されない（暫定ラベル）。

| コンポーネント | 分類 | 解消条件 |
| --- | --- | --- |
| Context / registry / typed events | stable | — |
| LlmService seam (`register` / `stream`) | stable | — |
| `FileSystem::scoped` root 閉じ込め | beta | symlink 検証後に開くまで TOCTOU 残存（std のみの PoC では openat 相当が不可）。検証済み canonical path を返して軽減 |
| 組み込み LLM provider (`from_async_text`) | stub | 実 HTTP / timeout / retry の end-to-end 検証 |
| OTLP (`ctx.otel`) | stub | 実送信の実装（現在 in-memory） |
| JSONL 永続化 (`mount`) | beta | 通常 turn への自動配線 |
| `hooks-claude-code` / `hooks-codex` | phase-1 | Ask / Stop / Subagent の実装と配線 |
| `user-questions` | phase-1 | timed wait / steer / continued |
| `sandbox-policy` | phase-1 | 実行ポイント（Shell / FS / Tool）への接続 |
| `goal-fold` | phase-1 | 書き込み側との統合 |
| `ToolPipeline` approval 既定 | fail-open | production では `install_default_controls(hook_effect, policy)` を呼ぶ（fail-closed approval + 実際の hook deny effect + sandbox gate を一括登録） |

根拠: `src/services/llm.rs:110,166,192` / `telemetry.rs:5` /
`session_persistence_jsonl.rs:238` / `sandbox_policy.rs:1,6` /
`approval.rs:157` / `hooks_claude_code.rs:139`。

**既知の fail-open / stub は「未対応」ではなく現状の制約**。fail-open 行は
解消条件を満たした時点で stable へ移動する暫定ラベルであり、永久分類ではない。
本番では `require_approval()` と answerer 登録、実行ポイントへの policy 接続を
明示的に行うこと。

## 検証

```bash
cargo fmt --all --check
cargo check --all-targets
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
cargo run --example agent_demo
cargo run --example agent_checkpoint_demo
cargo run --example load_cordis_yml
cargo run --example mount_composition
cargo run --example composition_with_tool_fs
cargo run --example mount_real_composition
cargo run --example permission_flow_demo

# 全 example の headless smoke test
bash scripts/smoke-examples.sh

# fmt / check / clippy / test / smoke をまとめて実行
bash scripts/verify.sh
```

`agent_demo` は scripted LLM provider で agent-loop を回し、tool-fs の
`write` / `read` を使って実ファイルを作成・検証する end-to-end デモです
（ネットワークやモデル認証は不要）。

`agent_checkpoint_demo` は agent-loop の session イベントを checkpoint hook 経由で
JSONL に永続化し、restore までを検証するデモです。

`load_cordis_yml` は DSH 実サンプルの `cordis.yml` を丸ごと loader で読み込み、
6 エントリと `!!js` 式（dshHomePath / process.env / 三項）の解決を検証します。

`mount_composition` は YAML -> Composition -> 検証済み config -> PluginFactory -> Fiber の
起動までを検証します。

`composition_with_tool_fs` は YAML の `config` で有効なツールを選び、実際に
`write` / `read` を呼んでファイルを検証します。

`mount_real_composition` は実 DSH fixture を読み込み、port が存在する
エントリだけを fiber として起動し、未対応のものをレポートします。

`permission_flow_demo` は preset 切替 → approval mode → tool 実行の
権限フローを検証します（read-only / workspace-write は deny、
danger-full-access は allow）。

テストは `tests/cordis.rs`（ランタイム）、`tests/config_loader.rs`（Loader / `!!js`）、
`tests/core_services.rs`（コアサービス・port）、`tests/official_plugin_port.rs`
（`dsh-command-compact`）、`tests/official_plugin_ports.rs`（`command-goal` / `tool-todo`）
に分かれています（全 196 件）。

`mount_real_composition` は上流 `snapshots/sdk/text-turn/cordis.yml` の
6 エントリすべてを fiber として起動します。

## ライセンス

`AGPL-3.0-or-later`（[LICENSE](LICENSE) を参照）。上流 Cordis（MIT、
Copyright (c) 2026 DeepSeek）の API 契約を Rust に移植しているため、
上流の著作権表示と許諾通知は [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)
に保持しています。MIT 互換のコードを AGPL で再配布する形です。

## 関連

- 旧 PoC（ACP 経由の GPUI クライアント）: `../dsh-gpui-client-poc/`
- 調査レポート: ローカル `passkey-gateway/docs/dsh-client-gpui-migration-2026-11-16.md`（非公開）
