# Docs

cordis-rs の設計判断と開発資料の索引です。

## 設計判断

- [ADR-001: Cordis 互換ランタイムを Rust で再実装する](ADR-001-cordis-runtime-reimplementation.md)
  — 選択肢の比較と採用理由、意図的な差分、省略した機能。
- [ADR-010: Context と events の依存境界](ADR-010-context-events-boundary.md)
  — EventSink による一方向化の設計（提案）。
- [ADR-009: Context と plugin の依存境界](ADR-009-context-plugin-boundary.md)
  — FiberObserver による依存逆転の設計（提案）。
- [ADR-008: library port の受け入れ基準](ADR-008-library-port-policy.md)
  — 追加基準、接頭辞規則、deprecated alias の削除時期。
- [実装進捗](PROGRESS.md) — 100 ラウンド時点の到達点と未対応項目。

## 使い方

- [README](../README.md) — 実装一覧、検証コマンド、デモの説明。

文書を更新した場合は、変更内容を ADR または README に反映してください。
