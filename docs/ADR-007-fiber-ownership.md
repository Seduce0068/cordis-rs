---
type: Reference
title: ADR-007 port の Fiber 所有規約
description: 全 port が従う Fiber 保持・解除の唯一の規約
tags: [cordis-rs, adr, fiber, lifecycle]
resource: https://github.com/Seduce0068/cordis-rs
generated: 2026-11-18T17:10:00+09:00
verified: 2026-11-18T17:10:00+09:00
status: accepted
---

## 決定

すべてのプラグイン port の `mount` ヘルパは `Fiber::mount_retained()` を使い、
`Fiber::mount()` の戻り値を捨ててはならない（ghost fiber パターンの禁止）。

`mount_retained()` は Fiber を context の effect list に保持し、effect 解除時に
`fiber.unload()` を呼ぶ。これにより service / listener / projection / tool
disposer が fiber の effect scope とともに確実に解除される。

## 根拠

最終健全性レビューで 16 port が `let _fiber = Fiber::mount(...)` で Fiber を
捨てていることが判明。Fiber がドロップされると effect scope が cleanup なしで
消え、登録済み service / listener が unload 不能になる。同根バグは f64f328
（message_feedback / native_tool_executor の shadow）で既に発生済みだった。

## 規約

1. 登録は fiber の親 context（root）に対して行う。
2. Fiber は `mount_retained()` で effect list に保持する。
3. disposer は effect scope に積み、`?` で結果を伝播する。
4. `let _fiber` は CI ゲート（`scripts/check-no-unsafe.sh`）で検出して拒否する。

## 検証

`cargo test --all-targets`（159 tests（受諾時点の件数。現行値は README / PROGRESS を参照））と CI（audit / unsafe lint / ghost-fiber gate
含む）が成功すること。
