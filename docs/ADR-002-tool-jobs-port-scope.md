---
type: Reference
title: ADR-002 プラグイン port の実装範囲と意図的差分
description: tool-jobs / pruner port で意図的に移植しなかった機能とその理由
tags: [cordis-rs, adr, plugin-port, jobs]
resource: https://github.com/Seduce0068/cordis-rs
generated: 2026-11-18T06:38:40+09:00
verified: 2026-11-18T06:38:40+09:00
status: accepted
---

## 決定

`@deepseek-ai/dsh-tool-jobs` の Rust port では、3ツール（`job_output` / `job_list` / `job_kill`）、
ステータス行描画、待機タイムアウト、kill の2分岐、caller による job 分離を実装する。
一方、completion notice 配信とモデル cursor 追跡は第2段階とし、今回は意図的に除外する。

## 根拠

ローカル `services/jobs.rs` は `start` / `get` / `view` / `list` / `read` / `kill` / `wait` と
`JobStatus` を提供するが、upstream の `attachController` / `jobs.events.subscribe` /
`outputLimitBytes` / model cursor に相当する仕組みが無い。
これらを port 内で簡易キューとして再実装すると、コアサービスの API と二重の状態を持ち、
不整合の原因になるため、まず3ツールを確定実装し、残りは separate change で扱う。

## 意図的な差分

- **completion notice 配信**: 未実装。settled ジョブの未回収通知は、ローカルでは
  `job_list` でユーザーが明示的に確認する運用を取る。
- **user steer による早期終了**: 未実装。ローカル `kill` は状態を反転させるだけで
  実プロセスを停止しないため、描画文言も "requested cancellation" に留める。
- **待機の実装**: upstream の非同期 wait に対し、ローカル `wait` は
  `Condvar::wait_timeout_while` なので `spawn_blocking` で async worker を塞がないようにした。
- **caller 規約**: `tool_bash` の background job は owner を `exec.session` に登録する
  （2026-11-21 実装、swarm review R2-FG06）。`tool_jobs` は従来どおり `session_id`
  引数を caller として渡し、一致する session だけが view / wait / kill できる。
  `exec.session` が無い直接 invoke では owner は `None` となり、既存の owner 無し
  挙動（caller 比較なし）を維持する。併せて `max_concurrent` の枠がグローバルな
  `"bash"` 単位から session 単位に変わった点に注意。

## 補遺: compaction-tool-result-pruner の注入 seam

レビューで、ローカル `CompactionEngine` は自前の `ToolResultPruner` を保持し、
共有 `toolResultPruner` サービスを注入する seam が無いことが判明した。
今回の port では (b) の方針を採り、`prune_session` が `tool/result` イベントを
直接 prune して shadow-price → replacement の隣接列を append する。
`CompactionEngine::with_pruner()` の追加は `src/services/compaction.rs` を伴うため別案件とする。

また、元イベントと置換イベントの両方が残るため、素朴な projection / token meter は
二重カウントしうる。置換イベントには `sourceEventSeqs` と `op: replace` を付与して
後から元を除外できるようにしてあるが、pressure 再判定は `TokenMeter` 側の責務であり、
本 port は character budget を token budget とみなさない。

## 補遺: port fiber の強参照保持

`tool_fs` / `tool_str_replace_editor` の `mount()` は `Fiber` を ctx の effect list に
保持する。Context の fiber registry は `Weak` のみなので、mount ハンドルを
捨てると fiber がドロップされ dependent refresh が走らなくなる（実測で確認）。
effect list → Disposer → cleanup closure → Fiber → child Context → parent Context
という強参照サイクルができるが、root context はプロセス生存なので実害は限定的。
scoped context では `dispose_effects()` まで解放されない点を設計上受け入れる。
`unload()` は終端で `retiring` は戻らないため、reload は notify 経由でのみ機能する。

## 補遺: mcp-resources のスコープ分離

レビュー指摘で、グローバル 1 枚の provider マップでは他エージェントのリソースが
読めることが判明した。`ScopeKey`（Global / Agent / Session / Both）単位で
provider を保持し、呼び出しは Both → Global の順にフォールバックして解決する。
既存 `register()` は Global 登録として後方互換を保ち、新規 `register_scoped()` が
スコープと URI scheme allowlist を受け取る。scheme allowlist は upstream に無い
ローカル拡張であり、scheme だけでは `file://` 配下の `../` は防げない点に注意。
AgentLoop が `ToolExecContext::agent("agent-1")` 固定であるため、実効的な分離には
呼び出し側が正しい agent/session を渡す改修が別途必要である。

## 検証

`cargo fmt --all --check` / `cargo check --all-targets` / `cargo clippy --all-targets -- -D warnings` /
`cargo test --all-targets`（140 tests）および GitHub Actions が成功していること。
