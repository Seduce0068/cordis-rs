---
type: Reference
title: ADR-006 Context 名前キーイベント API 移行
description: upstream ctx.on/emit への段階移行と descriptor API との共存方針
tags: [cordis-rs, adr, api-design, migration]
resource: https://github.com/Seduce0068/cordis-rs
generated: 2026-11-18T11:50:00+09:00
verified: 2026-11-18T11:50:00+09:00
status: accepted
---

## 決定

upstream の `ctx.on(name, listener)` / `ctx.emit(name, args)` に沿った名前キー API を
Context に追加する。既存の `Event<A,R>` descriptor API は維持し、段階移行する。

### Stage 1（今回）

- `Context::on(name, listener)` / `Context::once(name, listener)` / `Context::emit(name, args)` を追加。
- ペイロードは `serde_json::Value` 固定。リスナは `EmitListener<Value>` として登録。
- SlotKey は `(name, TypeId<Value>, TypeId<()>, TypeId<EmitListener<Value>>)` で、
  `Event::<Value, ()>::new(name).on_emit(...)` と**同じスロットを共有**する。
- `once` は `AtomicBool` で2回目以降を no-op にする（Disposer 循環参照を避けるため自己解除はしない）。
- `emit` は登録順に await する（upstream の同期 void とは意味が異なる点を明記）。

### Stage 2（今後）

- `prepend` / `global` / Context filter、disposer の boolean 戻り値を揃える。
- 既存 24 箇所の `on_*` と 10 箇所の `Event::new` を段階移行し、descriptor API を deprecated facade にする。
- `emit_detached` は upstream に無いため `emit_spawn` へ別名変更する。

## 注意点

「名前は型とセット」であり、動的 API は `Value` 固定なので他型の descriptor リスナとは見えない。
`parallel_errors` は `FallibleEmitListener` スロットを見るため、`on` で登録したリスナは届かない。

## 検証

`cargo fmt --all --check` / `cargo check --all-targets` / `cargo clippy --all-targets -- -D warnings` /
`cargo test --all-targets`（143 tests）が成功すること。
