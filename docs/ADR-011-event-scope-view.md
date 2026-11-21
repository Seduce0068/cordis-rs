---
type: Reference
title: "ADR-011: EventScopeView によるイベントスコープの表現"
description: "emit_with の2スコープ問題を解消する EventScopeView 抽象の設計"
tags:
  - adr
  - cordis-rs
  - architecture
  - events
resource: https://github.com/Seduce0068/cordis-rs
generated: 2026-11-21T08:46:32+09:00
verified: 2026-11-21T08:46:32+09:00
status: accepted
---

## 決定

ADR-010 の残課題（`emit_with` の2スコープ問題）を、concrete 型
`EventScopeView<'a>` で解消する。trait object（`&dyn EventSink`）は
ライフタイム制約で borrow 型を作れないため採用しない。

```rust
pub struct EventScopeView<'a> {
    registry: &'a EventRegistry,
    filter: Option<&'a EventFilter>,
}

impl<'a> EventScopeView<'a> {
    pub fn allows(&self, id: u64, name: &str) -> bool {
        self.registry.is_global(id) || self.filter.is_none_or(|f| f(name))
    }
}
```

- `Context::event_registry()` と新規 `Context::event_filter()` から構築する。
- `emit_with(self, this: &Context, name, args)` は
  `EventScopeView { registry: self.event_registry(), filter: this.event_filter() }`
  を組み、registry 所有者と filter スコープを明示的に分離する。
- `Event::emit` 系の `&Context` 版は adapter として残し、内部で
  `filter: None` の view を組む（後方互換）。
- remove 系3API（service 削除）は event filter と無関係なため触らない。

## 根拠

- R1（reviewer）: `emit_with`（Context::emit_with）は listener を
  `self` の registry から snapshot し、filter は `this.inner.event_filter`
  で判定している。ADR-010 の `EventSink::allows(&self, name)` では
  `this` を表現できない。`&dyn EventSink` は `'static` 制約で borrow 型に
  ならないため、concrete 型を推奨。
- R1（core-services-research）: 上流 cordis は filter を dispatch 時に
  thisArg に対して評価し、global は filter バイパス専用。`emit()` が
  filter を無视する現行挙動は upstream の thisArg なし dispatch と整合。
- 型付き Event の filter/global 無視（todo #40）は `filter: None` の view
  として「意図的な契約」に昇格できる。

## 裁定: registry の所有（重要）

core-services-research は「上流では子スコープが root の registry を共有する」
と報告したが、**現行実装は各 Context が独立した `EventRegistry::default()`
を持つ**（Context::make 内の EventRegistry::default()）。これを root 共有へ変更すると、
`ctx.provide` からの notify や inject 依存 reload（ADR-009 で保護した経路）に
波及し大規模変更となる。よって ADR-011 は**現行の独立 registry を前提**とし、
上流型の root 共有は第2段の段階的移行として「未着手」に記録する。
`EventScopeView` は単一 registry のみを指し、階層を導入しない。

## global と cleanup の扱い

- global は filter のバイパス専用であり、fiber ライフサイクルを免除する
  flag ではない（core-services-research の指摘）。
- HookMeta の削除は `EventRegistry::remove` が既に行っている
  （events.rs の `remove` 内の meta.remove(&id)）。R4 レビューで
  「未対応」前提が誤りであることを指摘され修正した。

## 移行手順（実装完了 2026-11-21）

1. `Context::event_filter()` getter を追加。
2. `EventScopeView<'a>` は context.rs に定義する（`EventFilter` は
   private 型のため events.rs からは参照できない）。`event_filter()`
   getter は `pub(crate)` とし、`EventFilter` を public にしない。
3. `emit_with` の内部を view 経由に変更（公開シグネチャ維持）。
4. emit 系 adapter を `filter: None` の view に統一。
5. 既存テスト（tests/cordis.rs の global/filtered dispatch テスト の global/filtered dispatch）が
   通ることを確認。

## 残る論点

- ADR-010 の `&dyn EventSink` 方針は本 ADR の concrete 型で置き換える。
  ADR-010 を proposed のまま残し、実装時に本 ADR を優先する。
- 子スコープの registry 共有（上流整合）は第2段 ADR で扱う。

## 関連

- [ADR-010](ADR-010-context-events-boundary.md) — Context/events 境界（proposed）
- [ADR-009](ADR-009-context-plugin-boundary.md) — FiberObserver による依存逆転
- [ADR-006](ADR-006-context-events-migration.md) — 名前キーイベント API の段階移行
