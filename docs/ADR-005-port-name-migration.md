---
type: Reference
title: ADR-005 plugin_ports の名前移行
description: 汎用名から <PORT>_ 接頭辞名への移行と deprecated alias の方針
tags: [cordis-rs, adr, api-design, migration]
resource: https://github.com/Seduce0068/cordis-rs
generated: 2026-11-18T11:35:23+09:00
verified: 2026-11-18T11:35:23+09:00
status: accepted
---

## 決定

`plugin_ports` の汎用名を出所が分かる接頭辞名へ変更し、旧名は 1 リリースの
deprecated alias として維持する。型・定数の実体は同じなので値の等価性は壊れない。

| 旧名 | 新名 |
| --- | --- |
| `EVENT_TYPE` | `IMAGE_OFFLOAD_EVENT_TYPE` |
| `Decision` | `ImageOffloadDecision` |
| `SERVICE_NAME` | `TOOL_RESULT_PRUNER_SERVICE_NAME` |
| `PROJECTION_KEY` | `TIME_CONTEXT_PROJECTION_KEY` |
| `SessionEvent` | `TodoSessionEvent` |
| `Counts` | `TodoWriteCounts` |
| `Tools` | `TodoTools` |

## 根拠

`plugin_ports::*` と `core_services::*` を同時に glob import すると、
`SessionEvent` 等が曖昧になる。core_services は `LoggedSessionEvent` に
リネーム済みだが、plugin_ports 側も接頭辞を付けることで衝突そのものを解消する。
deprecated 属性は re-export 時のみに付け、port モジュール内の参照には警告を出さない。

## 移行スケジュール

1. 今回: 新名を追加し旧名を deprecated alias として公開
2. 次リリース: README / examples を新名へ統一
3. その後: 旧 alias を削除（旧名参照が0件になったコミット。0.x では minor が破壊変更に相当し v1.0.0 を待たない。ADR-008 参照）

全 deprecated 属性の note は ``use `NEW` instead`` 形式に統一する（2026-11-21）。
`since` は意図的に省略している: Cargo.toml の version は `0.1.0` のままで
リリースタグ（v0.204.0 等）と乖離しており、`since` にタグ番号を入れると
rustdoc が誤った情報を表示するため、バージョン方針が固まるまで付けない。
`emit_detached`（events.rs）は本 ADR の alias 表には無い rename（→ `emit_spawn`）
で、削除時期は未定義（参照0件なので安全に削除可能だが注記統一の範囲外）。

### v1.0.0 削除チェックリスト（2026-11-21 追加）

README が予告する v1.0.0 では、上記の「旧名 → 新名」7件すべての deprecated
re-export を `src/lib.rs` から削除する。削除前に次を確認する。

- [ ] 上記7 alias の旧名参照が `src/` / `tests/` / `examples/` のどこにも無いこと。
- [ ] `src/lib.rs` に `#[deprecated]` が残っていないこと（`grep -c '#\[deprecated\]' src/lib.rs` が 0）。
- [ ] README / ADR の「旧名 alias は v1.0.0 で削除予定」を「削除済み」へ更新すること。

削除は 0.x の minor 相当の破壊変更に該当するため、タグは v1.0.0 を待つ（ADR-008 準拠）。
`emit_detached` は本チェックリストの対象外（別途判断）。

## 検証

`cargo fmt --all --check` / `cargo check --all-targets` / `cargo clippy --all-targets -- -D warnings` /
`cargo test --all-targets`（142 tests）が成功すること。
