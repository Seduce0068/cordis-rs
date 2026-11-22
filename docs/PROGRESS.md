---
type: Reference
title: "cordis-rs 実装進捗"
description: "Cordis 互換ランタイム再実装の到達点（100 自律ラウンド時点）"
tags:
  - cordis-rs
  - progress
generated:
  at: "2026-11-22T00:51:47+09:00"
status: current
---

## 概要

DSH の GPUI 再実装における最大の障壁だった Cordis プラグインを、Rust の
互換ランタイム `cordis-rs` と公式プラグインの port で置き換える取り組み。
100 自律ラウンド時点の到達点を記録する。

## 成果

- コアサービス 38 個（service 37 module + port 経由の `messageFeedback`）
- port 63 個（再利用 port。library port は hook-protocol を別枠で管理）
- テスト 283 件（統合 280 ＋ unit 3）、example 7 個
- GitHub タグ `v0.1.0` 〜 `v0.261.0`、commits 366
- ライセンス: AGPL-3.0-or-later（上流 MIT 通知は THIRD_PARTY_NOTICES.md に保持）
- 上流 `snapshots/sdk/text-turn/cordis.yml` の 6 エントリすべてを mount
- 直近の堅牢化: tool_todo サービス名分離（"todoTools"）、background job の
  session fence、Integer 範囲の i64 精確比較、bounded-wait API、
  gitleaks / markdownlint / doc-count / audit の CI ゲート整備

## ランタイム

- Context（root / child / isolate）、typed event（5 dispatch モード）、
  reversible effect、Fiber ライフサイクル
- 軽量 YAML loader と `!!js` 式評価（比較・算術・論理・三項・
  文字列メソッド・配列／オブジェクト／テンプレートリテラル）
- agent loop（systemPrompt / goals / compaction / approval / retry /
  timeout を接続）
- ADR-001〜012: 依存境界、port 範囲、fail 動作、名前移行、Fiber 所有、
  library port 基準、FiberObserver、EventScopeView、Shell 承認境界を整備
  （ADR-010 のみ proposed、他は accepted）

## GPUI クライアント

- 別リポジトリ `dsh-gpui-client-poc` に 18 bin
- todo / fs / agent / prompt / 実 composition / plugin トグル /
  権限フロー / ACP ストリーム / retry / session title を描画
- `scripts/smoke-gui.sh` で GPUI デモの起動を検証（外部依存デモは除外）

## 検証

- `scripts/verify.sh`: fmt → check → test → clippy → シークレットスキャン
  → markdownlint → cargo audit（未インストール時は `AUDIT_SKIP=1` 明示 opt-out 必須）
  → example smoke → no-unsafe → doc-counts（CI の `extra_gates` と同順）
- CI は共通ワークフロー `Seduce0068/workflows` の `rust-gates.yml@v1` に委譲
- `extra_gates`: smoke-examples / check-no-unsafe / check-doc-counts /
  check-secrets（gitleaks） / check-markdown（全 18 Markdown）
- pre-commit は gitleaks を主軸、CI スキャンを最終ゲートとして併設
- doc-count gate は src 内 unit test も計上し `cargo test -- --list` と一致

## 未対応

未着手タスクは todo.md を正とする（直近のスナップショットでは 20 件）。大半は設計変更を伴う大型項目:

- 公式プラグイン 332 個の全移植（現状 63 port）
- Schemastery DSL の完全互換とファイル監視 HMR
- loader.rs の4責務分割、23 個の Error enum の平坦化解消
- ToolRegistry::invoke 直呼びの approval ゲート化（ADR-012 第2段）
- completion notice 配信 / モデル cursor 追跡、sandbox escalation 接続
- JSONL 永続化の通常 turn への自動配線、FileSystem symlink TOCTOU
- OTLP 実送信（`ctx.otel` は in-memory レコードのみ。redaction / 上限 /
  失敗カウンタ / export 時刻は整備済み）
- モデル側残高不足により ACP ストリーミング本文の実描画は未検証
