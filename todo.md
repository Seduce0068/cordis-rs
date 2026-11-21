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
- [x] context ↔ events 設計＋EventScopeView 実装（ADR-011 accepted、2026-11-21）

## 未着手

### 優先度: 高

- [x] **FiberInner.retiring が true のまま戻らず依存 drain から永久除外される** — 2026-11-21 修正（Pending 復帰時に retiring を false へ、明示 unload は true 維持、R1 指摘反映）。
- [ ] **Shell が model 由来 command を無検証で sh -c に渡す（承認・allowlist なし）** — 根拠 `src/services/shell.rs:99-126`, `src/ports/tool_bash.rs:47-55,102-108` / R3-IV01 / 重大度 high / confirmed
- [x] **ShellRequest::with_timeout が実行経路で未使用でハングコマンドが無期限ブロック** — 2026-11-21 修正（run_with_timeout を追加し tokio::process + timeout で kill/reap、R1 指摘反映）。
- [x] **既定 fs サービスがルート無制限の FileSystem::local()（パス閉じ込め opt-in）** — 2026-11-21 対応（provide_fs_scoped を追加し host が明示選択、scoped root を構築時に canonicalize 固定、既存 provide_fs は互換維持、R1 敵対レビュー反映）。
- [x] **DeepSeekConfig.base_url が任意ホストへ API キー付きリクエストを送信** — 2026-11-21 修正（validated_endpoint で https＋api.deepseek.com allowlist、userinfo/path trick 拒否、送信前に検証、R1 敵対レビュー反映）。
- [ ] **inject 宣言_port 4個が apply() 空で依存喪失時に service/tool が残存** — 根拠 `src/ports/goal_round_driver.rs:174-180`, `src/ports/compaction_automation.rs:150-156`, `src/ports/user_questions.rs:190-196`, `src/ports/sandbox_policy.rs:209-215` / R1-ST01 / 重大度 high / confirmed
- [x] **ゴーストファイバー検出ゲートが実コードパターンと一致せず CI を通過させる** — 2026-11-21 修正（生の Fiber::mount 戻り値破棄を検出、mount_retained は exact match で除外、self_test 追加、R1 指摘反映）。

- [x] **with_llm_timeout の timeout 後に Error チャンクを無限生成する** — 2026-11-19 修正（TimeoutState::Terminated、R4 レビュー反映）。
- [x] **Session::append の seq 採番と Vec push が別ロックで並行順序が反転する** — 2026-11-19 修正（単一 Mutex の LogState に統合）。
- [x] **JSONL read/read_recovered が seq/type のスキーマ不正をセンチネル値に変換して fail-open** — 2026-11-19 修正（decode_line で seq/type/data を厳密検証、RecoveryReport に reason 追加）。
- [x] **Done なしで枯渇した LLM ストリームを部分テキストのまま成功として返す** — 2026-11-19 修正（Done 必須化、from_async_text に Done 追加、R4 指摘反映）。
- [x] **FileSystem::resolve が canonicalize 失敗時に未正規化パスへフォールバックし root 閉じ込めを弱める** — 2026-11-19 修正（最長既存 ancestor を canonicalize、検証済み canonical path を返却、R4 TOCTOU 指摘を README に明記）。
- [x] **context.rs と plugin.rs の双方向依存（レイヤ逆転）** — 2026-11-20 修正（FiberObserver trait で依存逆転、ADR-009、from_inner 削除）。
- [x] **context ↔ events のモジュール循環** — 2026-11-21 段4-5 完了（dispatch API の registry 化＋Disposer を primitives へ移動、direct import 削除。型依存は re-export 経由で残存）。
- [x] **effect の placeholder が no-op のまま await を跨ぎ teardown 中の登録が残存する** — 2026-11-19 修正（is_spent() で reentrant teardown を検知し children を unwind、R4 指摘反映）。
- [x] **invoke_blocking が tokio::time::sleep を含む retry を block_on で実行しデッドロックし得る** — 2026-11-19 修正（Tokio runtime 内からの呼び出しを fail-closed で拒否、R4 指摘反映）。
- [x] **!!js 式評価に再帰深度制限がなく深い入れ子でスタックオーバーフロー** — 2026-11-19 修正（evaluate_expression に limits/depth を伝播、R4 往復すり抜け指摘を反映）。

### 優先度: 中

- [ ] **依存 fiber の refresh 失敗が notify/drain 両経路で握り潰される** — 根拠 `src/context.rs:497-520`, `src/plugin.rs:112-120` / R2-ER02 / 重大度 medium / confirmed
- [ ] **CompositionState::save が固定 .tmp 名で並行保存競合（sessions.rs は修正済み）** — 根拠 `src/loader.rs:222-235`, `src/services/sessions.rs:329-342` / R2-BV03 / 重大度 medium / confirmed
- [ ] **Context::once が push と replace を別ロックで行い TOCTOU 二重発火の窓** — 根拠 `src/context.rs:276-302`, `src/events.rs:264-274` / R2-BV04 / 重大度 medium / likely
- [ ] **tool_todo の invoke_blocking に Tokio runtime 内呼び出しガードがない** — 根拠 `src/ports/tool_todo.rs:282-291`, `src/services/tools.rs:556-565` / R2-BV07 / 重大度 medium / likely
- [ ] **シェル出力 stdout/stderr が無上限でメモリ保持（DoS）** — 根拠 `src/services/shell.rs:136-142`, `src/ports/tool_bash.rs:82-94` / R3-IV05 / 重大度 medium / confirmed
- [ ] **read_window が bounded reads を謳いながらファイル全体を読み込む** — 根拠 `src/services/fs.rs:147-166` / R3-IV06 / 重大度 medium / confirmed
- [ ] **Hook マッチャが設定由来 regex を毎回コンパイルし失敗時に fail-open** — 根拠 `src/services/hook_protocol.rs:98-128` / R3-IV07 / 重大度 medium / confirmed
- [ ] **base64 添付デコードにサイズ上限がなく正規性チェックが全量再確保** — 根拠 `src/ports/attachment_admission.rs:56-80` / R3-IV08 / 重大度 medium / confirmed
- [ ] **otel_exporter の redaction が exporter 境界のみで events()/otlp_payload は生データ** — 根拠 `src/ports/otel_exporter.rs:22-65,96-113`, `src/services/telemetry.rs:104-110` / R4-OB01, R3-DE09 / 重大度 medium / confirmed
- [ ] **属性キー値に制御文字除去・長さ制限がなく OTLP/ログ注入と肥大化** — 根拠 `src/ports/otel_exporter.rs:13-38,101-108` / R4-OB02 / 重大度 medium / confirmed
- [ ] **セッションログの外部 API アップロード経路にマスキング機構がない** — 根拠 `src/ports/session_log_deepseek.rs:80-84,152-161` / R4-OB03 / 重大度 medium / confirmed
- [ ] **Exporter 失敗が完全に握り潰されテレメトリ障害自体が観測不能** — 根拠 `src/ports/otel_exporter.rs:58-64`, `src/services/telemetry.rs:49-54` / R4-OB04 / 重大度 medium / confirmed
- [ ] **OTLP タイムスタンプが固定 0 でイベント時刻が記録されない** — 根拠 `src/ports/otel_exporter.rs:101-105`, `src/services/telemetry.rs:14-19` / R4-OB05 / 重大度 medium / confirmed
- [ ] **counters が名前の高カーディナリティで無制限成長** — 根拠 `src/services/telemetry.rs:49-54,95-101` / R4-OB06 / 重大度 medium / confirmed
- [ ] **シークレットスキャンゲートが CI にも pre-commit にも存在しない** — 根拠 `.github/workflows/ci.yml:25-28`, `scripts/verify.sh:19-40` / R4-CI01 / 重大度 medium / confirmed
- [ ] **markdownlint がローカル専用で CI ゲートに含まれていない** — 根拠 `scripts/verify.sh:36-40`, `.github/workflows/ci.yml:25-28` / R4-CI02, R4-DP05 / 重大度 medium / confirmed
- [ ] **公式 plugin port テストが本番 port ではなくテスト内再実装を検証** — 根拠 `tests/official_plugin_port.rs:38-52`, `src/ports/command_compact.rs:18,34-38` / R5-TS01 / 重大度 medium / confirmed
- [ ] **固定 sleep で非同期状態遷移を待つテストが CI 負荷下でフレーク** — 根拠 `tests/cordis.rs:205,214,510,513`, `tests/config_loader.rs:360` / R5-TS02, R4-TQ03 / 重大度 medium / confirmed
- [ ] **JSONL 系テストの一時ディレクトリが PID のみで後始末されていない** — 根拠 `tests/core_services.rs:3419-3423,3928-3930,4173-4179` / R5-TS03 / 重大度 medium / confirmed
- [ ] **docs/index.md が ADR-002/003/004/006/007 を索引していない** — 根拠 `docs/index.md:5-17`, `docs/ADR-002-tool-jobs-port-scope.md:9` / R5-DC04 / 重大度 medium / confirmed
- [ ] **PROGRESS.md が status: current のまま旧スナップショット** — 根拠 `docs/PROGRESS.md:4,11,17,21-26` / R5-DC05 / 重大度 medium / confirmed
- [ ] **loader.rs が YAML 解析・式評価・検証・composition 構築の4責務（2102行）** — 根拠 `src/loader.rs:1,706,1220,2102` / R1-AR01 / 重大度 medium / confirmed
- [ ] **23個のドメイン Error enum が Error::Other に平坦化され種別分岐不能** — 根拠 `src/error.rs:4,30,34`, `src/loader.rs:108` / R1-AP02 / 重大度 medium / confirmed
- [ ] **port 初期化 API が mount と provide の二系統（lifecycle も不同）** — 根拠 `src/ports/tool_fs.rs:mount`, `src/ports/fs_patch.rs:provide` / R1-AP03 / 重大度 medium / confirmed
- [ ] **require_untyped が未登録サービスを無期限・無ログで待つ** — 根拠 `src/context.rs:555-571` / R5-VU09 / 重大度 medium / confirmed
- [ ] **GUI のエラー表示が次の成功後も消えない** — 根拠 `dsh-gpui-client-poc/src/bin/cordis_prompt_agent_ui.rs:165-187` / R5-VU01 / 重大度 medium / confirmed
- [ ] **smoke-gui.sh が外部 dsh / DeepSeek 認証を要するバイナリを含み失敗する** — 根拠 `dsh-gpui-client-poc/scripts/smoke-gui.sh:16-29,71-81` / R5-VU02 / 重大度 medium / likely
- [ ] **EventLogReporter が max_events 超過を無音破棄し消失検知手段がない** — 根拠 `src/services/telemetry.rs:77-102` / R5-VU04 / 重大度 medium / confirmed
- [ ] **破損 composition state が GUI で警告されず無効化設定が失われる** — 根拠 `src/loader.rs:238-283`, `dsh-gpui-client-poc/src/bin/cordis_entries_toggle_ui.rs:617-638` / R5-VU05 / 重大度 medium / confirmed
- [ ] **障害時トラブルシューティング / 復旧 runbook が存在しない** — 根拠 `docs/index.md:1-23`, `README.md:63-119` / R5-VU06 / 重大度 medium / likely

- [x] **check-doc-counts.sh が ports 数を算出するだけでドキュメント検証していない** — 2026-11-19 修正（ports 検証と矛盾行ゼロ条件を追加、陳腐な 20/13 を更新）。
- [x] **ローカル verify.sh と CI ゲートの clippy 引数・extra_gates が乖離** — 2026-11-19 修正（--locked、ゲート順、audit、check-no-unsafe、check-doc-counts を同期、R4 指摘反映）。
- [x] **push_with がリスナー公開後に HookMeta を書き global 判定に競合窓** — 2026-11-19 修正（meta.insert を slot 挿入前に移動）。
- [x] **型付き Event のディスパッチがスコープフィルタ／global を無視** — 2026-11-21 対応（emit の filter 無視を ADR-011 の意図的契約として明文化、emit_with は view 経由に変更）。
- [ ] **イベントレジストリがコンテキスト階層を継承せずサービス lookup と非対称** — 2026-11-21 設計完了（ADR-011 第2段: accessor 実装済み、named 移行は設計済み・Trigger 待ちで未実装）。cross-scope 実需要（fiber ctx 購読＋root emit）が発生した時点で移行する。
- [x] **Lifecycle に validation なしの公開 setter があり不正遷移／未知 u8 を Failed に丸める** — 検証せず却下（下記メモ参照）。
- [x] **サービス削除 API が3系統存在し副作用とエラー意味が異なる** — 2026-11-21 修正（共通 drop_service_entry ヘルパに集約、fiber 通知は各 API に残し使い分けを明文化、R1 指摘反映）。
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
- [x] **goal_fold の厳密キー一致がスキーマ進化を硬直化させる** — 2026-11-21 修正（必須キー存在チェックへ緩和、未知フィールド許容、version 厳密は維持、R1 指摘反映）。
- [x] **同名 projection の登録が重複許容で disposer が全件削除・view が先頭のみ使用** — 2026-11-21 修正（世代 id による置換と fencing、R1 指摘反映）。

### 優先度: 低

- [ ] **EventRegistry::slot が読み取りのたび空スロットを生成し動的イベント名で無限成長** — 根拠 `src/events.rs:196-203,264-274` / R2-BV09 / 重大度 low / confirmed
- [ ] **Error::Plugin が原因を文字列化し source チェーンと型情報が失われる** — 根拠 `src/error.rs:17-18`, `src/plugin.rs:118-119` / R2-ER05 / 重大度 low / confirmed
- [ ] **plugin_ports が71のフラット re-export を持ち公開 API が肥大** — 根拠 `src/lib.rs:43,113` / R1-AP04 / 重大度 low / likely
- [ ] **読み取り path が省略表記で src/ports 配下を辿れない（README 安定性マトリクス）** — 根拠 `README.md:88-90` / R5-DC07 / 重大度 low / confirmed
- [ ] **port 件数の定義が README と PROGRESS で食い違う** — 根拠 `README.md:31-32`, `docs/PROGRESS.md:22` / R5-DC08 / 重大度 low / confirmed
- [ ] **EventLogOptions に validation がなく batch_size=0 で除数判定が不成立** — 根拠 `src/services/telemetry.rs:21-34,99-101` / R4-OB07 / 重大度 low / confirmed
- [ ] **unsafe 禁止ゲートの走査範囲が狭く lint 設定と迂回可能** — 根拠 `scripts/check-no-unsafe.sh:6,15`, `.github/workflows/ci.yml:23` / R4-CI03 / 重大度 low / confirmed
- [ ] **smoke-examples.sh が example 0 件をそのまま成功と報告** — 根拠 `scripts/smoke-examples.sh:6-14` / R4-DP09 / 重大度 low / confirmed
- [ ] **MSRV (rust-version) が未宣言で edition 2024 要求が伝わらない** — 根拠 `Cargo.toml:1-8` / R4-DP07 / 重大度 low / confirmed
- [ ] **session reference のデコード上限欠如と markdown 引用の不完全エスケープ** — 根拠 `src/services/session_reference.rs:29-58` / R3-IV10 / 重大度 low / confirmed
- [ ] **compaction-automation の LLM 要約が pruning stub のまま（README に制約記載なし）** — 根拠 `src/ports/compaction_automation.rs:5-6`, `README.md:32,55` / R2-FG02 / 重大度 low / confirmed
- [ ] **hooks-claude-code の updatedInput/systemMessage/Ask/SessionStart が未接続** — 根拠 `src/ports/hooks_claude_code.rs:137-142` / R2-FG03 / 重大度 low / confirmed
- [ ] **mount_real_composition が部分失敗を assert で落とし診断レポートとして機能しない** — 根拠 `examples/mount_real_composition.rs:141-162` / R5-VU08 / 重大度 low / confirmed
- [ ] **README が mount_real_composition の動作を 2 箇所で矛盾説明** — 根拠 `README.md:137-138,149-150` / R2-FG04 / 重大度 low / confirmed
- [ ] **テスト用一時ディレクトリが PID 依存かつ手動削除で失敗時にゴミが残る** — 根拠 `tests/core_services.rs:594,3928`, `tests/config_loader.rs:705` / R4-TQ09 / 重大度 low / confirmed
- [ ] **GUI README が実装と矛盾する旧パス・旧バージョン記述** — 根拠 `dsh-gpui-client-poc/README.md:35,171`, `Cargo.toml:10` / R5-VU03 / 重大度 low / confirmed

- [x] **!!js ブロックスカラーの本文が eprintln! で stderr にそのまま出力される** — 2026-11-21 削除（YAML 任意文字列の stderr 生書き／ログ注入リスクも解消）。
- [x] **tokio の exact pin が dependencies と dev-dependencies で二重定義** — 2026-11-21 対応（feature union と pin 共有が意図的であることをコメント化、R1 指摘反映）。
- [x] **smoke 対象 example が手動配列で実体と drift しうる** — 2026-11-21 修正（find -maxdepth 1 + basename + sort で動的検出、R1 指摘反映）。
- [x] **deprecated re-export に since がなく注記書式も不統一** — 2026-11-21 修正（全8箇所の note を use X instead に統一、since 省略の理由を ADR-005 に記録、R1 指摘反映）。
- [x] **EmitErrors が件数しか表示せず source() も実装しない** — 2026-11-21 修正（details() と source() を追加、Display 文言は維持、R1 指摘反映）。
- [x] **YAML マッピングの重複キーが黙って上書きされる** — 2026-11-21 修正（entry レベルの重複を warnings に記録、last-wins 維持、R1 指摘反映）。
- [x] **コメント除去とインデント正規化がブロックスカラー結合前に適用され式本文が変わる** — 2026-11-21 修正（Line.raw を追加し block scalar 本文は元行を参照、R1 指摘反映）。
- [x] **SkillRegistry::catalog が provider 失敗を空カタログとして握り潰す** — 2026-11-21 修正（SkillCatalog::failures と SkillProviderFailure を追加、PartialEq は skills のみ比較、R1 指摘反映）。

## 完了

- [x] `todo.md` をプロジェクト内へ配置（2026-11-17）

## メモ

- **残課題（2026-11-21、R1 指摘）**: ブロック本文の空行は tokenize がドロップするため、literal block の `"a\n\nb"` が `"a\nb"` になる。空行を残すと既存パーサ（parse_map/parse_seq の indent ループ）が壊れるリスクがあり、block 専用の元行保持など別設計が必要。裸の `#` と相対インデントは Line.raw により保持されるよう修正済み。

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

- **swarm コンボ R6 集約（2026-11-21、round 38）**: `swarm-project-review`（R1-R5: architecture/API/error-handling/boundary/input-validation/deps-exposure/observability/CI/testing/docs/UX）と `swarm-todo-discovery`（R1静的シグナル/R3テスト・依存/R5価値運用）の8エージェントから約60件を取得。重複統合（固定sleep、markdownlint、otlp redaction）、却下2件: (1) 「tokio exact pin を外す」— 前回 R1 で xattr 0.2.3 衝突判断済みのため既存判断維持、(2) 「16箇所の `let _ = mount_retained` がゴーストファイバー」— `src/plugin.rs:174-188` を確認し mount_retained 自体が effect で fiber を保持しておりアンロードは動作（既存却下維持）。ただし `tool_fs.rs:318-321` のコメント（Weak 保持のためハンドル必須）とは内部矛盾するため要判断として記録。**CI ゲートの `let _fiber` パターン不一致は事実**なので gate 改善を high で採用。speculative 1件（services→ports 非対称依存）はメモ留め。
