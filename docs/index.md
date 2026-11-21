# Docs

cordis-rs の設計判断と開発資料の索引です。

## 設計判断

- [ADR-001: Cordis 互換ランタイムを Rust で再実装する](ADR-001-cordis-runtime-reimplementation.md)
  — 選択肢の比較と採用理由、意図的な差分、省略した機能（accepted）。
- [ADR-002: プラグイン port の実装範囲と意図的差分](ADR-002-tool-jobs-port-scope.md)
  — tool-jobs / pruner port で移植しなかった機能とその理由（accepted）。
- [ADR-003: fs-patch の実装選定](ADR-003-fs-patch-selection.md)
  — 既存 crate を採用せず std-only 実装を選んだ理由（accepted）。
- [ADR-004: 公開 seam の fail 動作](ADR-004-fail-semantics.md)
  — approval / timeout / checkpoint / dispose の既定 fail 動作と根拠（accepted）。
- [ADR-005: plugin_ports の名前移行](ADR-005-port-name-migration.md)
  — 接頭辞名への移行と deprecated alias の削除時期（accepted）。
- [ADR-006: Context 名前キーイベント API 移行](ADR-006-context-events-migration.md)
  — ctx.on/emit への段階移行と descriptor API との共存（accepted）。
- [ADR-007: port の Fiber 所有規約](ADR-007-fiber-ownership.md)
  — 全 port が従う Fiber 保持・解除の唯一の規約（accepted）。
- [ADR-008: library port の受け入れ基準](ADR-008-library-port-policy.md)
  — 追加基準、接頭辞規則、deprecated alias の削除時期（accepted）。
- [ADR-009: Context と plugin の依存境界](ADR-009-context-plugin-boundary.md)
  — FiberObserver による依存逆転（accepted）。
- [ADR-010: Context と events の依存境界](ADR-010-context-events-boundary.md)
  — 一方向化の設計。段5-6 完了（direct import 除去）だが、段4（残り dispatch API の
  registry 化）は未着手で真の cycle 解消は未達（proposed）。
- [ADR-011: EventScopeView によるスコープ表現](ADR-011-event-scope-view.md)
  — emit_with の2スコープ問題の設計と root registry 共有の段階移行（accepted）。
- [ADR-012: model 由来 Shell コマンドの承認境界](ADR-012-shell-command-approval.md)
  — tool boundary の承認ゲートで command 本文を扱う設計（accepted）。

## 開発資料

- [実装進捗](PROGRESS.md) — 現行の到達点と未対応項目。
- [トラブルシューティング / 復旧 Runbook](troubleshooting.md) — 障害時の切り分け手順。

## 使い方

- [README](../README.md) — 実装一覧、検証コマンド、デモの説明。

文書を更新した場合は、変更内容を ADR または README に反映してください。
