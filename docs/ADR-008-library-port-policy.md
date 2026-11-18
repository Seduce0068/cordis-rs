---
type: Reference
title: ADR-008 library port の受け入れ基準
description: pure library port を追加する際の判断基準と API 整理方針
tags: [cordis-rs, adr, library-port, api]
resource: https://github.com/Seduce0068/cordis-rs
generated: 2026-11-18T23:35:00+09:00
verified: 2026-11-18T23:35:00+09:00
status: accepted
---

## 決定

`src/ports/` への pure library port 追加は、次の受け入れ基準をすべて満たす場合のみ行う。

1. **重複しない**: 既存 port／`src/services/` に同一目的の helper が無い。类似処理が複数 port にインラインで散らばっている場合は、共通化が目的なら追加を認める。
2. **利用箇所がある**: 追加した関数を呼ぶテスト、host、または別 port が1つ以上存在する。単なる「upstream にあったから」だけでは追加しない。
3. **upstream 忠実**: upstream の MIT ソースを untrusted reference として読み、変換・検証ルールを its まま再現する。差分がある場合はコメントとテストで固定する。
4. **library port として登録しない**: `provide(_ctx)` は no-op とし、`ctx.provide` でサービスを増やさない。公開名にはドメイン接頭辞を付け、`plugin_ports` 直下に汎用名を出さない。
5. **port 数を KPI にしない**: port 数の増加はカバレッジであって完成度ではない。README の安定性マトリクスと併せて利用者に実装状況を伝える。

## 根拠

第2弾〜第3弾の健全性レビューで、50 port から60 port へ短期間で増加し、
`plugin_ports` に50超の re-export が Flat に並んだ。deprecated alias が7個残り、
利用者が旧名と新名のどちらを使うか判断できなくなった。

## deprecated alias の削除時期

既存の7個（`Decision` / `EVENT_TYPE` / `SERVICE_NAME` / `PROJECTION_KEY` /
`Counts` / `SessionEvent` / `Tools`）は後方互換のため残す。削除時期は日付や
semver major ではなく**イベント**で定義する: 旧名の参照が `rg` で0件になった
コミットで削除する。0.x プロジェクトでは minor が破壊変更に相当するため、
v1.0.0 を待たず次 minor で削除してよい（ADR-005 の「semver major」は
永久保留になり得るため、この方針で上書きする）。移行進捗は CI の
`-D warnings`（deprecated 使用で即失敗）で担保する。

## 棚卸し結果（2026-11-18）

60 port を走査した結果、実質的な呼び出し0件は「同一 port 内からしか呼ばれて
いない公開 helper」に絞られた（自動集計の23件の大半は `mount_*` alias の誤検知）。

- **private 化して削減**: `tool_pairing::balance_at` / `mcp_templates::parse_segments`
  / `llm_attribution::launch_suffix` / `session_persistence_jsonl::validate_session_id`
  / `agent_instructions::register_file`（外部参照は lib.rs re-export のみで削除済み）。
  再 public 化条件: 他 port / host / test から検証・登録の再利用が必要になった時点で、
  利用箇所を添えて ADR を更新してから export する。
- **legacy として残存**: `user_approval::APPROVAL_OUTCOMES` /
  `tool_fs::READ_LIMIT` / `STREAM_MIN_SIZE`（upstream parity 定数として value あり）。
- **現状維持**: `sandbox_policy::is_valid_path` / `sandbox_mode_fold`（tool 実行前
  validation で将来利用）、`goal_fold::apply_goal_event`（round 連続性検証で将来利用）、
  `compaction_checkpoint::COMPACT_CHECKPOINT_KIND` / `llm_message::CONTEXT_SUMMARY_MAX_CHARS`
  / `harness_dev_context` の prompt 定数（protocol／upstream 文言として価値あり）。

削除は「private 化 → re-export 削除」の2段階で実施し、CI の `-D warnings` で
残存参照を検知する。これは ADR-008 の「利用箇所が1つ以上」基準の事後適用であり、
「定義ファイル内だけで使われる helper は当初から public にすべきではなかった」
という判断に基づく。他 port から1件でも参照があれば private 化しない。

## 関連

- [ADR-001](ADR-001-cordis-runtime-reimplementation.md) — ライセンスと意図的な差分
- [README](../README.md) — 安定性マトリクスと推奨エントリポイント
