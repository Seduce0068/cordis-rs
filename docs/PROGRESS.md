---
type: Reference
title: "cordis-rs 実装進捗"
description: "Cordis 互換ランタイム再実装の到達点（100 ラウンド時点）"
tags:
  - cordis-rs
  - progress
generated:
  at: "2026-11-17T21:56:29+09:00"
status: current
---

## 概要

DSH の GPUI 再実装における最大の障壁だった Cordis プラグインを、Rust の
互換ランタイム `cordis-rs` と公式プラグインの port で置き換える取り組み。
100 ラウンド時点の到達点を記録する。

## 成果

- コアサービス 38 個（service 37 module + port 経由の `messageFeedback`）
- 再利用 port 20 個、library port 1 個（`hook-protocol`）
- テスト 175 件（5 テストファイル）、example 7 個
- GitHub タグ `v0.1.0` 〜 `v0.151.0`、commits 216
- ライセンス: AGPL-3.0-or-later（上流 MIT 通知は THIRD_PARTY_NOTICES.md に保持）
- 上流 `snapshots/sdk/text-turn/cordis.yml` の 6 エントリすべてを mount

## ランタイム

- Context（root / child / isolate）、typed event（5 dispatch モード）、
  reversible effect、Fiber ライフサイクル
- 軽量 YAML loader と `!!js` 式評価（比較・算術・論理・三項・
  文字列メソッド・配列／オブジェクト／テンプレートリテラル）
- agent loop（systemPrompt / goals / compaction / approval / retry /
  timeout を接続）

## GPUI クライアント

- 別リポジトリ `dsh-gpui-client-poc` に 18 bin
- todo / fs / agent / prompt / 実 composition / plugin トグル /
  権限フロー / ACP ストリーム / retry / session title を描画
- `scripts/smoke-gui.sh` で 12 GPUI デモの起動を検証

## 検証

- `scripts/verify.sh`: fmt → check → clippy → test → example smoke
- CI は共通ワークフロー `Seduce0068/workflows` の `rust-gates.yml@v1` に委譲
- smoke script を `extra_gates` に接続

## 未対応

- 公式プラグイン 332 個の全移植（現状 53 port）
- Schemastery DSL の完全互換とファイル監視 HMR
- OTLP 実送信（`ctx.otel` は in-memory レコードのみ）
- モデル側残高不足により ACP ストリーミング本文の実描画は未検証
