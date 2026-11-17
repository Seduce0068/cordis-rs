---
type: Reference
title: ADR-003 fs-patch の実装選定
description: Codex-style patch port に既存 crate を採用せず std-only 実装を選んだ理由
tags: [cordis-rs, adr, tech-selection, fs-patch]
resource: https://github.com/Seduce0068/cordis-rs
generated: 2026-11-18T07:50:00+09:00
verified: 2026-11-18T07:50:00+09:00
status: accepted
---

## 決定

`src/ports/fs_patch.rs` に Codex-style patch（`*** Begin Patch` / `*** Update File:` /
`@@ anchor` / `*** End Patch`）の `parse_patch` / `apply_hunks` を **std のみ**で実装する。
crates.io の既存 crate は採用しない。

## 既存ソリューションの調査結果

| 候補 | ライセンス | 判定 | 不採用理由 |
| --- | --- | --- | --- |
| similar 3.2.1 | Apache-2.0 | 不採用 | diff 生成側のみで patch 適用 API が無い |
| diffy 0.5.2 | MIT/Apache-2.0 | 不採用 | unified diff (`---/+++/@@`) 専門、hashbrown 必須 |
| flickzeug 0.6.0 | MIT/Apache-2.0 | 不採用 | unified diff 専門、141 KB |
| patch-apply 0.8.3 | MIT | 不採用 | unified diff 専門、chrono + nom 依存、低活発 |
| mpatch 1.8.1 | MIT | 不採用 | Codex 形式非対応、CLI + Rayon で過剰 |

`apply_patch` / `codex-apply-patch` crate は crates.io に存在しない（API 404）。
決定的な差は**フォーマット互換性**で、全候補が unified diff 系であり、
DSH が使う Codex 形式を解釈できない。ライセンスはすべて AGPL-3.0-or-later と
互換でブロッカーではない。

## ライセンス

DeepSeek の貢献は MIT（Copyright (c) 2026 DeepSeek）。パーサ／マッチャの設計は
OpenAI Codex rust-v0.158.0 apply-patch 由来で Apache-2.0（Copyright 2025 OpenAI）。
両方の帰属を `THIRD_PARTY_NOTICES.md` に保持する。

## 検証

`cargo fmt --all --check` / `cargo check --all-targets` / `cargo clippy --all-targets -- -D warnings` /
`cargo test --all-targets` が成功すること。
