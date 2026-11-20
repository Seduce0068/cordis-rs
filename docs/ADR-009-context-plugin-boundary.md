---
type: Reference
title: "ADR-009: Context と plugin の依存境界を逆転する"
description: "context.rs が plugin::FiberInner を直接知る双方向依存を解消する設計"
tags:
  - adr
  - cordis-rs
  - architecture
resource: https://github.com/Seduce0068/cordis-rs
generated: 2026-11-20T19:35:00+09:00
verified: 2026-11-20T19:35:00+09:00
status: accepted
---

## 決定

`Context` が `plugin::FiberInner` を直接 import する双方向依存を、次の構成で一方向的へ逆転する。

1. `context.rs` に **observer trait** を定義する。
   `FiberObserver: Send + Sync` は `injects(name: &str) -> bool`、
   `is_retiring() -> bool`、`refresh() -> BoxFuture<'static, Result<()>>` を提供する。
2. `Context` は `FiberRegistry`（`Weak<dyn FiberObserver>` のリスト）を所有する。
   `register_fiber` の引数を `Weak<dyn FiberObserver>` に差し替える。
3. `plugin.rs` の `Fiber` が trait を実装し、`Weak<Self>` を登録する（現在の
   `Weak<FiberInner>` と同じ自己 prune 契約）。
4. `notify_fibers` / `drain_dependent_fibers` は **Context のファサードとして残す**。
   内部で trait 経由に切り替えるだけで、公開（pub(crate)）API と同期 remove →
   async drain の順序契約は不変。

## 根拠

- `src/context.rs:13` が `crate::plugin::FiberInner` を import し、
  `:510-558` で `Weak<FiberInner>` / `Fiber::from_inner` / `refresh` /
  `injects` / `is_retiring` を使用。`src/plugin.rs:8` が `Context` / `Lifecycle`
  を import しており、依存が双方向。
- reviewer の R1 分析: 案 (a) observer trait は依存を
  `context → 抽象 trait`、`plugin → context` に一方向化でき、pub(crate) API
  と drain 契約を維持できる。コスト・リスクが完全外部化 (b) より大幅に小さく、
  Disposer 移動 (c) より実際に効く（(c) は FiberInner import を残すので解消にならない）。
- core-services-research の R1 分析: 完全外部化は4条件を同時に満たす必要があり破綻しやすい。
  (1) root スコープからの weak 解決、(2) drop 済み fiber の自動 prune、
  (3) `injects` / `is_retiring` による絞り込み（2フェーズ drain のデッドロック回避）、
  (4) 強参照の単一所有（`mount_retained` の effect と二重所有しない）。
  Context 所有の registry ＋ ファサード残留ならこれらを維持できる。

## 検討した代替案

| 案 | 評価 |
| --- | --- |
| (a) `Weak<dyn FiberObserver>` ＋ Context 所有 registry | **採用**。依存逆転・API 不変・契約維持。Weak 自己 prune の再実装が必要。 |
| (b) Fiber 追跡を plugin 層へ完全外部化 | 却下。service 変更通知を named event 化する必要があり、同期 remove と async drain が矛盾。ghost fiber 復活リスクが高い。 |
| (c) Disposer を共通 primitives へ移動 | 却下。cycle の原因は `FiberInner` import であり、`plugin.rs` は Disposer を import していないので解消にならない。 |

## 移行手順

1. `context.rs` に `FiberObserver` trait と `FiberRegistry`（Weak リスト＋自己 prune）を定義。
2. `plugin.rs` の `Fiber` に trait を実装し、`register_fiber` の呼び出しを差し替え。
3. `notify_fibers` / `drain_dependent_fibers` の内部を trait 経由へ。ファサード署名は維持。
4. 回帰テスト: Fiber drop 後に observer がスキップされること、retiring fiber が
   drain で無視されないこと、inject 依存の unload/reload（`tests/cordis.rs:190-215`）が通ること。

## 残る論点

- `Fiber` の強参照は `mount_retained` の effect が保持する（ADR-007）。registry は Weak のみ持ち、
  強参照の所有者を1つに固定する規約を維持する。
- `dispose_effects` と `Fiber::unload` の相互呼び出しは現状のまま（trait 化では触らない）。
  循環の完全解消は別案件。
- plugin-ports の R1 知見: 上流 cordis は Context が core の `Fiber` プリミティブを
  持つ一方、plugin 管理は `RegistryService` 側にある。完全外部化 (b) は上流に近いが、
  まず型依存を切る (a) が費用対効果で優先。observer は **Context を再入しない**
  ことと `drain` が `async` のままであることをテストで固定する。registry 化は
  第2段 ADR で「observer を registry へ移す」方針として記録する。

## 関連

- [ADR-001](ADR-001-cordis-runtime-reimplementation.md) — Cordis 互換ランタイムの再実装
- [ADR-007](ADR-007-fiber-ownership.md) — Fiber 所有権の単一化
