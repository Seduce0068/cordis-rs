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

（なし）

## 未着手

### 優先度: 高

- [x] **with_llm_timeout の timeout 後に Error チャンクを無限生成する** — 2026-11-19 修正（TimeoutState::Terminated、R4 レビュー反映）。
- [ ] **Session::append の seq 採番と Vec push が別ロックで並行順序が反転する** — 根拠 `src/services/sessions.rs:60-73` / R2-DI,R2-BV / 重大度 high / confirmed
- [ ] **JSONL read/read_recovered が seq/type のスキーマ不正をセンチネル値に変換して fail-open** — 根拠 `src/ports/session_persistence_jsonl.rs:114-118,149-155` / R2-DI / 重大度 high / confirmed
- [ ] **Done なしで枯渇した LLM ストリームを部分テキストのまま成功として返す** — 根拠 `src/services/llm.rs:176-185` / R2-ER / 重大度 high / confirmed
- [ ] **FileSystem::resolve が canonicalize 失敗時に未正規化パスへフォールバックし root 閉じ込めを弱める** — 根拠 `src/services/fs.rs:83-90` / R2-ER / 重大度 high / confirmed
- [ ] **context.rs と plugin.rs の双方向依存（レイヤ逆転）** — 根拠 `src/context.rs:13,510-558` / R1-AR / 重大度 high / confirmed
- [ ] **context ↔ events のモジュール循環** — 根拠 `src/context.rs:12`, `src/events.rs:29` / R1-AR / 重大度 high / confirmed
- [ ] **effect の placeholder が no-op のまま await を跨ぎ teardown 中の登録が残存する** — 根拠 `src/context.rs:651-683` / R2-BV / 重大度 high / likely
- [ ] **invoke_blocking が tokio::time::sleep を含む retry を block_on で実行しデッドロックし得る** — 根拠 `src/services/tools.rs:428-453,501-509` / R2-BV / 重大度 high / likely
- [ ] **!!js 式評価に再帰深度制限がなく深い入れ子でスタックオーバーフロー** — 根拠 `src/loader.rs:1866-1872,1249-1254` / R3-IV / 重大度 medium / confirmed（DoS につき高に昇格）

### 優先度: 中

- [ ] **check-doc-counts.sh が ports 数を算出するだけでドキュメント検証していない** — 根拠 `scripts/check-doc-counts.sh:11-21` / R1-CF / 重大度 medium / confirmed
- [ ] **ローカル verify.sh と CI ゲートの clippy 引数・extra_gates が乖離** — 根拠 `.github/workflows/ci.yml:22-28`, `scripts/verify.sh:11-23` / R1-CF / 重大度 medium / confirmed
- [ ] **push_with がリスナー公開後に HookMeta を書き global 判定に競合窓** — 根拠 `src/events.rs:182-200` / R1-AP / 重大度 medium / confirmed
- [ ] **型付き Event のディスパッチがスコープフィルタ／global を無視** — 根拠 `src/context.rs:355-387`, `src/events.rs:428-590` / R1-AP / 重大度 medium / confirmed
- [ ] **イベントレジストリがコンテキスト階層を継承せずサービス lookup と非対称** — 根拠 `src/context.rs:206-231,560-584` / R1-AP / 重大度 medium / confirmed
- [ ] **Lifecycle に validation なしの公開 setter があり不正遷移／未知 u8 を Failed に丸める** — 根拠 `src/context.rs:15-36,246-253` / R1-AP / 重大度 medium / confirmed
- [ ] **サービス削除 API が3系統存在し副作用とエラー意味が異なる** — 根拠 `src/context.rs:440-501` / R1-AP / 重大度 medium / confirmed
- [ ] **キャンセル判定が実行前の1回のみで実行中の変化が伝播しない** — 根拠 `src/services/tools.rs:468-471` / R2-BV / 重大度 medium / confirmed
- [ ] **SessionProjectionRegistry::view が read guard 保持中にユーザ fold を呼ぶ** — 根拠 `src/services/sessions.rs:208-215` / R2-BV / 重大度 medium / likely
- [ ] **CheckpointCursor の pending/commit が TOCTOU で二重永続化し固定 tmp 名が競合** — 根拠 `src/services/sessions.rs:235-270` / R2-BV,R2-DI / 重大度 medium / likely
- [ ] **JsonlPersistence::create の path.exists → File::create に TOCTOU があり既存ログを truncate し得る** — 根拠 `src/ports/session_persistence_jsonl.rs:193-218` / R2-DI / 重大度 medium / likely
- [ ] **JsonlPersistence::list がメモリマップのみで再起動後にディスク上セッションを列挙できない** — 根拠 `src/ports/session_persistence_jsonl.rs:247-258` / R2-DI / 重大度 medium / confirmed
- [ ] **restore が永続化 seq を破棄して再採番し seq が再起動をまたいで不安定** — 根拠 `src/ports/session_persistence_jsonl.rs:261-272` / R2-DI / 重大度 medium / confirmed
- [ ] **VarTable::from_env が全環境変数を無制限に取り込み式から任意の環境変数を参照可能** — 根拠 `src/loader.rs:149-159,1632-1658` / R3-IV / 重大度 medium / confirmed
- [ ] **inject 名の内部化が Box::leak で無上限に蓄積** — 根拠 `src/loader.rs:414-427` / R3-IV / 重大度 medium / confirmed
- [ ] **errno 分類が Linux 固有の数値ハードコードで OS によって誤分类** — 根拠 `src/ports/fs_errno.rs:62-92` / R3-IV / 重大度 medium / likely
- [ ] **永続化された有効/無効オーバーライドが mount 経路で反映されない** — 根拠 `src/loader.rs:176-183,301-318` / R1-CF / 重大度 medium / confirmed
- [ ] **状態ファイル破損時に警告なく YAML デフォルトへフォールバック** — 根拠 `src/loader.rs:214-232` / R1-CF / 重大度 medium / confirmed
- [ ] **未認識の設定キーがサイレントに破棄される** — 根拠 `src/loader.rs:272-275,1131-1175` / R1-CF / 重大度 medium / confirmed
- [ ] **PreparedExtensions::accept が Mutex ガード保持中に外部コールバックを実行し再入デッドロック** — 根拠 `src/ports/deepseek_llm_api_extensions.rs:139-152` / R2-ER / 重大度 medium / likely
- [ ] **schedule edit が set_status_public の失敗を握り潰して成功応答を返す** — 根拠 `src/ports/tool_schedule.rs:120-126` / R2-ER / 重大度 medium / confirmed
- [ ] **FileSearch が I/O エラーを握り潰し不完全な検索結果を完全なものとして返す** — 根拠 `src/services/file_search.rs:143-161` / R2-ER / 重大度 medium / confirmed
- [ ] **goal_fold の厳密キー一致がスキーマ進化を硬直化させる** — 根拠 `src/ports/goal_fold.rs:247-282,494-500` / R2-DI / 重大度 medium / likely
- [ ] **同名 projection の登録が重複許容で disposer が全件削除・view が先頭のみ使用** — 根拠 `src/services/sessions.rs:179-196,207-218` / R2-BV / 重大度 medium / confirmed

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

- グローバルTODO: `~/.config/opencode/todo.md`（複数プロジェクト横断のタスク用）。
- **swarm コンボ R6 集約（2026-11-19）**: `swarm-project-review` R1〜R5 の20エージェントから67件を取得。重複5件を統合、却下1件、speculative 1件を下記へ。
  - 却下: 「多数のポートが Fiber::mount_retained の戻り値を破棄」— `src/plugin.rs:126-141` を確認し `mount_retained` 自体が fiber を effect で保持しているため誤検知（ADR-007 準拠）。
  - speculative: 「単調増加カウンタの u64 オーバーフロー」— `src/context.rs:185`, `src/services/tools.rs:146`, `src/services/sessions.rs:37`。到達に極端な回数が必要で測定根拠なし。
  - **未レビュー**: workflow 出力がハーネス側で切り詰められ `swarm-todo-discovery` の全件（R1-ST〜R5-VU）を取得できなかった。同一情報を再取得しない原則により未調査として記録。次回 goal で該当範囲を代替走査すること。
