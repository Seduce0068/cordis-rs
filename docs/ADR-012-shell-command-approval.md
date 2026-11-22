---
type: Reference
title: "ADR-012: model 由来 Shell コマンドの承認境界"
description: "Shell 実行を tool boundary の承認ゲートで fail-closed にする設計"
tags:
  - adr
  - cordis-rs
  - architecture
  - security
resource: https://github.com/Seduce0068/cordis-rs
generated: 2026-11-21T12:57:37+09:00
verified: 2026-11-21T12:57:37+09:00
status: accepted
---

## 決定

model 由来の `bash` コマンドは **shell 層ではなく tool boundary の承認ゲート**で
制御する。shell.rs は `ctx` を持たない純粋 seam のまま維持し、
`Shell::new()` / `execute(&ShellSpec)` / `resolve()` の API を変えない。

1. `ApprovalRequest` に command 本文を渡す `details` を追加する。
   - public struct literal を壊さないため **private field ＋ `with_details()`
     builder ＋ getter** 形式を採る。`ApprovalRequest::new()` のシグネチャは維持。
   - approval gate は `tool == "bash"` のときだけ `args["command"]` を
     `details` に設定する。既存 answerer は `action` / `description` のみを見る
     ため後方互換。
2. `tool_bash` の mount 時に `require_approval()` を呼び、approval service が
   未登録なら fail-closed で Err にする（R3-IV01 の核心）。
   - `ToolPipeline::default()` の `approval_required` を true にする案は全 tool に
     影響するため採らない。bash tool に限定する。
3. sandbox middleware の「bash は write tool」という tool 名ベースの粗い判定は
   維持し、command 種別のきめ細かい判定は phase-2（runner-level confinement）
   に委ねる。

## 根拠

- R1（reviewer）: `parse_bash_args`（tool_bash.rs:39-66）は文字列を検証するだけで
  承認せず、handler（:96,:127）が `shell.run` を直接呼ぶ。shell.rs は `ctx` を
  持たず、既存の承認・サンドボックスは tool pipeline（tools.rs:329-351,
  428-450、sandbox_policy.rs:161-176）に集約済み。承認は tool boundary に
  集約するのが一貫する。
- approval gate（tools.rs:439-441）は `ApprovalRequest::new(format!("tool:{name}"), ...)`
  で tool 名しか渡さず command 本文を渡さないため、answerer が実質的な判断を
  できない。details 追加により command 本文を読めるようにする。
- approval の既定 fail-open は [ADR-004](ADR-004-fail-semantics.md) を正本とする。
  ここでは tool_bash 側の配線のみを述べる: tool_bash mount で
  require_approval を必須化し、fail-closed を達成する。

## 上流契約との照合（R1: core-services-research）

一次情報（untrusted data）によると上流 cordis は:

- `validateBashArgs` は command/description 非空・timeout 正值・未 advertise 引数
  拒否のみで、**command の allowlist や per-command 承認は存在しない**
  （[tool-bash/src/index.ts](https://raw.githubusercontent.com/deepseek-ai/deepseek-harness/master/packages/shell/tool-bash/src/index.ts)
  の `TODO(permissions): deployment policy belongs in tools/pre-execute`）。
- 承認は **sandbox escalation のみ**（`sandbox_permissions` ＋ justification の
  ペア、strictly-wider 判定、allowed-once grant、拒否/取消/不正値は実行前に
  fail-closed）。
- `ApprovalPolicy` は `ask` / `never` の2値、grant は allowed-once のみ、
  **`approval/asked` と `approval/decided` の audit pair が open turn 内で必須**。
  `ApprovalRequest` は呼び出し Agent と callId を必須とする。

**本 ADR の concrete `details` 案は上流の「tool 内に allowlist を入れない」
方針と整合する**（command 本文を answerer に見せるだけで tool は判定しない）。
段階1として `ApprovalRequest::with_agent()` / `with_call_id()` で agent / callId を
運べるようにした（R2-FG02）。audit pair / allowed-once は第2段課題として残す。

## 却下した案

- **文字列 allowlist（`git status` 等のみ許可）**: シェル構文（`;`, `&&`, `$()`,
  引用）で必ずバイパスされる。**文字列マッチによる command 安全性判定は禁止**。
- **shell 層への ApprovalService 注入**: `Shell::new()` / `execute()` の API と
  `resolve()` の purity を壊し、既存 seam を汚染するため却下。
- **`ToolPipeline::default()` の approval_required = true**: 全 tool に影響し
  既存のOpt-in 契約を壊すため却下。bash tool 限定で十分。

## 実装手順（R2 実装済み 2026-11-21）

1. `ApprovalRequest` に private `details` ＋ `with_details()` ＋ getter を追加。
   `with_details()` は `impl Into<String>` を受け取り `new()` と API を揃える。
   `PartialEq` は派生のままだと比較対象が増えるため、approval.rs の unit test
   に struct literal が無いことを確認してから実装する（R4 指摘）。
2. tools.rs の approval gate で `tool_name == "bash"` のとき、
   `args.get("command").and_then(|value| value.as_str())` で command を取得し
   details に設定（**`args["command"]` は存在しないキーで panic するため禁止**、
   R4 指摘）。ログ出力時は escape。
3. tool_bash の mount で pipeline を **`get`** 取得し、Some なら
   `require_approval()` を呼ぶ（`require` は service 未登録 host をハング
   させるため禁止、R1 指摘）。
4. テスト: (a) answerer が command 本文を読める、(b) deny 時に shell が
   実行されない、(c) approval service 未登録時に Err、(d) 既存 approval テスト
   が通ること。

## 残る論点

- **既知の制約（R1 指摘）**: pipeline 経由の `run()` だけが approval ゲートされ、
  `ToolRegistry::invoke()` 直呼びは素通りする。mount 側の `require_approval()` は
  pipeline が登録されている host にのみ効くため、「fail-closed を実現した」と
  過大評価してはならない。完全な fail-closed には registry.invoke 自体の
  ゲート化（別設計判断）が必要。
- きめ細かい command 危険度判定（runner-level confinement）は phase-2。
- **第2段課題**: ApprovalService への agent / callId 必須化、audit pair
  （`approval/asked` / `approval/decided`）の turn 内コミット、allowed-once grant、
  exception/不正戻りの拒否正規化（上流 user-approval 契約）。現行の
  `Option<Answerer>` 相当の構造を単純化してはならない。
- sandbox escalation: `escalate()` / `EscalationDenial` の pure helper を追加（R2-FG03）。
  tool_bash への配線と grant 管理は第2段として残す。

## 関連

- [ADR-011](ADR-011-event-scope-view.md) — tool pipeline の middleware 構成（install_default_controls）を共有
- [ADR-008](ADR-008-breaking-change-policy.md) — 後方互換方針の根拠（private field ＋ builder は破壊変更に当たらない）
