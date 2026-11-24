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
- [x] **Shell が model 由来 command を無検証で sh -c に渡す（承認・allowlist なし）** — 2026-11-21 R2 実装完了（ADR-012 accepted: ApprovalRequest details、approval gate が command 本文を escape して渡す、tool_bash mount は get 方式で require_approval。registry.invoke 直呼びは素通り既知制約）。
- [x] **ShellRequest::with_timeout が実行経路で未使用でハングコマンドが無期限ブロック** — 2026-11-21 修正（run_with_timeout を追加し tokio::process + timeout で kill/reap、R1 指摘反映）。
- [x] **既定 fs サービスがルート無制限の FileSystem::local()（パス閉じ込め opt-in）** — 2026-11-21 対応（provide_fs_scoped を追加し host が明示選択、scoped root を構築時に canonicalize 固定、既存 provide_fs は互換維持、R1 敵対レビュー反映）。
- [x] **DeepSeekConfig.base_url が任意ホストへ API キー付きリクエストを送信** — 2026-11-21 修正（validated_endpoint で https＋api.deepseek.com allowlist、userinfo/path trick 拒否、送信前に検証、R1 敵対レビュー反映）。
- [ ] **inject 宣言_port 4個が apply() 空で依存喪失時に service/tool が残存** — 根拠 `src/ports/goal_round_driver.rs:174-180`, `src/ports/compaction_automation.rs:150-156`, `src/ports/user_questions.rs:190-196`, `src/ports/sandbox_policy.rs:209-215` / R1-ST01 / 重大度 high / confirmed / 2026-11-21 設計判断: 完全移行は mount() -> Service シグネチャ変更を伴い呼び出し元多数を壊すため別 ADR 案件。require 化（R1 提案）は Runtime Proof で既存 fail-fast テストをハングさせることが判明し却下。user_questions/sandbox_policy は projection の has ガードで二重登録済み防止。
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

- [x] **依存 fiber の refresh 失敗が notify/drain 両経路で握り潰される** — 2026-11-21 修正（drain の join_all Result を集約し remove_service_async / generation に伝播、同期 remove_service は互換維持、notify の fire-and-forget は次段、R1 指摘反映）。
- [x] **CompositionState::save が固定 .tmp 名で並行保存競合（sessions.rs は修正済み）** — 2026-11-21 修正（pid + nanos 一意 tmp 名、状態フィールド追加なしで Clone/PartialEq 維持、R1 指摘反映）。
- [x] **Context::once が push と replace を別ロックで行い TOCTOU 二重発火の窓** — 2026-11-21 修正（EventRegistry::push_once で id 確保＋ラッパ登録を1ロック内でatomic化、R1 指摘反映）。
- [x] **tool_todo の invoke_blocking に Tokio runtime 内呼び出しガードがない** — 2026-11-21 修正（Handle::try_current で fail-closed、既存 ContextUnavailable バリアント流用、R1 指摘反映）。
- [x] **シェル出力 stdout/stderr が無上限でメモリ保持（DoS）** — 2026-11-21 修正（with_max_output_bytes builder、UTF-8 境界安全な切り詰め＋マーカー、run / run_with_timeout 両経路、R1 指摘反映）。
- [x] **read_window が bounded reads を謳いながらファイル全体を読み込む** — 2026-11-21 修正（BufReader::lines 1パスで全行保持せずカウント、total_lines 契约維持、R1 指摘反映）。
- [x] **Hook マッチャが設定由来 regex を毎回コンパイルし失敗時に fail-open** — 2026-11-21 修正（CompiledMatcher を追加し compile 1回で保持、不正は fail-closed non-match、既存 matches は adapter 化、R1 指摘反映）。
- [x] **base64 添付デコードにサイズ上限がなく正規性チェックが全量再確保** — 2026-11-21 修正（decode_canonical_base64_limited で decode 前に入力長拒否、saturating で overflow 防止、既存 API は adapter 化、R1 指摘反映。再エンコード除去は別段階）。
- [x] **otel_exporter の redaction が exporter 境界のみで events()/otlp_payload は生データ** — 2026-11-21 修正（otlp_payload が内部で redact、冪等、allowlist 値の制御文字も除去、R1 敵対レビュー反映）。
- [x] **属性キー値に制御文字除去・長さ制限がなく OTLP/ログ注入と肥大化** — 2026-11-21 修正（文字数ベース上限＋冪等マーカー、session_id は別上限 256、UTF-8 安全、R1 指摘反映）。
- [x] **セッションログの外部 API アップロード経路にマスキング機構がない** — 2026-11-21 修正（mask_event_data で deny-by-default allowlist、candidate 生成時に適用してバイト計算も正確、冪等、R1 敵対レビュー反映）。
- [x] **Exporter 失敗が完全に握り潰されテレメトリ障害自体が観測不能** — 2026-11-21 修正（RedactingExporter が AtomicU64 失敗カウンタを保持し failures() で観測、export は Ok 維持、clone 間共有、R1 指摘反映）。
- [x] **OTLP タイムスタンプが固定 0 でイベント時刻が記録されない** — 2026-11-21 修正（otlp_payload が export 時 UTC nanos を timeUnixNano に設定、EventRecord は不変で後方互換維持。per-event 時刻は別 ADR、R1 指摘反映）。
- [x] **counters が名前の高カーディナリティで無制限成長** — 2026-11-21 修正（with_max_counters builder で上限、超過名は `__other__` に集約、予約キー衝突防止、R1 指摘反映）。
- [x] **シークレットスキャンゲートが CI にも pre-commit にも存在しない** — 2026-11-21 修正（scripts/check-secrets.sh を CI extra_gates と verify.sh に追加、pre-commit 設定、--redact、未インストール時 fail、R1 敵対レビュー反映）。
- [x] **markdownlint がローカル専用で CI ゲートに含まれていない** — 2026-11-21 修正（check-markdown.sh を追加し全17 md を動的 lint、verify.sh と CI extra_gates に統合、CI は npx で v0.24.1 pin、R1 指摘反映）。
- [x] **公式 plugin port テストが本番 port ではなくテスト内再実装を検証** — 2026-11-21 修正（stub ファイルに目的明示、tests/production_ports.rs で本番 mount 3件を検証、R1 指摘反映）。
- [x] **固定 sleep で非同期状態遷移を待つテストが CI 負荷下でフレーク** — 2026-11-21 修正（wait_for_state ヘルパーと bounded deadline ポーリングに置換、2s/10ms、yield 確保、R1 指摘反映）。
- [x] **JSONL 系テストの一時ディレクトリが PID のみで後始末されていない** — 2026-11-21 修正（TempDirGuard: PID+nanos＋Drop で削除、`Deref<Target=Path>` で既存コード維持、R1 指摘反映）。
- [x] **docs/index.md が ADR-002/003/004/006/007 を索引していない** — 2026-11-21 修正（ADR-001〜012 の完全索引と status 表記を現状に合わせて再構成、R1 指摘反映）。
- [x] **PROGRESS.md が status: current のまま旧スナップショット** — 2026-11-21 修正（v0.245.0 / 331 commits / 237 tests / 67 ラウンド時点に更新、gitleaks / markdownlint ゲートを記載）。
- [ ] **loader.rs が YAML 解析・式評価・検証・composition 構築の4責務（2102行）** — 根拠 `src/loader.rs:1,706,1220,2102` / R1-AR01 / 重大度 medium / confirmed
- [x] **23個のドメイン Error enum が Error::Other に平坦化され種別分岐不能** — 2026-11-22 段階1修正（Error::Domain(&static str, String) 追加、ShellError/PersistenceError/ToolError に From 実装、Error::Other は不変、残り20個は段階移行、回帰テスト、R1 指摘反映）。
- [x] **port 初期化 API が mount と provide の二系統（lifecycle も不同）** — 2026-11-22 段階1修正（呼び出し0件の no-op fs_patch::provide を削除、README に mount（fiber 永続）／provide（純粋 service）の使い分けを明記、二系統の統一は v1.0 設計課題として記録、R1 指摘反映）。
- [x] **require_untyped が未登録サービスを無期限・無ログで待つ** — 2026-11-21 修正（require_untyped_with_timeout を追加、既存 require 85 箇所は不変、エラーに service 名と ctx 名、R1 指摘反映）。
- [x] **GUI のエラー表示が次の成功後も消えない** — 2026-11-21 修正（take_prompt で前回の error バナーをクリア、R5-VU01 反映）。
- [x] **smoke-gui.sh が外部 dsh / DeepSeek 認証を要するバイナリを含み失敗する** — 2026-11-21 修正（認証必須 / 外部 dsh 必須の2 bin を除外し手動起動を README に明記、R5-VU02 反映）。
- [x] **EventLogReporter が max_events 超過を無音破棄し消失検知手段がない** — 2026-11-21 修正（dropped_events カウンタと dropped() アクセサを追加、R5-VU04 反映）。
- [x] **破損 composition state が GUI で警告されず無効化設定が失われる** — 2026-11-21 修正（GUI を try_load() に統一、JSON 構文のみならスキーマ不正も検出、設定喪失を明示、R1 指摘反映）。
- [x] **障害時トラブルシューティング / 復旧 runbook が存在しない** — 2026-11-21 修正（doc-create スキルの runbook テンプレートで docs/troubleshooting.md を作成、index / README からリンク、R5-VU06 反映）。

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

- [x] **EventRegistry::slot が読み取りのたび空スロットを生成し動的イベント名で無限成長** — 2026-11-21 修正（try_slot 追加で snapshot 系は空スロットを生成しない、R2-BV09 反映）。
- [x] **Error::Plugin が原因を文字列化し source チェーンと型情報が失われる** — 2026-11-21 修正（`Plugin(String, #[source] Box<Error>)` で根本原因を保持、R2-ER05 反映）。
- [x] **plugin_ports が71のフラット re-export を持ち公開 API が肥大** — 2026-11-21 判断（削除せず現状維持を README に明記。glob import 0 件・名前衝突 0・cargo check 成功の Runtime Proof、旧名 alias は v1.0.0 方針維持）。R1-AP04 反映。
- [x] **読み取り path が省略表記で src/ports 配下を辿れない（README 安定性マトリクス）** — 2026-11-21 修正（6つの根拠パスを root 相対完全修飾に変更、R5-DC07 反映）。
- [x] **port 件数の定義が README と PROGRESS で食い違う** — 2026-11-21 検証の結果却下（両者とも「再利用 port 63、library port は hook-protocol 別枠」で整合済み。doc-count gate も tests=240 ports=63 で通過）。R5-DC08 の指摘は古いスナップショット Based。
- [x] **EventLogOptions に validation がなく batch_size=0 で除数判定が不成立** — 2026-11-21 修正（validate() と InvalidOptions、record() が不正設定を fail-closed で拒否、R4-OB07 反映）。
- [x] **unsafe 禁止ゲートの走査範囲が狭く lint 設定と迂回可能** — 2026-11-21 修正（全 Rust ソースを走査、unsafe extern パターン追加、benches 配下の検出を自己検証、R4-CI03 反映）。
- [x] **smoke-examples.sh が example 0 件をそのまま成功と報告** — 2026-11-21 修正（検出0件で明示エラー終了、R4-DP09 反映）。
- [x] **MSRV (rust-version) が未宣言で edition 2024 要求が伝わらない** — 2026-11-21 修正（rust-version = "1.85" を宣言、cargo metadata で確認、R4-DP07 反映）。
- [x] **session reference のデコード上限欠如と markdown 引用の不完全エスケープ** — 2026-11-21 修正（ペイロード長上限とエラーの URI 非露出、ラベルは [ ] ) 改行をエスケープ、R3-IV10 反映）。
- [x] **compaction-automation の LLM 要約が pruning stub のまま（README に制約記載なし）** — 2026-11-21 修正（安定性マトリクスに phase-1 行と解消条件を追加、R2-FG02 反映）。
- [x] **hooks-claude-code の updatedInput/systemMessage/Ask/SessionStart が未接続** — 2026-11-22 段階1修正（HookEffect::Ask 追加、MergedDecision::Ask をマップ、deny middleware は next() 委譲、install_default_controls も Ask をブロック対象外に、回帰テスト、R1 指摘反映。updatedInput/systemMessage と完全な approval 連携は残り）。
- [x] **mount_real_composition が部分失敗を assert で落とし診断レポートとして機能しない** — 2026-11-21 修正（assert を外し skipped/failed を id と理由付きで一覧出力、R5-VU08 反映）。
- [x] **README が mount_real_composition の動作を 2 箇所で矛盾説明** — 2026-11-21 修正（診断レポート化した実装に合わせ「6 エントリすべてを起動」を「port 存在分のみ起動＋failed/skipped 一覧」に修正、R2-FG04 反映）。
- [x] **テスト用一時ディレクトリが PID 依存かつ手動削除で失敗時にゴミが残る** — 2026-11-21 修正（残り18箇所を TempDirGuard に横展開、`AsRef<Path>` / Deref で互換維持、3ファイルでガード定義を同期、R4-TQ09 反映）。
- [x] **GUI README が実装と矛盾する旧パス・旧バージョン記述** — 2026-11-21 修正（状態パスを XDG state home 構成に、同期 tag を v0.255.0 に修正、R5-VU03 反映）。

- [x] **!!js ブロックスカラーの本文が eprintln! で stderr にそのまま出力される** — 2026-11-21 削除（YAML 任意文字列の stderr 生書き／ログ注入リスクも解消）。
- [x] **tokio の exact pin が dependencies と dev-dependencies で二重定義** — 2026-11-21 対応（feature union と pin 共有が意図的であることをコメント化、R1 指摘反映）。
- [x] **smoke 対象 example が手動配列で実体と drift しうる** — 2026-11-21 修正（find -maxdepth 1 + basename + sort で動的検出、R1 指摘反映）。
- [x] **deprecated re-export に since がなく注記書式も不統一** — 2026-11-21 修正（全8箇所の note を use X instead に統一、since 省略の理由を ADR-005 に記録、R1 指摘反映）。
- [x] **EmitErrors が件数しか表示せず source() も実装しない** — 2026-11-21 修正（details() と source() を追加、Display 文言は維持、R1 指摘反映）。
- [x] **YAML マッピングの重複キーが黙って上書きされる** — 2026-11-21 修正（entry レベルの重複を warnings に記録、last-wins 維持、R1 指摘反映）。
- [x] **コメント除去とインデント正規化がブロックスカラー結合前に適用され式本文が変わる** — 2026-11-21 修正（Line.raw を追加し block scalar 本文は元行を参照、R1 指摘反映）。
- [x] **SkillRegistry::catalog が provider 失敗を空カタログとして握り潰す** — 2026-11-21 修正（SkillCatalog::failures と SkillProviderFailure を追加、PartialEq は skills のみ比較、R1 指摘反映）。

- [x] **サービス名 "tools" が ToolRegistry と TodoTools で衝突し文字列キー127箇所に散在** — 2026-11-21 修正（tool_todo 側を `TODO_TOOLS_SERVICE_NAME` = "todoTools" に改名、provide/inject/apply 3箇所同時、25箇所の require は不変、両順序共存テスト追加、R1 指摘反映）。
- [x] **ToolRegistry::invoke 直呼びが approval ゲートを素通り** — 2026-11-22 判断（ゲート化せず現状維持。71件の invoke 直呼び互換を守り、pipeline.run() のみがゲートを通る旨を README に明記、R1 指摘反映）。
- [ ] **job kill が状態反転のみで実プロセスを停止しない** — 根拠 `docs/ADR-002-tool-jobs-port-scope.md:30-31`, `src/services/jobs.rs:275-282` / R2-FG04 / 重大度 medium / confirmed（意図的差分だが停止経路なし）
- [x] **provide-if-absent ガードが `get::<T>()` の型不一致を未登録と誤判定（6箇所）** — 2026-11-21 修正（6箇所を get_untyped() に置換、型衝突を正しく検出、回帰テスト追加、R1 指摘反映）。
- [x] **FieldSpec の min/max が f64 のみで Integer(i64) の範囲検証が精度落ち** — 2026-11-21 修正（min_int/max_int + range_int を追加、i64 同士で比較、既存 range(f64) は維持、2^53 超回帰テスト追加、R1 指摘反映）。
- [x] **check-doc-counts.sh が src 内 unit test を件数から除外** — 2026-11-21 修正（src+tests の総数 247 に統一し内訳明記、stale test count チェック追加、cargo test --list と一致、R1 指摘反映）。
- [x] **verify.sh が cargo-audit 未インストール時に警告だけでスキップし全ゲート合格と表示** — 2026-11-21 修正（未インストール時は AUDIT_SKIP=1 の明示 opt-out を要求、なければ fail、README に記載、R1 指摘反映）。
- [x] **ADR-010 の status と進捗記述が index / 本文で矛盾（段4 未着手なのに完了表記）** — 2026-11-21 修正（index を「段5-6 完了・段4 未着手・真の cycle 解消未達」に修正、R1 指摘反映）。
- [x] **ApprovalService に agent/callId/audit pair/allowed-once が無い** — 2026-11-22 段階1修正（ApprovalRequest に with_agent/with_call_id builder と getter 追加、既存 new/request シグネチャ不変、回帰テスト、R1 指摘反映。audit pair と allowed-once は ADR-012 の第2段として残す）。
- [x] **sandbox escalation の共通 helper が無く実行ポイントへ接続されていない** — 2026-11-22 修正（pure な escalate() と EscalationDenial を追加、private rank で strictly-wider 判定、SandboxMode の Ord 変更なし、denial 文言に3桁コードなし、回帰テスト、R1 指摘反映。tool_bash への配線は次段）。
- [x] **completion notice 配信とモデル cursor 追跡が未実装** — 2026-11-22 修正（JobRecord に settle callback、RegistryState に notices/cursor、pending_notices/acknowledge 追加、job_output で回収、ADR-002 更新、回帰テスト、R1 指摘反映。モデル cursor の外部可視化は残り）。
- [x] **job owner が固定ラベル "bash" で sessionId fence が弱い** — 2026-11-21 修正（background job の owner を exec.session に変更、session 単位の fence テスト追加、ADR-002 更新、R1 指摘反映）。
- [x] **JSONL 永続化が通常 turn へ自動配線されていない** — 2026-11-22 修正（Session に observer 追加、SessionStore::observe_all 経由で mount 時に自動登録、restore は observer 非通知で二重追記回避、failures カウンタ追加、回帰テスト、R1 指摘反映）。
- [ ] **FileSystem::scoped の symlink TOCTOU 残存** — 根拠 `README.md:82`, `src/services/fs.rs:83-125` / R2-FG08 / 重大度 medium / confirmed
- [x] **session-query port が SQLite/FTS5 ではなく決定的インメモリ検索の差分が README に無い** — 2026-11-21 修正（安定性マトリクスに phase-1 行と FTS5 未実装を明記、R2-FG09 反映）。
- [x] **v1.0.0 での旧名 alias 削除が未トラック（deprecated 7個）** — 2026-11-21 修正（ADR-005 に v1.0.0 削除チェックリストと検証手順を追加、R2-FG10 反映）。
- [x] **クレート版数とリリースタグの記述が Cargo.toml / PROGRESS / ADR-001 で食い違う** — 2026-11-21 修正（PROGRESS/ADR-001 のタグ範囲を v0.261.0 に最新化、README に crate version 0.1.0 固定・タグは時系列マーカー方針を明記、R1-CF03 反映）。

- [x] **tool_todo::mount が "sessionProjections" を core と同名で提供し二重登録で衝突** — 2026-11-22 修正（`TODO_PROJECTIONS_SERVICE_NAME` = "todoProjections" に改名、provide/inject/apply 3箇所同時、core 側不変、両順序共存テスト追加、R2 指摘反映）。
- [x] **inject() が空なのに apply() で require() する port（tool_schedule / tool_exit_plan_mode）** — 2026-11-22 修正（両 port の inject() に依存名を宣言、回帰テスト追加、R2 指摘反映）。
- [x] **SessionTitles::derive_from_messages が get→set 別ロックで LLM 生成タイトルを上書き** — 2026-11-22 修正（set_if_absent で単一 write guard の原子的挿入、回帰テスト追加、R2 指摘反映）。
- [x] **CredentialProvider::set が参照名規則を検証せず解決不能な値を受理** — 2026-11-22 修正（大文字・数字・アンダースコアのみ許可、不正名は false、回帰テスト追加、R2 指摘反映）。
- [x] **PlanMode セッション別ログが無上限で蓄積し削除 API も無い** — 2026-11-22 修正（with_max_events builder で既定100件に上限、古いログを drain、回帰テスト追加、R2 指摘反映）。
- [x] **FieldSpec::range に min>max スキーマ不正チェックが無い** — 2026-11-22 修正（range()/range_int() に assert!(min<=max) 追加、should_panic テスト2件、R2 指摘反映）。
- [x] **EventLogReporter::with_max_counters(0) が黙って 1 に丸め validate と方針不一致** — 2026-11-22 修正（assert!(max_counters > 0) で fail-fast、should_panic テスト追加、R2 指摘反映）。
- [x] **DeepSeek 拡張の warn-once 集合が永続し初回以降の provider 失敗が不可視** — 2026-11-22 修正（warned_fields を registry から prepare() ローカルへ移動、毎回 failures に失敗が載るよう変更、回帰テスト追加、R1 指摘反映）。
- [x] **McpResourceRegistry::register_scoped の bool が他レジストリと逆意味、unregister が空スロープを残す** — 2026-11-22 修正（新規登録で true、空スコープを remove、回帰テスト追加、R2 指摘反映）。
- [x] **SessionQueryEngine::index がスナップショット保持で更新が反映されず unindex 手段も無い** — 2026-11-22 検証の結果修正（Session は内部 Arc 共有で実質ライブ参照。remove/refresh を追加し doc コメントを実態に修正、回帰テスト追加、R2 指摘反映）。
- [x] **production_ports.rs の本番 port カバレッジが3件に留まる（tool_fs/tool_bash/session_persistence_jsonl を追加）** — 2026-11-22 修正（3 port の mount スモークを追加、計6件の本番 port を検証、R3 指摘反映）。
- [x] **未生成の error enum バリアントが API サーフェスに残る（UserQuestionError 等）** — 2026-11-22 一部修正（user_questions の5バリアントに phase-2 予約コメント＋code() 回帰テスト。NotInPlanMode は既存テストが Ok の "PLAN_NOT_ACTIVE" を期待するため Err 化を見送り、StaleVersion は適用側 comparison 未配線で別 ADR 案件として残す）。
- [x] **README のテストファイル一覧に tests/production_ports.rs が載っていない** — 2026-11-22 修正（テストファイル一覧に本番 port mount 検証を追加、R3 指摘反映）。
- [x] **verify.sh がリポジトリ直下を cwd 前提でスクリプト参照表記も混在** — 2026-11-22 修正（先頭で repo root に cd、全スクリプト参照を `bash scripts/...` に統一、別 cwd から全ゲート通過を検証、R3 指摘反映）。
- [x] **CodexToolPayload が未使用で hook rewrite 相当が未接続（Claude と同じ ADR で扱う）** — 2026-11-22 一部修正（CodexToolPayload に Serialize を付与し tool_payload() が struct を構築、未使用シンボル解消。 rewritten input 相当の未接続は Claude/Codex 共通の phase-2 として残る）。
- [x] **check-doc-counts.sh が core service 数（38）を検証しない** — 2026-11-22 修正（services モジュール数+messageFeedback を計算し3ドキュメントで照合、gate 通過、R3 指摘反映）。

- [x] **PermissionPresets::register で追加したカスタム preset を switch できない** — 2026-11-22 修正（Preset::parse 失敗を Err ではなく preset=None として扱い spec の sandbox/approval を適用、公開型不変、回帰テスト追加、R1 指摘反映）。
- [x] **RetryPolicy::error_code が任意テキストの3桁数字を HTTP ステータスとして誤抽出** — 2026-11-22 修正（HTTP/status/TIMEOUT ラベル直後の3桁のみ抽出、"retry in 500ms" 等は拒否、回帰テスト追加、R1 指摘反映）。
- [x] **RetryPolicy::jitter_ratio に検証がなく不正値で busy retry になる** — 2026-11-22 修正（ratio を [0,1] にクランプ＋NaN 防御、jitter に 0.1 下限、回帰テスト追加、R3 指摘反映）。
- [x] **EventLogReporter::new が EventLogOptions を検証せず不正設定を保持** — 2026-11-22 修正（new() で validate() を assert、with_max_counters と方針統一、should_panic テスト追加、R3 指摘反映）。
- [x] **check-secrets.sh がローカル導入法として存在しない cargo install gitleaks を案内** — 2026-11-22 修正（案内を pre-commit install と GitHub Releases／taiki-e に差し替え、R3 指摘反映）。
- [x] **verify.sh と CI extra_gates で markdownlint の実行順が乖離** — 2026-11-22 修正（markdownlint を secret scan 直後に移動し CI の extra_gates と同順に、別 cwd から全ゲート通過を検証、R3 指摘反映）。
- [x] **Duration::from_millis の doc がクランプと誤記（実装は格納のみ）** — 2026-11-22 修正（コメントを「検証なしの格納」に修正、範囲チェックは try_from_millis の責務と明記、R3 指摘反映）。
- [x] **WorkspaceRegistry::sessions_in が未存在 workspace を空 Vec に丸める** — 2026-11-22 修正（tool_session_list 側で事前に get(id) 存在確認し NotFound を InvalidArgs として拒否、公開 API 不変、R3 指摘反映）。
- [x] **6 件の ADR に古いテスト件数が残り doc-count ゲート対象外** — 2026-11-22 修正（6 ADR の件数に「受諾時点」と明記、現行値は README/PROGRESS を参照と注記、履歴文書として保持、R4 指摘反映）。
- [x] **PROGRESS.md と README の検証ゲート記述が verify.sh の実順と不一致** — 2026-11-22 修正（PROGRESS / README / runbook の3記述を verify.sh 実順に統一、R4 指摘反映）。
- [x] **runbook が mount_real_composition / require の旧挙動を案内** — 2026-11-22 修正（diagnostic レポートと require_untyped_with_timeout を反映、R4-DC11 反映）。
- [x] **公開ドキュメントにホスト固有絶対パスと非公開レポート参照が残存** — 2026-11-22 修正（runbook の cd を汎用記述に、README の非公開レポート参照を削除、R4-DC12 反映）。
- [x] **examples が固定 /tmp パスを使用し smoke 並列実行で衝突しうる** — 2026-11-22 修正（3 example の固定 /tmp パスを PID+nanos 一意パスに変更、7 examples smoke 通過、R4 指摘反映）。
- [x] **Shell の stdout/stderr 読み取りエラーが握り潰され不完全出力が成功として返る** — 2026-11-22 修正（read_to_end の Err を ShellError::Spawn として伝播。public enum 変更を避け既存バリアントを流用。エラー再現は OS パイプ破壊が必要で回帰テストは見送り、正常系テストで不変確認）。

- [x] **LlmService の既定 provider を dispose 後も default 名が残り resolve(None) が失敗** — 2026-11-22 修正（disposer が既定 provider 削除時に default 名をクリア、回帰テスト追加、R5 指摘反映）。
- [x] **TimeoutPolicyPlugin の inject("tools") と apply の require("toolPipeline") が不一致** — 2026-11-22 修正（inject を "toolPipeline" に変更、R2-ST02 と同種、回帰テスト追加、R5 指摘反映）。
- [x] **CommandRegistry が同名コマンドを先着優先で保持し ToolRegistry と世代ルールが異なる** — 2026-11-22 修正（HashMap + 世代 id の最後登録優先に変更、古い disposer が新定義を消さない fencing、回帰テスト追加、R5 指摘反映）。
- [x] **FieldSpec の単発 min/max ビルダーに順序・NaN 検証がなく range() と方針が割れる** — 2026-11-22 修正（min()/max() に NaN assert 追加、range() と方針統一、should_panic テスト2件、R5 指摘反映）。
- [x] **ValidationReason::OutOfRange が i64 境界を f64 に丸め 2^53 超で診断値が誤る** — 2026-11-22 修正（IntegerOutOfRange バリアントを追加し i64 を exact 保持、Display と validate を更新、既存 f64 range は OutOfRange 維持、R5 指摘反映）。
- [x] **PluginRegistry::register が同一 id を黙って上書きし MountReport に現れない** — 2026-11-22 修正（try_register と LoaderError::DuplicatePlugin を追加、既存 register は後方互換 adapter として維持、回帰テスト追加、R5 指摘反映）。
- [x] **FsError の Display に安定コードがなくログから種別を判別できない** — 2026-11-22 修正（FsErrorCode に Display、FsError::Display にコード接頭辞、回帰テスト追加、R5 指摘反映）。
- [x] **unsafe 禁止ゲートの行頭 unsafe { 取りこぼしとセルフテスト欠如** — 2026-11-22 修正（regex を (^|空白)unsafe{ に修正、行頭・インデント・extern fn の3形セルフテスト追加、検出を実証、R5 指摘反映）。
- [x] **ADR-001 だけ OKF frontmatter がなくメタデータ形式が他と不一致** — 2026-11-22 修正（type/title/description/tags/generated/verified/status の YAML frontmatter を追加、箇条書きメタデータ削除、R5 指摘反映）。
- [x] **CommandDefinition::new がコマンド名を検証せず parse_command と規約が切れる** — 2026-11-22 修正（先頭 `/` と `[a-z][a-z0-9_-]*` を assert、should_panic／正規名テスト追加、R5 指摘反映）。
- [x] **PROGRESS.md の残タスク件数が todo.md の実未着手数と乖離** — 2026-11-22 修正（固定数をやめ todo.md を正とする記述に変更、スナップショット値 20 件を明記、R6 指摘反映）。
- [x] **tool_goal の max_goal_rounds が u64→u32 無検証キャストで丸められる** — 2026-11-22 修正（u32::try_from で範囲外を InvalidArgs 拒否、回帰テスト追加、R6 指摘反映）。
- [x] **hooks_claude_code の parse_config が数値設定を無検証で受け入れる** — 2026-11-22 修正（0 と usize 超过値を Err として拒否、回帰テスト追加、R6 指摘反映）。
- [x] **session_turn_outline の turn 比較が u64→i64 キャストで順序を誤る** — 2026-11-22 修正（last_turn を `Option<u64>` にして u64 同士で比較、i64::MAX 超回帰テスト追加、R6 指摘反映）。

- [x] **port の effect 解除が disposer エラーを握り潰し ADR-007/004 に違反** — 2026-11-22 修正（実測8箇所の on_dispose を `disposer.dispose().await` の直接伝播に統一、握り潰し箇所ゼロを確認、R7 指摘反映）。
- [x] **config::validate が未宣言の config キーをサイレントに破棄し警告経路もない** — 2026-11-22 修正（validate_with_warnings 追加で既存 validate() 不変、MountReport.warnings に config: 前缀で集約、to_plugin は adapter 維持、回帰テスト2件、R1 指摘反映。runbook への警告の読み方追記は次案件）。
- [x] **FieldSpec::with_default が既定値の型・範囲を検証せず不正スキーマを通す** — 2026-11-22 修正（既定値挿入経路で validate_value を通し型違反を Err 化、回帰テスト追加、R7 指摘反映）。
- [x] **FieldSpec の単発境界 setter に前後関係の検証がなく到達不能レンジを受理** — 2026-11-22 修正（min/max/min_int/max_int が反対側 bound と比較して fail-fast、should_panic 2件追加、R7 指摘反映）。
- [x] **check-doc-counts.sh が README のテストファイル一覧を検証していない** — 2026-11-22 修正（tests/*.rs と README 列挙の双方向照合 gate を追加、片方省略時の fail を実証、R7 指摘反映）。
- [x] **doc-count ゲートが実際の cargo test 件数と突き合わせていない** — 2026-11-22 修正（check-doc-counts.sh に cargo test --all-targets -- --list の実登録件数照合を追加、静的283件と一致を確認、R7 指摘反映）。
- [x] **todo.md と ADR-010 / docs/index.md で段4（dispatch API registry 化）の完了状態が矛盾** — 2026-11-22 修正（Runtime Proof で *_with_registry 実装を確認、ADR-010 と index.md を段4完了に更新、re-export 経由の型依存残存は明記、R7 指摘反映）。
- [x] **「6 エントリすべてを mount」という記述が部分 mount 実装と乖離** — 2026-11-22 修正（PROGRESS / ADR-001 を「port 存在分のみ mount＋failed/skipped レポート」に修正、example ヘッダーは既に正確、R7 指摘反映）。
- [x] **production_ports.rs だけ TempDirGuard 規約を回避** — 2026-11-22 修正（Drop で削除する TempDirGuard を導入、手組み一時パスと手動 remove_dir_all を削除、他3テストファイルと規約統一、R7 指摘反映）。
- [x] **timeout 経路の child.kill/wait エラーが破棄され後始末失敗が観測不能** — 2026-11-22 修正（TimedOutCleanupFailed バリアント追加、kill/wait 失敗を構造的に保持、既存 TimedOut と Display は不変、R1 指摘反映）。
- [x] **Cargo.toml の tokio features 説明コメントが actual feature 一覧と乖離** — 2026-11-22 修正（library 側 process/io-util を含む実 feature を反映、同期要旨を明記、R7 指摘反映）。
- [x] **PROGRESS.md が status: current のまま 100 ラウンド・20 件の旧スナップショット** — 2026-11-22 修正（159 ラウンド・14 件・日付明記に更新、未着手数は todo.md を正とする記述に統一、R7 指摘反映）。
- [x] **runbook に中国語混在の誤記（列举）** — 2026-11-22 修正（「列举」を「列挙」に修正、R7 指摘反映）。

- [x] **with_llm_retry が Done を合成し枯渇ストリームを成功として返す** — 2026-11-22 修正（Done/Error 無しの枯渇時に Error を返し fail-closed、chain(Done) は LlmService::stream が Error で先に return するため維持、flaky テストに Done 追加＋回帰テスト、R1 指摘反映）。
- [x] **PermissionPresets::register が組み込み preset 名を黙って上書きできる** — 2026-11-22 修正（try_register を追加して組み込み名を拒否、register は legacy last-write-wins を維持、回帰テスト追加、R8 指摘反映）。
- [x] **RetryPolicy の遅延フィールドに下限がなく 0 で busy retry になる** — 2026-11-22 修正（backoff_ms の返り値に 1ms 下限、max_delay_ms=0 の回帰アサーション追加、R8 指摘反映）。
- [x] **公開ヘルパが UTF-8 境界を無視したバイトスライスで panic しうる** — 2026-11-22 修正（has_utc_offset は get() で境界防御、active_at_token は char_indices で column→byte 変換、マルチバイト回帰テスト追加、R8 指摘反映）。
- [x] **状態を持つ TimeContext / TmuxContext が Clone で状態共有されない** — 2026-11-22 修正（last を Arc<RwLock<Option<..>>> 化して clone 間で状態共有、mark_injected を &self に、sample_message も &TimeContext に変更、回帰テスト追加、R1 指摘反映）。
- [x] **StorageHub::write が Object 以外で必須フィールド検証をスキップ** — 2026-11-22 修正（必須フィールド持つドメインは Object 以外を InvalidRecord 拒否、必須なしドメインの scalar は従来どおり許容、回帰アサーション追加、R8 指摘反映）。
- [x] **SkillRegistry::catalog が read guard 保持中に provider コールバックを実行** — 2026-11-22 修正（guard 内で providers を clone して drop 後にコールバック実行、reentrant デッドロック回帰テスト追加、R8 指摘反映）。
- [x] **hooks.json の未知イベント名・不正エントリが警告なしでスキップ** — 2026-11-22 修正（parse_config_with_warnings 追加で4種のスキップ理由を警告、parse_config は adapter 維持、phase-1 未対応は情報トーン、回帰テスト追加、R1 指摘反映）。
- [x] **GoalService::clear が commit 失敗を握り潰す** — 2026-11-22 修正（try_clear を追加して commit 結果を伝播、command_goal で失敗を CommandResult::error 化、clear() は adapter 維持、R8 指摘反映）。
- [x] **MessageFeedback の version が辞書順比較で数値版を誤判定** — 2026-11-22 修正（両者が数値なら u64 比較、他形式は従来の辞書順を維持、9→10 回帰テスト追加、R8 指摘反映）。
- [x] **validate_with_warnings がネスト Object/Array の未宣言キーを検出しない** — 2026-11-22 修正（validate_inner で warnings を再帰伝播、ネスト Object/Array の未宣言キーも検出、回帰テスト追加、R8 指摘反映）。
- [x] **WorkspaceRegistry::sessions_in の doc に存在しない cwd 引数の記述** — 2026-11-22 修正（コメントを実際のシグネチャに合わせ「Sessions attached to the workspace.」に修正、R8 指摘反映）。
- [x] **CommandRegistry::names が HashMap 順で非決定的** — 2026-11-22 修正（names() の結果を sort して人間向け一覧を決定的に、R8 指摘反映）。
- [x] **ADR-004 の fail 動作表が現行実装と2点乖離** — 2026-11-22 修正（approval 既定 fail-open と post-execute 失敗時実行を現行実装に合わせ明記、R8 指摘反映）。
- [x] **README の「根拠」path:line アンカーが陳腐化** — 2026-11-22 修正（session_persistence_jsonl.rs:348、hooks_claude_code.rs:152,159 にアンカー更新、R8 指摘反映）。
- [x] **ADR-008 の「将来利用」棚卸しと alias 削除方針が現状と矛盾** — 2026-11-22 修正（sandbox_mode_fold / apply_goal_event を「利用あり」に更新、alias 削除方針の優先文書を明記、R8 指摘反映）。
- [x] **README が非推奨 alias emit_detached を推奨** — 2026-11-22 修正（推奨 API を emit_spawn に変更、R8 指摘反映）。
- [x] **doc-count ゲートが port/service の名前一覧を照合しない** — 2026-11-22 修正（ports/mod.rs と README port 一覧の双方向照合 gate を追加、漏れていた session-title-llm を追記、fail 動作を実証。service は module 名と service 名が 1:1 でないため既存件数 gate に委ね、R8 指摘反映）。

- [x] **with_llm_timeout が total request deadline を持たない** — 2026-11-22 修正（with_llm_total_timeout を timeout_at で追加、既存 decorator 不変、定期チャンクでも発火する回帰テスト、R1 指摘反映）。
- [x] **OTLP EventRecord に per-event 時刻が無く全イベント同一 timestamp** — 2026-11-22 修正（EventRecord 不変のまま ReporterState が (record, ms) を保持、events_with_timestamps と otlp_payload_with_timestamps を追加、otlp_payload は adapter 維持、回帰テスト、R1 指摘反映）。
- [x] **approval audit pair（approval/asked + approval/decided）が未記録** — 2026-11-23 修正（provide_with_session / with_session を追加し Session に asked→decided を記録、asked_seq で対応付け、details は制御文字除去＋長さ制限、既存 provide/request 不変、回帰テスト、R1 指摘反映）。
- [x] **tool_todo の StaleVersion が適用側 comparison 未配線** — 2026-11-23 修正（write_with_version と todo_version を Tools に追加、既存 invoke/TodoWriteArgs 不変、stale は ToolFailure::Args で拒否、回帰テスト、R1 指摘反映）。
- [x] **同期 remove_service が generation を検証せず最新登録を消し得る** — 2026-11-23 修正（remove_service_generation_sync を追加、既存 remove_service は明示的全削除として不変＋doc 明記、stale disposer 回帰テスト、R1 指摘反映）。
- [x] **Context::provide がサービス名を検証せず空文字を受理** — 2026-11-22 修正（空／空白のみのサービス名を Err で拒否、回帰テスト追加、R9 指摘反映）。
- [x] **EventRegistry::remove が存在しない key で空スロットを生成** — 2026-11-22 修正（remove を try_slot ベースに変更し未登録 listener の削除でスロットを作らない、internal 回帰テスト追加、R9 指摘反映）。
- [x] **typed_event! の decode error store がプロセスグローバル** — 2026-11-22 修正（EventRegistry に decode_errors 追加、マクロは registry のみに記録、静的 take_decode_errors は後方互換 shim として残す、Context 分離回帰テスト追加、R1 指摘反映）。
- [x] **is_bail_value が bool のみ対応（一般 R 型で false 相当を判定できない）** — 2026-11-23 修正（R = bool のみ false を非 bail と認識する制限を doc 明記。false 相当 newtype の一般化は設計判断として残す、R9 指摘反映）。
- [x] **Context::set_lifecycle が Fiber 状態と無関係に公開され二重状態** — 2026-11-23 段階1修正（plugin.rs の二重更新を set_both ヘルパーに集約、set_lifecycle の pub API は維持し doc で Fiber::refresh/unload 推奨を明記、R1 指摘反映）。段階2（ctx.lifecycle への単一情報源化）は ADR 化して別案件。2026-11-19 却下の「単調性検証」とは観点が異なる。
- [x] **Fiber::mount が inject 未充足でも Pending で返す契約を doc 明記** — 2026-11-22 修正（await が load 完了を待たず Pending で返る旨を doc に明記、state()/Active 待ちを案内、R9 指摘反映）。
- [x] **README/ADR-001 が Some(false) を upstream と異なると誤記** — 2026-11-22 修正（Some(false) は非 bail で upstream 同一と明記、R9 指摘反映）。
- [x] **README に emit の await 版／emit_spawn の使い分け表が無い** — 2026-11-22 修正（emit は await、upstream 準拠の fire-and-forget は emit_spawn と明記、R9 指摘反映）。
- [x] **hook 監査イベント文字列が Claude/Codex で重複** — R6 Runtime Proof で誤検知と判定（監査定数は hooks_claude_code の1箇所のみ、hooks_codex は既にその audit_pair を再利用。重複なし）。
- [ ] **hooks updatedInput rewrite が未接続（select_tool_hooks 未使用）** — 根拠 `src/ports/hooks_codex.rs:81`, `todo.md:115,162` / R9-ST04 / 重大度 low / medium
- [ ] **DeepSeek 実 HTTP が FakeTransport 依存で end-to-end 無い** — 根拠 `src/ports/llm_transport.rs:122`, `README.md:73` / R9-ST09 / 重大度 low / confirmed
- [x] **同期 remove_service の generation 迂回（drain 非同期経路のみ修正済み）** — 2026-11-23 R9-DI01 と同時修正（generation sync API 追加）。

- [x] **SessionStore::create が自動 ID 生成で next_id を二重インクリメント** — 2026-11-23 修正（auto ID 分岐のインクリメントを1回のみに、明示 id 指定時は従来どおり cursor 前進、連番の回帰テスト追加、R10 指摘反映）。
- [x] **ScheduleService が Cancelled レコードを resume で復活させる** — 2026-11-23 修正（set_status に Cancelled 終端ガード、cancel→cancel は冪等として維持、ScheduleError::Cancelled が生成されるように、回帰テスト、R1 指摘反映）。
- [x] **Subagent/Agent registry が世代 fencing 無しで古い disposer が新登録を消す** — 2026-11-23 修正（tools.rs の next_id + (id, value) パターンを Subagent/Agent factory registry へ移植、stale disposer は no-op、内部 get を .map(|(_, h)| h.clone()) に更新、回帰テスト、R1 指摘反映）。
- [x] **StorageHub::define_domain が空名・空 required_fields・重複を許す** — 2026-11-23 修正（try_define_domain / try_register_backend を追加して検証付き API を提供、StorageError に InvalidDomain / InvalidBackend 追加、既存 API は無条件上書きの旧挙動を維持、回帰テスト、R1 指摘反映）。
- [x] **StorageHub::write が domain/backend 未設定でエラー優先順が不安定** — 2026-11-23 修正（domain → backend → record の順に解決、backend 解決を1箇所に集約、設定エラーが入力エラーより優先、回帰テスト、R1 指摘反映）。
- [x] **PlanMode::set が空 session_id を受理** — 2026-11-23 修正（try_set/try_enter/try_exit と PlanModeError を追加して fail-closed に、set は try_set の expect adapter、seq を消費せず拒否、回帰テスト、R1 指摘反映）。
- [x] **CredentialProvider が数字始まりの参照名を受理** — 2026-11-23 修正（共通ヘルパー is_valid_env_name で先頭を大文字または `_` に制限、credential_ref と set の両方に適用し重複ロジックも解消、回帰テスト、R1 指摘反映）。
- [x] **WorkspaceRegistry::create が archive 済み id の再利用を拒否** — 2026-11-23 修正（archived レコードのみ create で上書き再作成、order.push をスキップして list 重複を回避、session_ids は空・attach は新レコードに成功、生きている id は Duplicate 維持、回帰テスト、R1 指摘反映）。
- [x] **Session::append_with_seq が seq の飛びを許容** — 2026-11-23 修正（restore_with(strict) を追加し明示指定時のみ seq gap を Corrupt として検出、既定の restore は飛び許容を維持、回帰テスト、R1 指摘反映）。
- [x] **ドメイン Error enum の多くが std::error::Error を実装しない** — 2026-11-23 修正（Storage/Goal/Agent/Subagent/Workspace/Schedule の6 enum に impl std::error::Error を追加、回帰テスト、R10 指摘反映）。
- [x] **verify.sh の cargo audit が extra_gates より前で「同順」記述と矛盾** — 2026-11-23 修正（audit を extra_gates 相当の全ゲート実行後に移動、R10 指摘反映）。
- [x] **README.md:118 の clippy 例が CI の追加フラグを欠く** — 2026-11-23 修正（verify.sh と同じ --locked / -A type_complexity / -D undocumented_unsafe_blocks を記載、R10 指摘反映）。
- [x] **check-doc-counts.sh が examples/ のファイル一覧を検証しない** — 2026-11-23 修正（examples/*.rs と README の cargo run --example 一覧の双方向チェックを追加、故意の drift でゲート失敗することを確認、R10 指摘反映）。
- [x] **check-markdown.sh のグロブが target/ と .git/ を含む可能性** — 2026-11-23 修正（markdownlint-cli2 に !**/target/** と !**/.git/** の除外 glob を追加、PROGRESS の件数固定表記を動的表記に変更、R10 指摘反映）。
- [x] **lib.rs の deprecated alias 6個に since が欠ける** — 2026-11-23 修正（ADR-005 の方針「since は意図的に省略」に合わせ、唯一 since が付いていた1件を外して7件すべて統一、R10 指摘反映）。
- [x] **ToolPipeline approval 既定 fail-open の根拠が ADR-004/012 で二重管理** — 2026-11-23 修正（ADR-004 を正本とし、ADR-012 と README を参照化。既存の振る舞いテスト tool_pipeline_require_approval_is_opt_in_fail_closed が fail-open を固定済みのため getter 追加は見送り、R1 指摘反映）。
- [x] **README の LLM API 一覧に transport seam が欠ける** — 2026-11-23 修正（handler_from_transport / LlmTransport / with_llm_total_timeout を API 一覧に追加、R10 指摘反映）。

- [x] **GoalRoundDriver::drive が write guard 保持中に session.append を呼ぶ** — 2026-11-23 修正（判定・competing リセット・round 採番・状態更新は guard 内、durable append のみ guard 外の2フェーズ化、observer から note_competing 再入デッドロック解消、タイムアウト付き回帰テスト、R1 指摘反映）。
- [x] **MessageFeedback::put が read guard 保持中に session.append を呼ぶ** — 2026-11-23 修正（put / put_events とも Session を clone して state guard を drop してから操作、observer からの service 再入デッドロック解消、タイムアウト付き回帰テスト、R11 指摘反映）。
- [x] **SessionStore::observe_all が write guard 保持中に session.observe を呼ぶ** — 2026-11-23 修正（store write guard 内で sessions を clone して guard を drop してから各 session.observe を呼ぶ、observer からの store 再入デッドロック解消、タイムアウト付き回帰テスト、R11 指摘反映）。
- [x] **write_with_version が check と increment を別ロックに分け並行書込みが両方通る** — 2026-11-23 修正（ToolsState に todo_in_flight を追加、check→reserve を単一 std Mutex lock 内で原子的に実行し await 中は guard を保持しない、並行呼び出しで1件のみ成功、回帰テスト、R1 指摘反映）。
- [x] **JobRegistry が完了ジョブを削除せず notices も縮小しない** — 2026-11-23 段階1修正（acknowledge で通知を drain し cursor を 0 リセット、未読集合は単調減少、回帰テスト、R1 指摘反映）。完了ジョブの即削除は get/list 契約（完了後も view/read で出力回収）を壊すため見送り、保持方針は別 ADR 案件。
- [x] **JobHandle::settle と start のロック順序が未明示（AB-BA 窓）** — 2026-11-23 修正（settle のロック順序契約を doc 明記：record guard を terminal lock と on_settle コールバックより前に drop、R11 指摘反映）。
- [x] **FakeTransport::send が requests guard 保持中に responses lock を取得** — 2026-11-23 修正（requests push の guard をブロックスコープで明示的に drop してから responses lock を取得、R11 指摘反映）。
- [x] **JsonlPersistence::open が exists チェックと insert を別ロックで行う** — 2026-11-23 修正（handle 検索・path 存在確認・insert を単一 write guard 内で原子的に実行、並行 open での重複 handle 生成を防止、R11 指摘反映）。
- [x] **ExecutorSelector が unload 時に古い executor を clear しない** — 2026-11-23 修正（register_with_id と clear_generation を追加し所有付きクリア、mount の disposer で fiber.unload 後に自生成分のみクリア、共有 holder の他人を 보호、回帰テスト、R1 指摘反映）。
- [x] **DeepSeekConfig::validated_endpoint が plugin_ports に re-export 漏れ** — R6 Runtime Proof で誤検知と判定（impl DeepSeekConfig のメソッドは型 re-export で呼べる、tests も既に plugin_ports::DeepSeekConfig 経由で成功）。
- [x] **compaction_automation の compaction_count / note_success が未使用公開 API** — 2026-11-23 修正（完全に未使用の2メソッドと doc コメントを削除、R11 指摘反映）。
- [x] **feedback_producer の shared_event が未使用公開 helper** — 2026-11-23 修正（未使用の shared_event と未使用 Arc import を削除、R11 指摘反映）。
- [x] **command_feedback::execute_command が internal-only の公開 API** — 2026-11-23 修正（モジュール内 handler からのみ呼ばれるため pub を外し private 化、R11 指摘反映）。
- [x] **native_tool_executor::with_max_parallel が未使用 builder** — R6 Runtime Proof で誤検知と判定（mount :213 で executor 生成時に実際に使用されている）。
- [x] **sandbox_policy::WRITE_TOOLS が内部使用のみで公開面に残る** — 2026-11-23 修正（is_write_tool 実装のみで使用され re-export もされていない定数を private 化、R11 指摘反映）。
- [x] **tool_str_replace_editor の CLIP_MARKER / DEFAULT_MAX_OUTPUT_CHARS がデッド定数** — R6 Runtime Proof で誤検知と判定（clip 処理 :24-28 で実際に参照されている）。
- [x] **session_log_deepseek::DEFAULT_MAX_BYTES が re-export 漏れ** — 2026-11-23 修正（plugin_ports に re-export、回帰テスト追加、R11 指摘反映）。
- [x] **FsErrorCode / AttachmentErrorCode の as_str が re-export 漏れ** — R6 Runtime Proof で誤検知と判定（impl のメソッドは型 re-export で呼べる、個別 re-export は不要）。
- [x] **otel_exporter の batches / failures が re-export 漏れ** — R6 Runtime Proof で誤検知と判定（OTelExporter のメソッドは型 re-export で呼べる）。

- [x] **tool_str_replace_editor::parse_range が符号付き end と 32bit 丸めを無検証** — 2026-11-23 修正（start / insert_line を usize::try_from、end を end_raw < -1 明示拒否＋try_from、-1 は末尾の既存契約維持、回帰テスト、R1 指摘反映）。
- [x] **session_stats / tmux_context / time_context の u64 加算が overflow し得る** — 2026-11-23 修正（3箇所の加算を saturating_add に変更、u64::MAX で頭打ちし corrupt state で 0 に巻き戻らない契約を module doc 明記、回帰テスト、R1 指摘反映）。
- [x] **session_turn_outline::outline の limit*2 が unchecked** — 2026-11-23 修正（preview の3箇所を saturating_mul(2) に、take を saturating_sub(1) に統一し巨大 limit での overflow を防止、R12 指摘反映）。
- [x] **attachment_admission の max_bytes 境界が 1 文字広い（最大 N+2 バイト許容）** — 2026-11-23 修正（encoded 上限を厳密 ceil(N/3)*4 に、decode 後も bytes.len() > max_bytes をチェック、DoS 事前フィルタ維持、回帰テスト、R1 指摘反映）。
- [x] **compaction_image_offload::parse_targets が u64→usize 丸めで巨大 index を受理** — 2026-11-23 修正（image index を usize::try_from で検証し範囲外を拒否、32bit での暗黙丸めを防止、R12 指摘反映）。
- [ ] **ToolError::Execution の文字列エラーと構造化 enum が混在** — 根拠 `src/services/tools.rs`, `src/ports/tool_str_replace_editor.rs:98-141` / R12-IV06 / 重大度 medium / medium
- [x] **tool_session_list::resolve_page が offset 上限を検証せず 32bit で丸める** — 2026-11-23 修正（limit / offset とも usize::try_from で Range 検証、32bit での暗黙丸めを拒否、R12 指摘反映）。
- [x] **session_stats が assistant attempt+message で ttftEvents を二重計上** — 2026-11-23 修正（message 側で attempt 由来の firstTokenSeq がある場合は ttftEvents を increment せず ttftSteps のみ、message 単独の既存挙動は維持、回帰テスト、R1 指摘反映）。
- [x] **Duration::from_millis が safe-integer 域を迂回して decompose/render を通す** — 2026-11-23 修正（decompose が MAX_SAFE_MILLIS を再検査しない設計意図を doc 明記、try_from_millis が opt-in 境界、両方の境界回帰テスト、R12 指摘反映）。
- [x] **PruneError::InvalidConfig の文言 'below' が等価許容と矛盾** — 2026-11-23 修正（文言を "must not exceed the threshold" に変更、head+tail+marker==threshold が Ok／+1 で Err の境界テスト追加、R12 指摘反映）。
- [x] **JobRegistry::wait の timeout=false 経路がテスト未固定** — 2026-11-23 修正（未 settle のジョブに timeout_ms=1 で wait して false、settle 後は true を固定する回帰テスト追加、R12 指摘反映）。
- [x] **ToolExecContext の signal_cancelled と cancel_flag の優先順位が doc 未明記** — 2026-11-23 修正（両方が dispatch 前 is_cancelled で OR 判定され、cancel_flag は body 後にも再分類されることを doc 明記、R12 指摘反映）。
- [x] **EventLogReporter::record の batch boundary が集約カウンタ依存であることを doc 未明記** — 2026-11-23 修正（境界が cardinality 集約後の per-name counter に従う旨を doc 明記、R12 指摘反映）。
- [x] **AgentLoop の module doc が max_steps 到達時の未処理 tool call を説明しない** — 2026-11-23 修正（R6 Runtime Proof で「未処理 call は無く各 step の tool call は実行済み」と確認し、その旨を doc 明記、R12 指摘反映）。
- [x] **SessionQueryEngine::remove が index のみで SessionStore に触れないことを doc 未明記** — 2026-11-23 修正（index のみに影響し SessionStore は不変であることを doc 明記、R12 指摘反映）。
- [x] **HookProtocol の matcher 不正時のログ有無が doc／テスト未固定** — 2026-11-23 修正（CompiledMatcher::Never のコンパイル失敗はサイレントで config load 時の matcher_diagnostic が診断を担う旨を doc 明記、既存 non-match テストで固定済み、R12 指摘反映）。

- [x] **ToolResultPruner / CompactionEngine の設定加算が unchecked で巨大設定で panic** — 2026-11-23 修正（pruner は saturating + 明示 overflow 拒否で InvalidConfig、compaction は閾値オーバーフローを明示 Err、既定値は不変、回帰テスト、R1 指摘反映）。
- [x] **LlmService::register の disposer が名前ベースで default 所有権が弱い** — 2026-11-23 修正（default を (provider_id, name) に変更、disposer は id 比較で自分が所有する default のみクリア、set_default(&str) は名前→id adapter として不変、回帰テスト、R1 指摘反映）。
- [x] **AgentLoop::with_session の Session 共有（Arc）が doc 未明記** — 2026-11-23 修正（Session が Arc ハンドルで clone がログを共有し独立コピーでない旨を builder doc 明記、R13 指摘反映）。
- [ ] **ToolOutput の deep clone が observer 経路で繰り返される** — 根拠 `src/services/tools.rs:83-110,423`, `src/services/agent_loop.rs:312` / R13-OW04 / 重大度 low / medium
- [ ] **Event の Arc 值与値保持がサービスごとに一貫しない** — 根拠 `src/events.rs:355-368`, `src/ports/user_questions.rs:141-145` / R13-OW05 / 重大度 low / low
- [x] **CredentialProvider::with_env が既存 clone と env を共有しない** — 2026-11-23 修正（env を Arc の RwLock に変更し with_env が共有状態を更新、resolve は read で参照、シグネチャ不変、clone 間共有の回帰テスト、R1 指摘反映）。
- [x] **Fake* テスト用型が本番公開 API に露出し誤配線リスク** — 2026-11-23 修正（FakeTransport / FakeCollector の型 doc に TEST ONLY と本番配線先を明記、feature ゲートは公開 API 破壊のため見送り、R13 指摘反映）。
- [x] **ProjectionFold の stateful fold 契約（Arc の Mutex capture）が doc 未明記** — 2026-11-23 修正（Fn 束縛では状態変更できないため Arc の Mutex/RwLock で捕捉する旨を型 doc 明記、R13 指摘反映）。
- [x] **AgentLoop::with_call_parser / with_checkpoint の古い hook cleanup 未呼び出しが doc 未明記** — 2026-11-23 修正（旧 hook は単に drop され cleanup は呼ばれない旨を各 builder の doc 明記、R13 指摘反映）。
- [x] **FsObservationPolicy の clone が二段マップ全体を共有する契約が doc 未明記** — 2026-11-23 修正（clone が owner→path マップ全体を共有し独立コピーでない旨を型 doc 明記、R13 指摘反映）。
- [x] **ToolRegistry::register の generation-guard 回帰テストが未固定** — 2026-11-23 修正（同名2登録→旧 disposer が新定義を消さない、二重 dispose が no-op を固定する回帰テスト追加、R13 指摘反映）。
- [x] **DeepSeekLlmApiExtensions::register の bool 拒否分岐が未テスト** — 2026-11-23 修正（空／前後空白／重複で false、正常で true、unregister が true→false を固定する回帰テスト追加、R13 指摘反映）。
- [x] **SessionTitleFirstPromptLlm::generate の空 messages → Ok(false) が未テスト** — 2026-11-23 修正（有効 config で空 messages を渡し Ok(false) を返す early return を固定する回帰テスト追加、R13 指摘反映）。
- [x] **ScheduleService::create が空 title/prompt を受理（検証なし）** — 2026-11-23 修正（create が Result でないため enum 変更を避け、空文字をそのまま受理する契約を doc 明記＋回帰テスト固定、R13 指摘反映）。
- [x] **ToolRegistry::names のソート契約が未テスト** — 根拠 `src/services/tools.rs:212-218` / R13-ST05 / 重大度 low / confirmed → 上記 R13-ST01/07 の回帰テストに統合して固定済み。
- [x] **PluginInventory::list の宣言順保持が未テスト** — R6 Runtime Proof で誤検知と判定（既存 plugin_inventory_projects_composition_entries が宣言順 a,b と enabled override の両方を検証済み）。
- [x] **generation-guard disposer の二重 dispose 冪等性が未テスト** — 2026-11-23 修正（ToolRegistry 側を R13-ST01 の回帰テストで固定。commands 側は同種実装で既存テストあり）。

- [x] **Fiber refresh の dispose_effects 失敗で retiring=true / Unloading が残る** — 2026-11-23 修正（dispose 失敗時に retiring をクリアし Failed へ戻して Error::Plugin を伝播、正常系の Pending 遷移は不変、R1 指摘反映）。
- [x] **Fiber::mount 失敗時に登録済み observer が解除されず蓄積** — R6 Runtime Proof で誤検知と判定（register_fiber は Arc::downgrade した Weak のみ登録、mount 失敗時は strong Arc が drop され strong_count()==0 となり dependent_fibers の retain で次回通知時に prune、蓄積なし）。修正不要。
- [x] **FileSystem::write_text が create_dir_all 後の write 失敗で親ディレクトリを残す** — 2026-11-23 修正（既存の tmp+rename パターンを再利用し部分ファイルを防止、失敗時に新規作成した親を remove_dir で rollback（失敗は無視、remove_dir_all 不使用）、回帰テスト、R1 指摘反映）。
- [x] **FileSystem::edit_text の version guard が read→write 間で TOCTOU** — 2026-11-23 修正（version を単一 write guard で予約（check＋increment）し I/O 直前に実行、失敗時は旧値に rollback、write_text も同じ予約経路に統一、並行 edit が片方だけ成功する回帰テスト、R1 指摘反映）。
- [x] **JSONL append が flush 失敗で部分永続化し再試行で seq 重複** — 2026-11-23 修正（JsonlSessionHandle に clone 間共有の poison フラグを追加、open/write/flush 失敗で poison を立て再試行を拒否し重複 seq 行の生成を防止、回帰テスト、R1 指摘反映）。
- [x] **mcp_resources::mount が provide 成功後 effect 失敗で registry サービスを残す** — 2026-11-23 修正（provide が返す Disposer を effect scope の on_dispose に登録し、effect 失敗時の既存 child unwind に service も含める、正常系の登録と戻り値は不変、R1 指摘反映）。
- [x] **command_goal::mount が goals provide 後に commands 失敗で部分登録** — 2026-11-23 修正（provide_goals / provide_commands に Disposer を返す新 API を追加し既存は adapter として維持、mount が後続失敗時に新規作成した service のみ明示 rollback、既存 goals は所有権尊重で対象外、R1 指摘反映）。
- [x] **tool_bash::mount が require_approval 後の失敗で承認要求を残す** — 2026-11-23 修正（require_approval を全 fallible ステップ成功後の最後に移動、failed mount で pipeline だけ fail-closed になる副作用リークを解消、正常系と fail-closed 契約は不変、R1 指摘反映）。
- [x] **GoalStore::commit に rollback 境界が無い（将来の不整合リスク）** — 2026-11-23 修正（GoalState に Clone を追加し commit を clone→apply→swap に変更、apply が失敗したら元状態を保持、検証エラーで event が増えない回帰テスト、R14 指摘反映）。
- [x] **RepeatToolReminder::observe が状態更新を発行より前に実施** — 2026-11-23 修正（canonicalize(arguments) の二重計算を1回に統合。後続処理が失敗しない現在の実装では状態順序による実害なし、R14 指摘の small 部分を反映）。
- [x] **format_file_mention の Directory quoted 分岐が doc/テスト未固定** — 2026-11-23 修正（Directory の quoted mention が末尾引用符を省略する grammar を doc 明記、File との対比を固定する回帰テスト追加、R14 指摘反映）。
- [x] **WorkingDirectory::set の「絶対パスを返す」(doc) が未検証** — 2026-11-23 修正（相対 path が保存済み cwd に対して絶対化されること、後続の相対 path がその cwd 基準で解決されることを固定する回帰テスト追加、R14 指摘反映）。
- [x] **JobHandle::read の offset 範囲外挙動が doc/テスト未定義** — 2026-11-23 修正（範囲外 offset が clamp され空文字＋現在 end を返す挙動を doc 明記、usize::MAX で固定する回帰テスト追加、R14 指摘反映）。
- [x] **RetryPolicy::should_retry の retry>=max_retries 境界が doc/テスト未固定** — 2026-11-23 修正（既存テストが retry=3=max_retries で Stop を固定済みと確認、doc に予算枯渇条件を明記、R14 指摘反映）。
- [x] **EventObserver が append_with_seq で発火しない契約が未テスト** — 2026-11-23 修正（append は observer 発火、append_with_seq は発火しないことを対で固定する回帰テスト追加、JSONL restore の二重追記防止を担保、R14 指摘反映）。

- [x] **rollback 修正が command_goal のみで同型 mount helper が未修正** — 2026-11-23 修正（command_feedback / tool_session_query / skill_filesystem の3 mount に command_goal と同じ Disposer rollback を横展開、provide_session_query / provide_skills に with_disposer を追加し既存 provide は adapter 維持、host 所有 service は対象外、R15 指摘反映）。
- [x] **fs edit_text の previous 取得と reserve_version が別ロックで rollback 値が誤り得る** — 2026-11-23 修正（reserve_version が (previous, next) を単一 write guard 内で返すように変更、write_text / edit_text の両呼び出し側を更新し失敗時 rollback 値を原子的に確定、R15 指摘反映）。
- [x] **fs write_text が version guard 無しで既存 version を直接上書き** — 2026-11-23 修正（write_text_checked を追加して expected_version を渡せるようにし、既存 write_text は unguarded adapter として維持、stale version 拒否の回帰テスト、R15 指摘反映）。
- [x] **JSONL poison が append のみで read / restore が古い文件内容を返す** — 2026-11-23 修正（check_not_poisoned ヘルパーを追加し read / read_recovered の冒頭でチェック、restore_with は read 経由で間接的に遮断、poison 後の全 read 経路が Corrupt を返す回帰テスト、R15 指摘反映）。
- [x] **tool_bash が shell provide の Disposer を捨て mount 失敗で shell が残る** — 2026-11-23 修正（shell provide の Disposer を保持し Fiber mount 失敗時に dispose、host 提供の shell は所有権尊重で対象外、R14-RB08 の approval 順序も維持、R15 指摘反映）。
- [x] **mcp_resources が effect 登録自体の失敗時に service が残る** — 2026-11-23 修正（effect の Result を inspect し、disposer が scope に渡る前に失敗した場合は明示 dispose、R14-RB06 の scope 登録と併せて全失敗経路をカバー、R15 指摘反映）。
- [x] **GoalStore copy-swap が events Vec を毎回 deep copy（性能）** — 2026-11-23 修正（commit が goal / next_revision のみを一時 view にコピーして apply し、成功時に書き戻し＋events を in-place push に変更、検証エラー時不変契約は維持、R1 で apply の全分岐が「検証→変更」であることを実証、R15 指摘反映）。
- [x] **fs tmp ファイル名が pid+nanos で並列 write が衝突し得る** — 2026-11-23 修正（FsState に write_seq を追加し tmp 名に採番を含めて同一プロセスの並列 write でもユニークに、R15 指摘反映）。
- [x] **fs create_dir_all 失敗時に created_dirs を rollback しない** — 2026-11-23 修正（rollback_dirs ヘルパーを抽出し、create_dir_all 失敗時と write 失敗時の両分岐から深い順の remove_dir を実行、R15 指摘反映）。
- [x] **fs reserve_version の next = current+1 が u64 overflow unchecked** — 2026-11-23 修正（checked_add(1) に変更し overflow 時に file version overflow エラーを返す、debug panic / release wrap を防止、R15 指摘反映）。
- [x] **SessionTitles max_chars=0 が黙って 1 に丸められる（contract 未検証）** — 2026-11-23 修正（set_generated / derive_from_messages の doc に「0 は 1 として扱う」と明記し、両メソッドで 1 文字になる回帰テスト追加、R15 指摘反映）。
- [x] **JobRegistry::new の max_concurrent / retain_bytes 下限 clamp が untested** — 2026-11-23 修正（0 が max_concurrent=1 / retain_bytes=1024 に丸まることを start＋append＋read で観測する回帰テスト追加、R15 指摘反映）。
- [x] **AgentLoop::with_max_steps(0) が 1 に丸められる契約が untested** — 2026-11-23 修正（scripted provider で max_steps(0) の loop が1ステップで終わることを固定する回帰テスト追加、R15 指摘反映）。
- [x] **NativeToolExecutor::with_max_parallel(0) の 1 丸めが untested** — 2026-11-23 修正（with_max_parallel(0) でも echo ツール呼び出しが正常完了することを固定する回帰テスト追加、R15 指摘反映）。
- [x] **MessageFeedbackConfig::max_note_bytes=0 の拒否時挙動が doc 未記載** — 2026-11-23 修正（new() が max_note_bytes=0 を fail-fast で拒否する旨を doc 明記、回帰テスト追加、R15 指摘反映）。
- [x] **CompactionAutomationConfig の pressure_threshold > 1.0 / NaN 扱いが untested** — 2026-11-23 修正（mount 側の is_finite / (0,1] 検証は実装済みと Runtime Proof で確認、NaN / inf / 0 / -1 / 1.5 を拒否する境界テストを追加、R15 指摘の未テスト部分を反映）。

- [x] **JsonlSessionHandle::append が poison helper を使わず Error::Other を返す（read と非対称）** — 2026-11-24 修正（append も check_not_poisoned() に統一し Domain("persistence") エラーを返す、エラー種別を固定する回帰テスト、R16 指摘反映）。
- [x] **ToolResultPruner::new が MAX 近傍で head_tail + marker_len overflow** — 2026-11-24 修正（checked_add に変更し MAX ガードを吸収、MAX 近傍の回帰テスト追加、R16 指摘反映）。
- [x] **fs write_seq が wrapping_add で一意性保証が方針と割れている** — 2026-11-24 修正（checked_add に変更し overflow 時に write sequence overflow エラー、version カウンタと fail-closed 方針を統一、R16 指摘反映）。
- [x] **GoalStore::commit の view が events: Vec::new() で apply に渡す契約変化が未明記** — 2026-11-24 修正（apply が履歴を参照しない契約を doc 明記、実際の event log は commit が検証成功後に push、R16 指摘反映）。
- [x] **CredentialProvider::with_env が self を consume し doc との接続が弱い** — 2026-11-24 修正（set_env(&self) を追加し登録済み provider も更新可能に、with_env はその adapter として維持、共有状態の回帰テスト、R16 指摘反映）。
- [x] **RetryPolicy::backoff_ms が random=NaN を finite ガードしない** — 2026-11-24 修正（random が非有限値なら中央値 0.5 にフォールバック、NaN 等が jitter を歪めないようにし回帰テスト追加、R16 指摘反映）。
- [x] **attachment_admission が max_bytes=0 の files/images 扱いを未定義** — 2026-11-24 修正（挙動は既に正しいと R1 が Runtime Proof で確認、doc に「0 は files の空ペイロードのみ許可／images は常に拒否」と明記し回帰テスト追加、R16 指摘反映）。
- [x] **FileSystem::read_window が limit=usize::MAX で全行保持し bounded reads を迂回** — 2026-11-24 修正（read_window が tool_fs::READ_LIMIT で limit を clamp、doc 明記、巨大 limit の回帰テスト、R16 指摘反映）。
- [x] **JobError::MaxConcurrent 分岐がテスト未固定** — 2026-11-24 修正（owner ごとの並列上限到達・文言・settle 後のスロット解放・他 owner の独立予算を固定する回帰テスト追加、R16 指摘反映）。
- [x] **GPUI SwitchErrorKind::Preset が cordis-rs のエラー文言に依存** — R6 Runtime Proof で誤検知と判定（GPUI は cordis-rs の文言を contains／parse せず `e.to_string()` を保持するだけ、grep 参照0件。R16-DUAL03 の contains 依存とは別类别で定数追加不要）。
- [x] **GPUI agent UI が ToolError 文字列 contains 判定で cordis-rs 文言変更に脆弱** — 2026-11-24 修正（cordis-rs が APPROVAL_DENIED_MARKER / PATH_ESCAPE_MARKER を公開し生成側も定数を使用、GPUI の assertion も同じ定数を参照、文言・エラー型は不変、R16 指摘反映）。R12-IV06 の構造化は別 ADR 案件。
- [x] **GPUI に ToolOutput::is_error()==true の回帰テストが無い** — 2026-11-24 修正（ToolOutput::from_error が is_error()==true、通常出力が false を固定する回帰テスト追加、R16 指摘反映）。
- [x] **GPUI schedule UI に Cancelled terminal の描画分岐が無い（要確認）** — R6 Runtime Proof で誤検知と判定（GPUI src/bin に schedule UI 自体が存在せず `ScheduleStatus` / `provide_schedule` 参照0件。描画分岐の有無を問う前提が成立しない。新規 schedule UI が必要なら別タスクとして起票）。

- [x] **PlanMode::set / enter が空 session_id で panic（try_set のみ安全）** — 2026-11-24 修正（panic 条件を doc 明記、should_panic テストで契約固定、コード変更なし後方互換、R17 指摘反映）。
- [x] **StorageHub::write が required_fields[0] を直接 index** — 2026-11-24 修正（first().cloned().unwrap_or_default() に変更し、ガード変更時に index out of bounds 化する脆さを除去、R17 指摘反映）。
- [x] **OutputRing::append の drain が UTF-8 境界を越えて多バイト文字を lossy 化** — 2026-11-24 修正（drain 位置を UTF-8 境界まで前進、多バイト文字列で U+FFFD が出ない回帰テスト、R17 指摘反映）。
- [x] **loader split_top_level の depth が i32 で負に転落し区切りを認識しなくなる** — 2026-11-24 修正（split_top_level / split_flow の depth を saturating_add/sub に変更、不正な閉じ括弧でも depth==0 の区切り判定が維持される、R17 指摘反映）。
- [x] **loader evaluate_expression の再帰上限が入力長/64 依存で高密度入れ子に弱い** — 2026-11-24 記録（R1 Runtime Proof で TooDeep による fail-closed が既に機能し機能バグなしと判定、測定根拠なしで上限変更せず現状維持、limits_for の上限式と事例待ち方針を doc 明記、R17 指摘反映）。
- [x] **loader ${...} 補間が入れ子 } を考慮せず最初で終端** — 2026-11-24 修正（入れ子検出時に InvalidExpression("nested interpolation is not supported") を返し、誤解を招く UnresolvedReference を防止。非入れ子補間は不変、回帰テスト追加、R17 指摘反映）。
- [x] **fs read_window が巨大な1行を丸ごと String 保持し DoS 耐性が残る** — 2026-11-24 記録（R1 Runtime Proof で「返す String は既に max_line_length に clamp 済み、ピーク削減には chunk スキャン（large）が必要」と判定。既知制約をコード doc に明記、完全対応は別 ADR 案件として残す）。
- [x] **JobRegistry::wait が timeout_ms を clamp せず実質無限待ちを作れる** — 2026-11-24 修正（service 側 wait も tool_jobs::MAX_WAIT_TIMEOUT_MS で clamp、tool 経由のガードを直接呼び出しでも迂回不可に、R17 指摘反映）。
- [x] **GPUI ACP 起動エラー4分岐が文字列のみでテスト未固定** — 2026-11-24 修正（4分岐の prefix を StartupErrorKind 定数＋message ヘルパーに集約、外部 dsh なしで prefix 生成を検証する回帰テスト追加、status 文言は不変、R17 指摘反映）。
- [x] **title_from_output の * / ** / *** と nested 拒否を doc/テスト固定** — 2026-11-24 修正（3幅の emphasis と nested 時の verbatim 保持を doc 明記＋回帰テスト追加、R17 指摘反映）。
- [x] **GPUI TitlePanel::apply_build_result の Err で旧 rows を捨てる契約が doc 未明記** — 2026-11-24 修正（失敗時に旧 rows を消して古いデータを表示しない契約を doc 明記、既存テストが空 rows を固定済み、R17 指摘反映）。
- [x] **GPUI SwitchErrorKind::Service が preset エラーを含む境界が未固定** — 2026-11-24 処理（「Service が preset エラーを含む」は実装上 false で誤検知、残余の「分岐境界が未固定」を classify_error ヘルパー＋テストで固定、R17 指摘反映）。
- [x] **WorkspaceRegistry archived の get=Some / attach=Err 非対称が doc 未明記** — 2026-11-24 修正（archived は表示用に get/list で返るが mutation は拒否する契約を doc 明記、既存テストに get=Some を追加固定、R17 指摘反映）。
- [x] **LlmService::resolve(None) が default 未設定で None を返す分岐が未テスト** — 2026-11-24 修正（空 registry で resolve(None) / resolve(Some(missing)) が None を返す回帰テスト追加。provider 登録済み・default 未設定は register が自動 default 化するため到達不能、R17 指摘反映）。

- [x] **TimeoutPolicy::default_timeout_ms の None 意味が service/port で食い違う** — R6 Runtime Proof で誤検知と判定（service の「no timeout」と port の「pass through」は同じ contract、単一の timeout_of に集約され port 側に独自既定なし、None/Some(0)→None は既存テスト固定済み）。
- [x] **SessionQueryEngine::trace の 'in seq order' が seq 飛び時に保証されない** — R6 Runtime Proof で誤検知と判定（append_with_seq が seq <= next_seq を拒否し log.events は常に seq 単調、trace はそのまま clone するだけ。「seq order」は昇順の意味で連続の意味ではない、gap 単調は既存 restore テスト固定済み）。
- [x] **GPUI resolve_patch_path の非 UTF-8 パス扱いが未定義** — R6 Runtime Proof で誤検知と判定（OsString を Path::new に直接渡し is_file チェック後そのまま返すロスレス経路、to_string_lossy 未使用、エラーは {:?} で escape 表示。非 UTF-8 は既に正しく扱われており、lossy 化を入れる提案は逆に有害）。
- [x] **fs read_window の clamp が total_lines に効くか未固定** — 2026-11-24 修正（clamp は保持行のみに効き total_lines は全行を数える契約を doc 明記、READ_LIMIT 超のファイルで固定する回帰テスト追加、R18 指摘反映）。
- [x] **retry should_retry_message が error_code() を経由するか未固定** — 2026-11-24 修正（Runtime Proof で経由を確認し、ラベル付き HTTP コードは Retry／「retry in 500ms」は Stop を固定する回帰テスト追加、R18 指摘反映）。
- [x] **PruneError::RemovalNotFound が生成箇所ゼロのデッド契約** — 2026-11-24 修正（reserved／現在は未 emit を doc 明記、バリアント削除による後方互換破壊は回避、R18 指摘反映）。
- [x] **ApprovalError::Cancelled が生成箇所ゼロのデッド契約** — 2026-11-24 修正（reserved／現在は未 emit を doc 明記、バリアント削除による後方互換破壊は回避、R18 指摘反映）。
- [x] **UserQuestionError の phase-2 予約4バリアントが code()/Display に漏れる** — 2026-11-24 修正（code()  containing は R2-ER06 の public code-set stability で意図的と確認、4バリアントに「never emitted / kept for stability」を doc 明記、既存 code 固定テストは不変、R18 指摘反映）。
- [x] **ShellError の TimedOut 系が timeout_ms を Display で落とす** — 2026-11-24 修正（timeout_ms() getter を追加し構造化値として公開、Display 文言と retry 誤検出回避は不変、回帰テスト追加、R18 指摘反映）。
- [x] **FsError の PartialEq が cause を無視する契約が doc 未明記** — 2026-11-24 修正（安定した分類フィールドのみで比較し cause は除外する意図を doc 明記、別 Arc インスタンスが等しい回帰テスト追加、R18 指摘反映）。
- [x] **RawTodo::new が検証を to_todo_list に遅延する旨が doc 未明記** — 2026-11-24 修正（検証遅延の契約を doc 明記、不正値が constructor を通り to_todo_list で拒否される回帰テスト追加、R18 指摘反映）。
- [x] **ToolExecution::agent("") が空名を受理し anonymous と区別される** — 2026-11-24 修正（空／空白名を None に正規化し anonymous と同一視、正常名は前後空白含めて不変、回帰テスト追加、R18 指摘反映）。
- [x] **PresetSpec が pub フィールドで register が内容を検証しない** — 2026-11-24 修正（sandbox/approval は enum で型保証、label/description は optional メタデータで検証不要という設計を doc 明記、None の spec が通る回帰テスト追加、R18 指摘反映）。
- [x] **TokenMeter::new が budget=0 を 1 に丸める意図が doc 未明記** — 2026-11-24 修正（0 → 1 の丸めが pressure を有限にし fail-closed にする意図を doc 明記、finite 性の回帰テスト追加、R18 指摘反映）。
- [x] **acknowledge の doc 冒頭が実装と逆だった（DUAL3-02 修正済みの記録）** — R18-DUAL3-02 / 完了済み（2026-11-24、ドキュメント＋clamp テスト）

- [x] **validate_refresh_interval が 0 を受理し毎ステップ注入になる** — 2026-11-24 修正（0 拒否はデフォルト設定の TimeContext 生成を壊すため却下、0 = 毎ステップ注入を doc 明記し回帰テスト追加、R19 指摘反映）。
- [x] **TimeoutPolicy::check が timeout_of を再解決し 0ms メッセージを作り得る** — 2026-11-24 修正（budget を1回だけ解決して期限判定と報告値を一致させ、unwrap_or(0) の自己矛盾メッセージを排除、既存テスト不変、R19 指摘反映）。
- [x] **shell with_timeout(0) が即タイムアウトになり hooks 側と非対称** — 2026-11-24 修正（run_with_timeout で timeout_ms=0 を InvalidCommand として構造化拒否、builder シグネチャと None/正数は不変、回帰テスト追加、R19 指摘反映）。
- [x] **telemetry が as_millis() の u128→u64 cast を黙って truncate** — 2026-11-24 修正（u64::try_from(...).unwrap_or(u64::MAX) に変更し、極端な未来時刻でも記録時刻が wrap しないようにした、R19 指摘反映）。
- [x] **retry backoff が 2^53 超の max_delay_ms を f64 で丸める** — 2026-11-24 記録（R1 Runtime Proof で到達困難な設定_domain の問題と判定、2^53 超の精度限界を max_delay_ms の doc 明記、コード変更なし、R19 指摘反映）。
- [x] **tool_jobs の負の timeout_ms が黙って DEFAULT にフォールバック** — 2026-11-24 修正（キー欠落は DEFAULT、数値だが u64 変換不可は InvalidArgs、非数値も InvalidArgs、正数は clamp のまま、回帰テスト追加、R19 指摘反映）。
- [x] **schedule due_ms の単位（Unix ms）が doc 未明記で秒渡しを検出できない** — 2026-11-24 修正（create / due の引数が Unix epoch milliseconds であることを doc 明記、既存 due テストが同単位を固定済み、R19 指摘反映）。
- [x] **LLM total timeout が budget 値をログに出さない（per-chunk と非対称）** — 2026-11-24 修正（total timeout エラーにも `after {n}ms` を追加し per-chunk と対称に、既存テストに budget 値のアサーション追加、R19 指摘反映）。
- [x] **ms u64 と Duration の混在に対する README 単位ガイドが無い** — 2026-11-24 修正（公開 API は `*_ms: u64`、llm_resilience decorator のみ Duration という単位規約を README に明記、R19 指摘反映）。
- [x] **StorageHub の空白名が受理され GoalStore/Shell と非対称** — 2026-11-24 修正（try_register_backend / try_define_domain を trim().is_empty() 拒否に揃え、空白名の回帰テスト追加、R19 指摘反映）。
- [x] **SessionQueryEngine::search の Unicode 正規化が text_search と非対称** — 2026-11-24 修正（lowercase のみで accent/fullwidth 正規化なしと doc 明記、cafe が café に hit しない契約を回帰テスト追加。normalize 適用は検索セマンティクスが広がるため却下、R19 指摘反映）。
- [x] **CredentialProvider::set が空白値を受理し resolve も解決可能** — 2026-11-24 修正（set と resolve の両方で空白値を未設定扱いに統一、空白値が拒否され real 値は前後空白保持で解決する回帰テスト追加、R19 指摘反映）。
- [x] **GPUI secret mask と OTLP redaction の大小文字扱いが非対称（要確認）** — R6 Runtime Proof で誤検知と判定（GPUI は denylist の大小無視、OTLP は allowlist の大小厳密で役割が逆。いずれも大小違いのキーを redact 側に倒し漏洩方向の非対称なし。case-insensitive 化は fail-open 化するため現状維持）。
- [x] **user_questions の serde 必須フィールド契約が doc/テスト未固定** — 2026-11-24 修正（Option 以外のフィールドは deserialize 時に必須という契約を doc 明記、欠落時に Err となる回帰テスト追加、R19 指摘反映）。
- [x] **ToolRegistry / CommandRegistry が空名を検証せず登録** — 2026-11-24 修正（ToolRegistry に try_register を追加し空名を拒否、既存 register は Disposer/last-write-wins 不変。CommandRegistry 側は CommandDefinition::new が既に construction 時に拒否しており誤検知気味であることを記録、R19 指摘反映）。

- [ ] **OTel channel_names の順序が doc 未明記（実装は Vec で決定的）** — 根拠 `src/services/telemetry.rs:252-260` / R20-ID01 / 重大度 low / small
- [ ] **SessionQueryEngine 再 index で古い Session ハンドルが divergence する** — 根拠 `src/services/session_query.rs:38-47` / R20-ID02 / 重大度 low / small
- [ ] **SystemPrompt::section の disposer が name のみで fencing 無し** — 根拠 `src/services/system_prompt.rs:78-99` / R20-ID04 / 重大度 low / small
- [x] **LlmService::names が同名 provider の重複を表示する** — 2026-11-24 修正（同名重複は R13-OW02 の id 所有の正当な帰結と判定、names() が registration ごとに1件返す契約を doc 明記＋回帰テスト追加、try_register 化は回帰テスト前提を壊すため却下、R20 指摘反映）。
- [ ] **Schedule next_id が overflow で既存 ID と衝突し得る** — 根拠 `src/services/schedule.rs:60,87-89` / R20-ID06 / 重大度 low / small
- [ ] **ApprovalService が同名 answerer を二重登録できる** — 根拠 `src/services/approval.rs:152-173` / R20-ID07 / 重大度 low / small
- [ ] **SubagentRegistry が重複登録を検出できない** — 根拠 `src/services/subagents.rs:57-77` / R20-ID08 / 重大度 low / small
- [ ] **Telemetry event_channel の id overflow で古い disposer が新 channel を消す** — 根拠 `src/services/telemetry.rs:71-72,232-248` / R20-ID09 / 重大度 low / small
- [ ] **GPUI tag_from_json が未知タグを `<unknown>` に丸めて観測不能** — 根拠 `acp_stream_ui.rs:235-251` / R20-DUAL5-01 / 重大度 low / small
- [x] **AskRequest の options / multi_select が必須で upstream 互換性が未確認** — 2026-11-24 修正（options と multi_select に #[serde(default)] を付与し最小 payload を受理、既存フィールド埋まり JSON は不変、回帰テスト追加。upstream schema の根拠はローカルに無く未確認として記録、R20 指摘反映）。
- [ ] **Answer が unknown fields を許す（upstream schema と非対称・要確認）** — 根拠 `src/ports/user_questions.rs:98-101` / R20-DUAL5-03 / 重大度 low / small
- [ ] **型付き Event の externally-tagged wire 形が未固定** — 根拠 `src/events.rs:68,86` / R20-DUAL5-04 / 重大度 low / small
- [ ] **GPUI entries toggle UI の state 再読込で未知キーの扱いが未固定** — 根拠 `cordis_entries_toggle_ui.rs:843` / R20-DUAL5-05 / 重大度 low / small

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
- **swarm コンボ R6 集約（2026-11-21、round 89）**: `swarm-project-review` R1（AR/CF/DR）と `swarm-todo-discovery` R1（ST/FG）の2エージェントから17件を取得。重複0件、却下0件、17件すべてを根拠 confirmed で `## 未着手` へ追記（子の重大度を reference として親が再判定: high2 / medium12 / low3）。並列2・深度1の制限プロファイル。R2〜R5 は同一情報の重複取得を避けるため省略し、R1 で打ち切る。
- **swarm コンボ R6 集約（2026-11-22、round 102）**: `swarm-project-review` R1（AR/CF/DR）と `swarm-todo-discovery` R1（ST/FG/TQ/DC）の2エージェントから16件を取得。既知の hooks-claude-code 重複は Codex 側の未接続として整理して追記。root が path:line を Runtime Proof で確認し16件を `## 未着手` へ追記（high1 / medium10 / low5、すべて small/medium の実装容易性）。R2〜R5 は同一情報の重複取得を避け省略。
- **swarm コンボ R6 集約（2026-11-22、round 119）**: `swarm-project-review` R1（AR/CF/AP/DC）と `swarm-todo-discovery` R1（ST/TQ/DC）の2エージェントから14件を取得。root が path:line を Runtime Proof で確認し14件を `## 未着手` へ追記（high1 / medium2 / low11）。R2〜R5 は同一情報の重複取得を避け省略。
- **swarm コンボ R6 集約（2026-11-22、round 132）**: `swarm-project-review` R1（AR/CF/AP/DC）と `swarm-todo-discovery` R1（ST/TQ/DC）の2エージェントから14件を取得。root が path:line を Runtime Proof で確認し14件を `## 未着手` へ追記（medium9 / low5）。R2〜R5 は同一情報の重複取得を避け省略。
- **swarm コンボ R6 集約（2026-11-22、round 148）**: `swarm-project-review` R1 と `swarm-todo-discovery` R1 の2エージェントから15件を取得。root が path:line を Runtime Proof で確認し、重複2件を統合して13件を `## 未着手` へ追記（high1 / medium9 / low3）。agent-12 の disposer「10箇所」は実測8箇所に修正。
- **swarm コンボ R6 集約（2026-11-22、round 161）**: `swarm-project-review` R1 と `swarm-todo-discovery` R1 の2エージェントから18件を取得（agent-13 は structured_output が利用不可のため通常メッセージで再送）。root が path:line を Runtime Proof で確認し18件を `## 未着手` へ追記（high1 / medium9 / low8）。
- **swarm コンボ R6 集約（2026-11-22、round 186）**: specialist 2体（cordis-researcher / plugin-ports）から18件取得。R6 で path:line を Runtime Proof で確認し、修正済み1件（ST01-06 port 名 gate）を除外して17件を追記。
- **swarm コンボ R6 集約（2026-11-23、round 199）**: specialist 2体（cordis-researcher / plugin-ports）から16件取得。R6 で path:line を Runtime Proof で確認し17件を追記（IV02 は既出メモのタスク化、うち1件は同根で重複カウント）。観点を「入力検証・エラー契約・トランザクション整合性」と「テスト・ドキュメント・ゲート乖離」に絞った再走。
- **swarm コンボ R6 集約（2026-11-23、round 215）**: specialist 2体（cordis-researcher / plugin-ports）から20件取得。R6 で path:line を Runtime Proof で確認し、1件を誤検知として除外（RS05 は既に guard を drop してから answerer を呼ぶ実装）して19件を追記。観点を「並行処理・所有権・ロック」と「未使用 API・re-export 漏れ」に絞った再走。
- **swarm コンボ R6 集約（2026-11-23、round 228）**: specialist 2体（cordis-researcher / plugin-ports）から17件取得。R6 で path:line を Runtime Proof で確認し、1件を誤検知として除外（IV10 は前ラウンドで acknowledge に clamp 追加済み）して16件を追記。観点を「境界値・冪等性」と「doc comment と実装の乖離」に絞った再走。
- **swarm コンボ R6 集約（2026-11-23、round 242）**: specialist 2体（cordis-researcher / plugin-ports）から17件取得。R6 で path:line を Runtime Proof で確認し17件を追記。観点を「Send/Sync・所有権・clone の深さ」と「未固定の公開 API 契約」に絞った再走。
- **swarm コンボ R6 集約（2026-11-23、round 253）**: specialist 2体から16件取得。R6 で path:line を Runtime Proof で確認し、1件を誤検知として除外（ST01f-01 は next_seq 初期値が0で seq=1 受理）、15件を追記。観点を「エラーパスの部分状態」と「doc/実装の矛盾」に絞った再走。
- **swarm コンボ R6 集約（2026-11-23、round 266）**: specialist 2体から16件取得。R6 で path:line を Runtime Proof で確認し16件を追記。観点を「直近ロールバック／原子化修正周辺の非対称」と「入力検証・境界値の未固定」に絞った再走。
- **swarm コンボ R6 集約（2026-11-24、round 297）**: specialist 2体から16件取得。R6 で path:line を Runtime Proof で確認し、3件を除外（RA07 は前回 doc 明記済み、RA10 は実害なし、DUAL05 は前回対処済み）し13件を追記。観点を「直近修正ラッシュの残余非対称」と「片側だけ更新されたエラー型・状態」に絞った再走。
- **swarm コンボ R6 集約（2026-11-24、round 311）**: specialist 2体から16件取得。PN01 は即修正、PN05 は Runtime Proof で panic 経路なしとして除外、残り14件を追記（追記処理の欠落を round 312 で検出して補完）。観点を「公開 API の panic 経路」と「未固定の Err/None 分岐」に絞った再走。
- **R18 コンボ（round 327）で DUAL3-02 を修正**: acknowledge の doc 冒頭を実装（drain + cursor rebase）に合わせ、through=usize::MAX の clamp テストを追加（2026-11-24）。
- **R19 コンボ（round 342）**: specialist 2体から16件取得。TM04 は即修正（ms→ns 乗算を saturating_mul に）、残り15件を追記。観点を「時刻・期間の単位変換と境界」および「空白・大小文字・serde 既定値の非対称」に絞った再走。
- **R20 コンボ（round 357）**: specialist 2体から15件取得。ID03 は即修正（同 rank を last-write-wins に統一）、残り14件を追記。観点を「ID/名前の衝突・一意性・順序契約」および「serde 表現安定性と読む側の対応」に絞った再走。ID01/ID10 は Runtime Proof で却下候補。
