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
verified: 2026-11-21T10:42:00+09:00
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
上流型の root 共有は第2段の段階的移行として本 ADR 内で扱う
（2026-11-21 更新: accessor は実装済み、named 移行は設計済み・未実装）。
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
5. 既存テスト（tests/cordis.rs の global/filtered dispatch テスト）が
   通ることを確認。

## 残る論点

- ADR-010 の `&dyn EventSink` 方針は本 ADR の concrete 型で置き換える。
  ADR-010 を proposed のまま残し、実装時に本 ADR を優先する。
- 子スコープの registry 共有（上流整合）は本 ADR の第2段セクション（下記）で扱う。
  **現状の既知制約**: 上流では registry が root に1つだけ存在し子はそれを
  共有するが、ローカルは各 Context が独立 registry を持つため、
  子で登録した listener は親や兄弟から emit しても届かない。
  port 側では cross-scope 購読を避ける規約を取る（R1 調査で確認）。
- isolate は service 解決の影であり event 可視性を変えない（上流整合）。
  EventScopeView に isolate 情報は入れず、`{ registry, filter }` の形を維持する。
- 第2段の最初の一歩（実装済み 2026-11-21）: 新規フィールドは追加せず、
  `Context::root_event_registry() -> &EventRegistry` を pub(crate) accessor として
  追加（R1 で Arc/Weak 保持は不要と判定。parent が强参照なので borrow は安全）。
  dispatch は現行の `event_registry()` のままで変更なし。次ステップで
  named emit → typed descriptor の順に shared へ移行する。

### 第2段の第二段: named emit の shared registry 移行（設計、未実装・Trigger 待ち）

**下記の移行手順は Trigger が来るまで実装しない**（現状は独立 registry を
維持）。上流の root 共有セマンティクスへ近づける必要が生じた場合に、
named emit の4経路を `root_event_registry()` へ集約する。R1 分析（reviewer）に基づく移行順序:

1. `on_with` の登録先を `root_event_registry().clone()` に変更。
2. `emit` / `emit_with` の snapshot を root へ。`emit_with` の view は
   `EventScopeView::new(self.root_event_registry(), this.event_filter())` とし、
   **filter は `this` のまま**（2スコープ契約を維持）。
3. `once` の `OnceRemover.registry` も root へ統一（登録先と削除先の不一致で
   一度きり semantics が壊れるのを防ぐ）。
4. Disposer の cleanup も root registry を clone したままにする（ghost listener 防止）。

**テスト前提の反転（重要）**: 現在の unit test
`root_event_registry_is_shared_and_dispatch_stays_local` は「子の listener が
root に現れない」ことを固定している。**移行時はこの期待を反転**させ、
「子で `on` した listener が root の `emit` で届く」を検証する。
一方 `tests/cordis.rs` の stage_b（登録も dispatch も root）は共有化しても
通るため、**「stage_b が通る＝移行 OK」と誤判定しないこと**。

**typed Event は別途**: `Event::on_emit` / `emit` 系（events.rs）は
`ctx.event_registry()` を直接見る。段5で切った events→context 依存を復活
させないため、host が `*_with_registry` 版を root registry で呼ぶ形にする
（typed descriptor の shared 移行はさらに別段階）。

**移行 Trigger（core-services-research の R1）**: 現行コードには cross-scope
の実需要が無く（named 登録・emit は同一 Context 对、tests も同一 ctx）、
独立 registry の裁定を即座に覆す根拠は無い。次のいずれかが要件に入った
時点で root 共有が必須となる: (1) port を増やし fiber ctx で購読して root
から emit する構成、(2) 上流 port 追加で cross-scope 購読が発生する構成。
スコープ制御は isolate ではなく `EventScopeView.filter` に集約する点は
上流と整合しており、isolate ごとの registry 分割は採らない。移行は
「accessor（実装済み）→ named のみ共有 → typed も共有」の3段で低リスク。

## 関連

- [ADR-010](ADR-010-context-events-boundary.md) — Context/events 境界（proposed）
- [ADR-009](ADR-009-context-plugin-boundary.md) — FiberObserver による依存逆転
- [ADR-006](ADR-006-context-events-migration.md) — 名前キーイベント API の段階移行
