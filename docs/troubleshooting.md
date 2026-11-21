---
type: Playbook
title: "cordis-rs 障害時トラブルシューティング / 復旧 Runbook"
description: "部分 mount 失敗・状態破損・service 待ち・テレメトリ異常の切り分け手順"
tags:
  - cordis-rs
  - runbook
  - operations
resource: https://github.com/Seduce0068/cordis-rs
generated: 2026-11-21T19:45:00+09:00
verified: 2026-11-21T19:45:00+09:00
status: "Self-reviewed"
---

## Overview

この runbook は cordis-rs（PoC）で想定される障害の切り分けと復旧手順をまとめる。
**本リポジトリは production-ready ではない**（README の安定性マトリクスを参照）。
ここに列举する手順は PoC 内の診断を目的とし、本番運用の保証ではない。

作成時に確認した観測 API: `CompositionState::try_load`、`JsonlPersistence::read_recovered`、
`EventLogReporter::dropped`、`RedactingExporter::failures`。

## Prerequisites

- Rust toolchain（edition 2024）と `cargo`
- リポジトリルートで実行すること
- ローカル完全検証: `bash scripts/verify.sh`
- ログは構造化イベント（`ctx.otel` は in-memory レコード）として残る

## Procedures

### 完全検証の実行

```bash
cd /home/koppe/document/deepseek-harness/default-workspace/work/cordis-rs
bash scripts/verify.sh
```

`verify.sh` は fmt → check → test → clippy → シークレットスキャン → markdownlint
→ cargo audit → example smoke → no-unsafe → doc-counts を順に実行する
（CI の `extra_gates` と同順）。
**CI は共通ワークフロー `rust-gates.yml@v1` に委譲**されており、extra_gates に
smoke / no-unsafe / doc-counts / secrets / markdown が接続されている。

### 部分 mount 失敗の確認

`composition.mount()` / `mount_with()` は `MountReport` を返す。
`failed`（plugin エラー）と `skipped_unknown`（未登録 plugin id）を確認する。
`examples/mount_real_composition.rs` はdiagnostic レポートとして `MountReport` の
`failed` / `skipped_unknown` を一覧出力する（assert で落とさない）。fixture 更新後は
この一覧を読んで未 port エントリを確認する。

### 状態ファイルの破損確認

- composition state: `CompositionState::try_load(path)` が `Err` を返した場合は破損。
  後方互換の `load()` は空 overrides に落ちるため、復元には `try_load` を使う。
- JSONL session: `JsonlPersistence::read_recovered()` が `RecoveryReport` を返し、
  `truncated_at_line` / `reason` で torn write を特定できる。
- 復旧前に該当ファイルをバックアップし、空 overrides で再起動していないことを確認する。

## Troubleshooting

| 症状 | 確認点 | 対処 |
| --- | --- | --- |
| plugin が起動しない | `MountReport.failed` / `skipped_unknown` | plugin id の登録漏れを解消。fiber の inject 依存を提供 |
| 再起動で無効化設定が戻る | `try_load` の `Err` | 破損 state をバックアップから復元。`load()` の空フォールバックで起動していないか確認 |
| `require` が応答しない | 依存 service 名の登録漏れ、provider の `mount` 成否 | 依存を先に provide。診断には `require_untyped_with_timeout()` で上限付き待ちを使う（無期限 `require` は従来どおり） |
| service reload が走らない | fiber の `retiring` / 依存 drain | 一時 unload 後 Pending に戻っていれば drain 対象。明示 unload は終端 |
| イベントが消えた | `EventLogReporter::dropped()` | `max_events` 超過で古いイベントが eviction されている。上限を上げるか記録間隔を見直す |
| テレメトリが止まっている | `RedactingExporter::failures()` | inner exporter の失敗回数。host 操作自体は成功し続けるためカウンタで検知する |
| シークレットスキャンが落ちる | `gitleaks detect --redact` の出力 | 実際の secret を削除。`--redact` により CI ログには生値が出ない |
| markdown lint が落ちる | `bash scripts/check-markdown.sh` | 全 Markdown が対象。MD ルール違反を修正 |

## Contacts

個人リポジトリのため外部ステークホルダーは存在しない。
問題は [Seduce0068/cordis-rs](https://github.com/Seduce0068/cordis-rs) の issue として記録する。
報告時は `verify.sh` の出力と上表の観測 API 値を添えること。

---

Status: Self-reviewed / Reviewed-by: lead agent / Date: 2026-11-21
