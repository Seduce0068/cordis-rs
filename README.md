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
|---|---|
| Context / スコープ | ルート + 子 + `isolate`（isolate は親へのフォールバックなし） |
| サービスレジストリ | `provide` / `get` / `require`（通知駆動の待機）/ `remove_service` |
| 5 つの dispatch モード | `emit` / `parallel` / `serial` / `bail` / `waterfall`（next 一回限り + veto） |
| effect | 逆順解除・single-shot disposer・reentrancy 対策・ネスト効果 |
| Fiber | `PENDING → LOADING → ACTIVE`、依存喪失で unload、復帰で reload |
| Plugin | `Plugin` trait + `FunctionPlugin`、`inject` による依存宣言 |
| コアサービス | `ctx.tools`（registry + pipeline）、`ctx.sessions`（append-only log + projection folds）、`ctx.commands` |
| 再利用 port | `dsh-tool-todo`、`dsh-command-feedback` |
| GPUI 連携 | 別リポジトリ `dsh-gpui-client-poc` の `cordis_todo_ui` が `todos` projection を描画 |

### 意図的な差分

- JS Proxy による `ctx.foo` アクセスは、Rust では明示的な `ctx.get::<T>()` / `require()` に置換。
- `serial` / `bail` の戻り値は `Option<R>`。`Some(0)` / `Some(String::new())` は bail。
  upstream の `false` だけは非 bail という差がある。
- Schemastery による Config 検証、YAML Loader、HMR は対象外。
- `emit` は試験の決定性のため await する（`emit_detached` で fire-and-forget も提供）。

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

`tests/cordis.rs` にランタイム自体の 7 テスト、
`tests/official_plugin_port.rs` に公式 `dsh-command-compact` の移植デモを収録。
移植デモは `inject = ['commands', 'compaction']` の待機・再ロードを検証します。

## ライセンス

MIT。上流 Cordis（MIT、Copyright (c) 2026 DeepSeek）の API 契約を Rust に
移植しているため、上流ライセンスを継承しています。個人リポジトリの既定は
AGPL-3.0-or-later ですが、ここでは上流 MIT との互換性を優先しました。

## 関連

- 旧 PoC（ACP 経由の GPUI クライアント）: `../dsh-gpui-client-poc/`
- 調査レポート: ローカル `passkey-gateway/docs/dsh-client-gpui-migration-2026-11-16.md`（非公開）
