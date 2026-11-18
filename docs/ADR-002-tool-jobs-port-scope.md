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
- **caller 規約**: 既存 `tool_bash` は owner を固定ラベル `"bash"` で登録するため、
  ツールは `session_id` 引数を caller として渡す。upstream の sessionId fence に比べると
  分離は弱く、producer 側の owner を `exec.session` に変更するのが今後の改善点である。

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

## 検証

`cargo fmt --all --check` / `cargo check --all-targets` / `cargo clippy --all-targets -- -D warnings` /
`cargo test --all-targets`（134 tests）および GitHub Actions が成功していること。
