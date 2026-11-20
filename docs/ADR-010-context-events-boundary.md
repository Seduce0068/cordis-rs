---
type: Reference
title: "ADR-010: Context と events の依存境界"
description: "context.rs と events.rs の双方向 import を一方向化する設計"
tags:
  - adr
  - cordis-rs
  - architecture
resource: https://github.com/Seduce0068/cordis-rs
generated: 2026-11-21T00:09:47+09:00
verified: 2026-11-21T00:09:47+09:00
status: proposed
---

## 決定（提案）

`context.rs:12` が `events::{BoxFuture, EventRegistry}` を import し、
`events.rs:29` が `context::{Context, Disposer}` を import する双方向依存を、
**`EventSink` 抽象の導入と既存 `with_registry` パターンの拡張**で一方向化する。

1. `EventSink` trait を `context` 側に定義する。
   `registry(&self) -> &EventRegistry` と `allows(&self, name: &str) -> bool`
   （スコープフィルタ判定）を提供する。
2. `impl EventSink for Context` を追加。フィルタ判定は Context の責務として残す。
3. `events.rs` の各 dispatch API（`on_serial` / `serial` / `on_bail` / `on_emit` /
   `emit` / `emit_spawn` / `emit_detached` / `parallel` / `parallel_errors` 等）を
   `&dyn EventSink` 受け取り版へ統一する。
4. 既存の `&Context` 受け取り版は adapter として残し、呼び出し元（`typed_event!`
   の生成コードを含む）を無変更にする。
5. `events.rs` は `Context` と `Disposer` の import を落とし、`EventSink` と
   `Disposer` の配置は別案件（primitives 整理）として切り出す。

## 根拠

- reviewer の R1 分析: waterfall 系は既に `on_waterfall_with_registry`（:321）と
  `waterfall_with_registry`（:353）で `&EventRegistry` 受け取りが完了しており、
  `&Context` 版は adapter になっている。**新規設計ではなく既存パターンの拡張**。
- 唯一の論点は `emit_with` の scope filter（context.rs:371-387）が
  `Context.inner.event_filter` を読む点。`EventSink` に `allows()` を置けば
  フィルタ判定を Context 側に残しつつ events の Context 依存を切れる。
- ADR-009 で Fiber 境界を一方向化した流れと整合する。

## 検討した代替案

| 案 | 評価 |
| --- | --- |
| (a) `Disposer` / `BoxFuture` を共通 primitives へ移動 | 却下。`EventRegistry` が必要な Context と Context が必要な dispatch API が残り循環が解消しない。ADR-009 の (c) と同型の「効かない案」。primitives 整理として別 ADR に回す。 |
| (b) `EventSink` ＋ `with_registry` パターンの拡張 | **採用**。既存実績があり adapter 化で後方互換を維持。filter は Context 側に残す境界を明記。 |
| (c) 現状維持 | 却下。ADR-009 の一方向化の流れと矛盾。waterfall だけ直した理由を説明できない。 |

## 移行手順

1. `context.rs` に `EventSink` trait と `impl EventSink for Context` を追加。
2. `events.rs` の残り dispatch API を `&dyn EventSink` 受け取り版へ追加。
3. `&Context` 版を adapter に変更（公開シグネチャ維持）。
4. `events.rs` の `use crate::context::{Context, Disposer}` を削除。
5. 既存の全イベントテスト（`emit` / `parallel` / `serial` / `bail` / `waterfall` と
   scope filter 系）が通ることを確認。

## 残る論点

- `Disposer` の配置（context.rs:50）は今回触らない。events が Disposer を
  使い続けるため、完全な「events → context 依存ゼロ」には primitives 移動が
  別途必要。ADR-010 は「Context 型依存を切る」ところまでを範囲とする。
- `typed_event!` マクロが生成する `on(ctx, ...)` / `emit(ctx, ...)` は adapter
  経由で動くことをテストで固定する。

## 関連

- [ADR-006](ADR-006-context-events-migration.md) — 名前キーイベント API の段階移行
- [ADR-009](ADR-009-context-plugin-boundary.md) — context/plugin の依存逆転
