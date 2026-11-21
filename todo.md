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
- [ ] **23個のドメイン Error enum が Error::Other に平坦化され種別分岐不能** — 根拠 `src/error.rs:4,30,34`, `src/loader.rs:108` / R1-AP02 / 重大度 medium / confirmed
- [ ] **port 初期化 API が mount と provide の二系統（lifecycle も不同）** — 根拠 `src/ports/tool_fs.rs:mount`, `src/ports/fs_patch.rs:provide` / R1-AP03 / 重大度 medium / confirmed
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
- [ ] **hooks-claude-code の updatedInput/systemMessage/Ask/SessionStart が未接続** — 根拠 `src/ports/hooks_claude_code.rs:137-142` / R2-FG03 / 重大度 low / confirmed
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
- [ ] **ApprovalService に agent/callId/audit pair/allowed-once が無い** — 根拠 `docs/ADR-012-shell-command-approval.md:67-68,104-107`, `src/services/approval.rs:14-19` / R2-FG02 / 重大度 medium / confirmed
- [ ] **sandbox escalation の共通 helper が無く実行ポイントへ接続されていない** — 根拠 `docs/ADR-012-shell-command-approval.md:108-109`, `src/ports/sandbox_policy.rs:1-6` / R2-FG03 / 重大度 medium / confirmed
- [ ] **completion notice 配信とモデル cursor 追跡が未実装** — 根拠 `docs/ADR-002-tool-jobs-port-scope.md:16,21-24,28-29` / R2-FG05 / 重大度 medium / confirmed
- [x] **job owner が固定ラベル "bash" で sessionId fence が弱い** — 2026-11-21 修正（background job の owner を exec.session に変更、session 単位の fence テスト追加、ADR-002 更新、R1 指摘反映）。
- [ ] **JSONL 永続化が通常 turn へ自動配線されていない** — 根拠 `README.md:85`, `src/ports/session_persistence_jsonl.rs:348-351` / R2-FG07 / 重大度 medium / confirmed
- [ ] **FileSystem::scoped の symlink TOCTOU 残存** — 根拠 `README.md:82`, `src/services/fs.rs:83-125` / R2-FG08 / 重大度 medium / confirmed
- [x] **session-query port が SQLite/FTS5 ではなく決定的インメモリ検索の差分が README に無い** — 2026-11-21 修正（安定性マトリクスに phase-1 行と FTS5 未実装を明記、R2-FG09 反映）。
- [x] **v1.0.0 での旧名 alias 削除が未トラック（deprecated 7個）** — 2026-11-21 修正（ADR-005 に v1.0.0 削除チェックリストと検証手順を追加、R2-FG10 反映）。
- [x] **クレート版数とリリースタグの記述が Cargo.toml / PROGRESS / ADR-001 で食い違う** — 2026-11-21 修正（PROGRESS/ADR-001 のタグ範囲を v0.261.0 に最新化、README に crate version 0.1.0 固定・タグは時系列マーカー方針を明記、R1-CF03 反映）。

- [x] **tool_todo::mount が "sessionProjections" を core と同名で提供し二重登録で衝突** — 2026-11-22 修正（`TODO_PROJECTIONS_SERVICE_NAME` = "todoProjections" に改名、provide/inject/apply 3箇所同時、core 側不変、両順序共存テスト追加、R2 指摘反映）。
- [x] **inject() が空なのに apply() で require() する port（tool_schedule / tool_exit_plan_mode）** — 2026-11-22 修正（両 port の inject() に依存名を宣言、回帰テスト追加、R2 指摘反映）。
- [x] **SessionTitles::derive_from_messages が get→set 別ロックで LLM 生成タイトルを上書き** — 2026-11-22 修正（set_if_absent で単一 write guard の原子的挿入、回帰テスト追加、R2 指摘反映）。
- [x] **CredentialProvider::set が参照名規則を検証せず解決不能な値を受理** — 2026-11-22 修正（大文字・数字・アンダースコアのみ許可、不正名は false、回帰テスト追加、R2 指摘反映）。
- [ ] **PlanMode セッション別ログが無上限で蓄積し削除 API も無い** — 根拠 `src/services/plan_mode.rs:20-23,36-49` / R2-OB02 / 重大度 medium / confirmed
- [x] **FieldSpec::range に min>max スキーマ不正チェックが無い** — 2026-11-22 修正（range()/range_int() に assert!(min<=max) 追加、should_panic テスト2件、R2 指摘反映）。
- [x] **EventLogReporter::with_max_counters(0) が黙って 1 に丸め validate と方針不一致** — 2026-11-22 修正（assert!(max_counters > 0) で fail-fast、should_panic テスト追加、R2 指摘反映）。
- [ ] **DeepSeek 拡張の warn-once 集合が永続し初回以降の provider 失敗が不可視** — 根拠 `src/ports/deepseek_llm_api_extensions.rs:30,86-105` / R2-OB03 / 重大度 medium / confirmed
- [x] **McpResourceRegistry::register_scoped の bool が他レジストリと逆意味、unregister が空スロープを残す** — 2026-11-22 修正（新規登録で true、空スコープを remove、回帰テスト追加、R2 指摘反映）。
- [ ] **SessionQueryEngine::index がスナップショット保持で更新が反映されず unindex 手段も無い** — 根拠 `src/services/session_query.rs:38-44` / R2-DI02 / 重大度 medium / likely
- [x] **production_ports.rs の本番 port カバレッジが3件に留まる（tool_fs/tool_bash/session_persistence_jsonl を追加）** — 2026-11-22 修正（3 port の mount スモークを追加、計6件の本番 port を検証、R3 指摘反映）。
- [ ] **未生成の error enum バリアントが API サーフェスに残る（UserQuestionError 等）** — 根拠 `src/ports/user_questions.rs:25-35`, `src/ports/tool_exit_plan_mode.rs:17-20` / R2-ER06 / 重大度 low / confirmed
- [ ] **README のテストファイル一覧に tests/production_ports.rs が載っていない** — 根拠 `README.md:168-171` / R3-DC06 / 重大度 low / confirmed
- [ ] **verify.sh がリポジトリ直下を cwd 前提でスクリプト参照表記も混在** — 根拠 `scripts/verify.sh:4-44` / R3-CF04 / 重大度 low / confirmed
- [ ] **CodexToolPayload が未使用で hook rewrite 相当が未接続（Claude と同じ ADR で扱う）** — 根拠 `src/ports/hooks_codex.rs:19-25,119-145` / R3-FG04 / 重大度 low / likely
- [ ] **check-doc-counts.sh が core service 数（38）を検証しない** — 根拠 `scripts/check-doc-counts.sh:8-26`, `README.md:30` / R3-CF05 / 重大度 low / likely

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
