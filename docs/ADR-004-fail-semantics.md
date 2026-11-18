---
type: Reference
title: ADR-004 公開 seam の fail 動作
description: approval / timeout / checkpoint / dispose の既定 fail 動作とその根拠
tags: [cordis-rs, adr, api-design, fail-closed]
resource: https://github.com/Seduce0068/cordis-rs
generated: 2026-11-18T11:23:14+09:00
verified: 2026-11-18T11:23:14+09:00
status: accepted
---

## 決定

API 設計レビュー第3弾の指摘を受け、seam ごとの既定 fail 動作を次のとおり定義する。

| seam | 既定 | 根拠 |
| --- | --- | --- |
| approval | fail-closed（answerer 不在で拒否） | 承認なしのツール実行を防ぐ |
| sandbox / path | fail-closed | セキュリティレビューで確認済み |
| persistence (checkpoint) | fail-closed（`try_load`） | 破損を黙って 0 にするとログを二重処理する |
| timeout | 構造化エラー結果（`Ok(ToolOutput)` + `is_error() == true`） | `Err` にすると retry が二重実行を起こすリスクがある |
| dispose | 失敗を呼び出し側へ伝播 | 解除漏れを検知可能にする |

## 補足

- `ToolOutput::is_error()` は `content.code` の `TOOL_*` / `*_ERROR` プレフィックスで判定するメソッド版である。
  リテラル変更を伴わない利点がある一方、tool が同じ文字列を通常データとして返すと誤判定する。
  厳密な属性が必要なら次期破壊リリースで型フィールドを導入する。
- `CheckpointCursor::load()` は後方互換のため残し、欠落・不正で 0 にフォールバックする。
  新規コードでは `try_load()` を使用し、NotFound のみ Ok(既定)、それ以外は Err とする。
- `ToolPipeline::run` は失敗結果で post-execute をスキップする（upstream の normalized outcome 契約との差分）。
  observer を失敗時にも通す変更は別案件とする。

## 検証

`cargo fmt --all --check` / `cargo check --all-targets` / `cargo clippy --all-targets -- -D warnings` /
`cargo test --all-targets`（142 tests）が成功すること。
