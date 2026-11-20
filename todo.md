---
type: Todo
title: "cordis-rs プロジェクトTODO"
description: "cordis-rs 固有のタスク。横断タスクはグローバルTODOへ。"
tags:
  - todo
  - tasks
  - "cordis-rs"
generated:
  by: opencode
  at: "2026-11-17T20:11:47+09:00"
verified: "2026-11-19T01:52:36+09:00"
status: current
---

## 進行中

- [x] context/plugin 依存逆転（ADR-009、R1 design + R2 coding 完了、2026-11-20）

## 未着手

### 優先度: 高

- [x] **with_llm_timeout の timeout 後に Error チャンクを無限生成する** — 2026-11-19 修正（TimeoutState::Terminated、R4 レビュー反映）。
- [x] **Session::append の seq 採番と Vec push が別ロックで並行順序が反転する** — 2026-11-19 修正（単一 Mutex の LogState に統合）。
- [x] **JSONL read/read_recovered が seq/type のスキーマ不正をセンチネル値に変換して fail-open** — 2026-11-19 修正（decode_line で seq/type/data を厳密検証、RecoveryReport に reason 追加）。
- [x] **Done なしで枯渇した LLM ストリームを部分テキストのまま成功として返す** — 2026-11-19 修正（Done 必須化、from_async_text に Done 追加、R4 指摘反映）。
- [x] **FileSystem::resolve が canonicalize 失敗時に未正規化パスへフォールバックし root 閉じ込めを弱める** — 2026-11-19 修正（最長既存 ancestor を canonicalize、検証済み canonical path を返却、R4 TOCTOU 指摘を README に明記）。
- [x] **context.rs と plugin.rs の双方向依存（レイヤ逆転）** — 2026-11-20 修正（FiberObserver trait で依存逆転、ADR-009、from_inner 削除）。
- [ ] **context ↔ events のモジュール循環** — 根拠 `src/context.rs:12`, `src/events.rs:29` / R1-AR / 重大度 high / confirmed
- [x] **effect の placeholder が no-op のまま await を跨ぎ teardown 中の登録が残存する** — 2026-11-19 修正（is_spent() で reentrant teardown を検知し children を unwind、R4 指摘反映）。
- [x] **invoke_blocking が tokio::time::sleep を含む retry を block_on で実行しデッドロックし得る** — 2026-11-19 修正（Tokio runtime 内からの呼び出しを fail-closed で拒否、R4 指摘反映）。
- [x] **!!js 式評価に再帰深度制限がなく深い入れ子でスタックオーバーフロー** — 2026-11-19 修正（evaluate_expression に limits/depth を伝播、R4 往復すり抜け指摘を反映）。

### 優先度: 中

- [x] **check-doc-counts.sh が ports 数を算出するだけでドキュメント検証していない** — 2026-11-19 修正（ports 検証と矛盾行ゼロ条件を追加、陳腐な 20/13 を更新）。
- [x] **ローカル verify.sh と CI ゲートの clippy 引数・extra_gates が乖離** — 2026-11-19 修正（--locked、ゲート順、audit、check-no-unsafe、check-doc-counts を同期、R4 指摘反映）。
- [x] **push_with がリスナー公開後に HookMeta を書き global 判定に競合窓** — 2026-11-19 修正（meta.insert を slot 挿入前に移動）。
- [ ] **型付き Event のディスパッチがスコープフィルタ／global を無視** — 根拠 `src/context.rs:355-387`, `src/events.rs:428-590` / R1-AP / 重大度 medium / confirmed
- [ ] **イベントレジストリがコンテキスト階層を継承せずサービス lookup と非対称** — 根拠 `src/context.rs:206-231,560-584` / R1-AP / 重大度 medium / confirmed
- [x] **Lifecycle に validation なしの公開 setter があり不正遷移／未知 u8 を Failed に丸める** — 検証せず却下（下記メモ参照）。
- [ ] **サービス削除 API が3系統存在し副作用とエラー意味が異なる** — 根拠 `src/context.rs:440-501` / R1-AP / 重大度 medium / confirmed
- [x] **キャンセル判定が実行前の1回のみで実行中の変化が伝播しない** — 2026-11-19 修正（共有 cancel_flag を追加し実行後に成功を再分類、retry/user_questions も対応、R4 指摘反映）。
- [x] **SessionProjectionRegistry::view が read guard 保持中にユーザ fold を呼ぶ** — 2026-11-19 修正（Arc を clone して guard を drop してから fold 実行、R1 指摘反映）。
- [x] **CheckpointCursor の pending/commit が TOCTOU で二重永続化し固定 tmp 名が競合** — 2026-11-19 修正（pending で cursor lock 内 snapshot、tmp 名に pid+連番、R1 指摘反映）。
- [x] **JsonlPersistence::create の path.exists → File::create に TOCTOU があり既存ログを truncate し得る** — 2026-11-19 修正（create_new(true) で原子的作成、AlreadyExists を正しく分類）。
- [x] **JsonlPersistence::list がメモリマップのみで再起動後にディスク上セッションを列挙できない** — 2026-11-21 修正（ディスクの .jsonl を統合、validate_session_id で逆変換フィルタ、R1 指摘反映）。
- [x] **restore が永続化 seq を破棄して再採番し seq が再起動をまたいで不安定** — 2026-11-21 修正（append_with_seq で seq 保持、単調性違反は Corrupt、R1 指摘反映）。
- [x] **VarTable::from_env が全環境変数を無制限に取り込み式から任意の環境変数を参照可能** — 2026-11-21 修正（from_env_allowlisted 導入、VarTable::get で集約、R1 敵対レビュー反映）。
- [x] **inject 名の内部化が Box::leak で無上限に蓄積** — 2026-11-21 修正（intern 名の検証・上限 512・長さ 128 を追加、R1 指摘反映）。
- [x] **errno 分類が Linux 固有の数値ハードコードで OS によって誤分类** — 2026-11-21 修正（ErrorKind 主軸＋raw errno フォールバック、R1 指摘反映）。
- [x] **永続化された有効/無効オーバーライドが mount 経路で反映されない** — 2026-11-21 修正（mount_with / mount_available_with を追加、既存 mount は adapter 化、R1 指摘反映）。
- [x] **状態ファイル破損時に警告なく YAML デフォルトへフォールバック** — 2026-11-21 修正（try_load で fail-closed、既存 load は後方互換 adapter）。
- [x] **未認識の設定キーがサイレントに破棄される** — 2026-11-21 修正（Composition::warnings と CompositionWarning を追加、PartialEq は entries のみ比較、R1 指摘反映）。
- [x] **PreparedExtensions::accept が Mutex ガード保持中に外部コールバックを実行し再入デッドロック** — 2026-11-21 修正（AcceptState + Condvar で並行冪等性を維持しつつガードを drop、R1 指摘反映）。
- [x] **schedule edit が set_status_public の失敗を握り潰して成功応答を返す** — 2026-11-21 修正（ToolError::Execution へ伝播、R1 指摘反映）。
- [x] **FileSearch が I/O エラーを握り潰し不完全な検索結果を完全なものとして返す** — 2026-11-21 修正（IndexStatus アクセサで I/O エラー・truncated を公開、R1 指摘反映）。
- [ ] **goal_fold の厳密キー一致がスキーマ進化を硬直化させる** — 根拠 `src/ports/goal_fold.rs:247-282,494-500` / R2-DI / 重大度 medium / likely
- [x] **同名 projection の登録が重複許容で disposer が全件削除・view が先頭のみ使用** — 2026-11-21 修正（世代 id による置換と fencing、R1 指摘反映）。

### 優先度: 低

- [ ] **!!js ブロックスカラーの本文が eprintln! で stderr にそのまま出力される** — 根拠 `src/loader.rs:943` / R3-IV / 重大度 low / confirmed
- [ ] **tokio の exact pin が dependencies と dev-dependencies で二重定義** — 根拠 `Cargo.toml:18,21` / R1-CF / 重大度 low / confirmed
- [ ] **smoke 対象 example が手動配列で実体と drift しうる** — 根拠 `scripts/smoke-examples.sh:5-13` / R1-CF / 重大度 low / confirmed
- [ ] **deprecated re-export に since がなく注記書式も不統一** — 根拠 `src/lib.rs:62-75`, `src/events.rs:522-529` / R1-AP / 重大度 low / confirmed
- [ ] **EmitErrors が件数しか表示せず source() も実装しない** — 根拠 `src/events.rs:104-114` / R1-AP / 重大度 low / confirmed
- [ ] **YAML マッピングの重複キーが黙って上書きされる** — 根拠 `src/loader.rs:1131-1176,1203-1213` / R3-IV / 重大度 low / confirmed
- [ ] **コメント除去とインデント正規化がブロックスカラー結合前に適用され式本文が変わる** — 根拠 `src/loader.rs:587-606,928-969` / R3-IV / 重大度 low / confirmed
- [ ] **SkillRegistry::catalog が provider 失敗を空カタログとして握り潰す** — 根拠 `src/services/skills.rs:68-81` / R2-ER / 重大度 low / confirmed

## 完了

- [x] `todo.md` をプロジェクト内へ配置（2026-11-17）

## メモ

- **残課題（2026-11-21、R1 指摘）**: `ScheduleError::Cancelled` は定義と Display のみで生成箇所がなく、`set_status()` に「cancelled な schedule は再開不可」ガードが無い。上流セマンティクスが cancel を終端とするなら別タスクで対応が必要。今回の失敗伝播では届かない。

- **残課題（2026-11-21、R1 指摘）**: inject 名の intern pool には上限 512 を設定したが、`FunctionPlugin` / `PluginFactory` の `Vec<&'static str>` は変更していない。R1 が提案した `inject_owned` 局所解（`Plugin` に default メソッドを追加し Fiber が優先して読む）は今回見送り。動的に異なる名前を大量生成する host では将来対応が必要。

- **残課題（2026-11-19、R1 指摘）**: `JsonlPersistence::create()` は `create_new()` 成功後に `state.handles` へ登録するが、その隙間で同じ id の `open()` が別ハンドルを登録し得る（両者が同じファイルへ append）。完全解消には `create_new()` と `handles.insert()` を単一の `state.write()` クリティカルセクションで覆う必要がある。今回は truncate 防止（R2-DI）に限定したため未対応。

- **ゴール上限到達（2026-11-19、256 ラウンド）**: 本セッションの自律ループは上限に達した。残り未着手30件は次セッション以降の引き継ぎ。優先順位: (1) 高優先度の context/plugin 依存逆転・context/events 循環は設計変更を伴うため、まず ADR 化してから段階実装する。(2) 中優先度のうち限定範囲で安全なもの（型付き Event のスコープフィルタ無視、イベントレジストリの階層継承、!!js コメント正規化、YAML 重複キー等）から順に swarm-coding を継続する。(3) discovery 未レビュー分（swarm 出力切り詰め）の代替走査も未実施。

- **却下（2026-11-19）**: 「Lifecycle setter に遷移検証を追加」— R4 レビューと Runtime Proof で確認。`plugin.rs` の refresh() は Unloading→Pending→Loading への復帰を正当経路として使い、Failed も終端ではない。単調性検証を入れると再ロード連鎖が死ぬ。ctx.lifecycle は fiber の guard/retiring が管理する真実の状態の観影であり、`From<u8>` の `_ => Failed` 丸めも fail-closed の安全網として機能しているため現状維持。

- **R4 補足（2026-11-19）**: `remove()` は slot→meta 順のままなので、push との対称性が崩れる残余競合がある。完全解消には `snapshot_with_ids` が global 判定を一緒に返す設計（`SlotEntry { listener, options }` への統合）が必要。ADR-006 の「meta 別管理」判断の再検討材料として残す。

- グローバルTODO: `~/.config/opencode/todo.md`（複数プロジェクト横断のタスク用）。
- **swarm コンボ R6 集約（2026-11-19）**: `swarm-project-review` R1〜R5 の20エージェントから67件を取得。重複5件を統合、却下1件、speculative 1件を下記へ。
  - 却下: 「多数のポートが Fiber::mount_retained の戻り値を破棄」— `src/plugin.rs:126-141` を確認し `mount_retained` 自体が fiber を effect で保持しているため誤検知（ADR-007 準拠）。
  - speculative: 「単調増加カウンタの u64 オーバーフロー」— `src/context.rs:185`, `src/services/tools.rs:146`, `src/services/sessions.rs:37`。到達に極端な回数が必要で測定根拠なし。
  - **未レビュー**: workflow 出力がハーネス側で切り詰められ `swarm-todo-discovery` の全件（R1-ST〜R5-VU）を取得できなかった。同一情報を再取得しない原則により未調査として記録。次回 goal で該当範囲を代替走査すること。
