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
既存 `with_registry` パターンの拡張で一方向化する方針。

> **2026-11-21 更新**: 当初の `EventSink` trait 方針は ADR-011 の concrete
> `EventScopeView<'a>` に置き換えられた（trait object は `'static` 制約で
> borrow 型を作れないため）。以下は当時の決定を履歴として残すもので、
> 実装は ADR-011 経路のみ。

1. ~~`EventSink` trait を `context` 側に定義する。~~ → ADR-011 の
   `EventScopeView` に置換。
2. ~~`impl EventSink for Context` を追加。~~ → `Context::event_filter()` +
   `EventScopeView::new()` に置換。
3. `events.rs` の dispatch API のうち、`emit_with` 以外を `&EventRegistry`
   受け取り版へ追加する（filter 2スコープを必要とする `emit_with` だけ
   context.rs に残す）。
4. 既存の `&Context` 受け取り版は adapter として残し、呼び出し元（`typed_event!`
   の生成コードを含む）を無変更にする。
5. `emit_with` は ADR-011 の `EventScopeView` 経由で view を組み、
   registry 所有者と filter スコープを分離する（実装済み）。
6. `Disposer` の配置は別案件（primitives 整理）として切り出す。

### R1 修正（実装前レビュー）

当初の `allows(name)` シグネチャでは `emit_with` の2スコープを表現できない。

`emit_with`（context.rs:376-387）は **registry 所有者 `self`** ではなく
**dispatch 先スコープ `this`** の `event_filter` で判定しており、
`allows(&self, name)` では `this` を渡せない。対策として:

- `EventSink` から `allows()` を外し、`emit` 系は `&Context` 版を維持。
- ADR-010 の範囲を「全 dispatch API の EventSink 化」から
  「**filter 非依存の dispatch API の EventSink 化**」へ縮小。
- 「events.rs の Context import を落とす」目標は**未達**として明示。
  完全な一方向化には `EventScopeView` などの別抽象が必要で、第2段 ADR に委ねる。
- 段1-3（EventScopeView の導入と emit_with の view 化）は ADR-011 として
  実装完了。段4-5（dispatch API の registry 化と Disposer 共通化）は未着手。
  範囲が EventScopeView に変更されたため本 ADR は proposed のまま、
  ADR-011 を accepted として運用する。

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
| (b) `EventSink` ＋ `with_registry` パターンの拡張 | **採用（一部完了）**。既存実績があり adapter 化で後方互換を維持。trait object は `'static` 制約で borrow 型にできないため、ADR-011 の concrete `EventScopeView<'a>` に置き換えた。 |
| (c) 現状維持 | 却下。ADR-009 の一方向化の流れと矛盾。waterfall だけ直した理由を説明できない。 |

## 移行手順と進捗（2026-11-21 更新）

1. ✅ `context.rs` に concrete `EventScopeView<'a>` と `event_filter()` getter を追加
   （ADR-011、status accepted）。
2. ✅ `emit_with` を view 経由に変更（registry 所有者 self と filter スコープ this を分離）。
3. ✅ `Context::emit` の filter 無視を「upstream thisArg なし dispatch と整合」として明文化。
4. ⬜ `events.rs` の残り10個の dispatch API（`on_serial` / `serial` / `on_bail` / `bail` /
   `on_emit` / `emit` / `emit_spawn` / `parallel` / `parallel_errors`）を
   `&EventRegistry` 受け取り版へ追加し、`&Context` 版を adapter 化。
   waterfall 系（`on_waterfall` / `waterfall`）は既に `*_with_registry` で完了済み。
   `emit_detached` は deprecated のため、新 API を足すなら `emit_spawn` に統合する。
5. ⬜ 残る `Disposer` 依存を共通 primitives へ移動。このとき初めて
   `events.rs` の `use crate::context::...` が消え、真の循环解消となる。
6. ✅ 既存の全イベントテスト（`emit` / `parallel` / `serial` / `bail` / `waterfall` と
   scope filter 系）が通ることを確認（215 tests）。

### R1 再評価（2026-11-21）

EventScopeView は context→events の依存（`this.inner.event_filter` 直接参照）を
減らしたが、**events→context の依存15箇所はすべて `ctx.event_registry()` 経由で
残存**。よって (a) Disposer/BoxFuture 共通化だけでは循环は解消せず、ADR の却下理由は
今も有効。段4（dispatch API の registry 化）を先に進め、段5で Disposer を
「最後の1本」として共通化するのが段階的経路。BoxFuture は既に events.rs 定義で
context.rs が import しており、Disposer とは別軸（context→events の向き）。

## 残る論点

- `Disposer` の配置（context.rs:50）は今回触らない。events が Disposer を
  使い続けるため、完全な「events → context 依存ゼロ」には primitives 移動が
  別途必要。ADR-010 は「Context 型依存を切る」ところまでを範囲とする。
- `typed_event!` マクロが生成する `on(ctx, ...)` / `emit(ctx, ...)` は adapter
  経由で動くことをテストで固定する。

## 関連

- [ADR-006](ADR-006-context-events-migration.md) — 名前キーイベント API の段階移行
- [ADR-009](ADR-009-context-plugin-boundary.md) — context/plugin の依存逆転
