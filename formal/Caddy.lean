/-!
 Caddy ゲート挙動の形式化（R1 の命題1〜3）。

 Bool 変数:
 - H : 正しい Host かどうか（not host 判定の入力）
 - O : パスが /oauth2/* 配下かどうか
 - A : forward_auth の応答が 401 以外かどうか
 - T : /trial* パスかどうか（:18888 のみ必須）
 応答:
 - R400 / R302 / U : Response → Bool

 実ホスト名・トークン値・具体的な upstream アドレスは一切含めない。
 未確認点はコメントに明記し、推測で埋めていない。
-/

/-- 応答結果。outOfScope は本命題の対象外（未確認の挙動）を表す。 -/
inductive Response where
  | badHost    -- 不正 Host に対する 400
  | redirect   -- 401 を受けた未認証リクエストの 302
  | upstream   -- アップストリームへの中継
  | outOfScope -- 本モデルが判定しない応答（oauth2-proxy 等）
  deriving Repr, DecidableEq

/-- 400 応答かどうか -/
def R400 : Response → Bool
  | Response.badHost => true
  | _ => false

/-- 302 応答かどうか -/
def R302 : Response → Bool
  | Response.redirect => true
  | _ => false

/-- アップストリーム到達かどうか -/
def U : Response → Bool
  | Response.upstream => true
  | _ => false

/-- ゲートの添字。:18888 のみ trial パス必須。 -/
inductive Gate where
  | g18888 | g18889 | g18890 | g18891
  deriving Repr, DecidableEq

/-- 当該ゲートが T（trial パス）を要求するか。 -/
def needsTrial : Gate → Bool
  | Gate.g18888 => true
  | _ => false

/--
 Caddy の判定順を表す関数。

 未確認点（推測で埋めていない）:
 - Caddy の `not host` の厳密な意味論は未検証。ここでは H = false を ¬H として扱う。
 - `respond` が後続ディレクティブを遮断するかは未確認。badHost を終端応答としてモデル化する。
 - O = true のときの oauth2-proxy の実動作は未確認のため outOfScope とし、
   具体的なステータス・ヘッダはモデルに含めない。
 - :18888 で T = false のときの実際の応答（404 等）は未確認のため outOfScope とし、
   U = false のみを主張する。
 - A の判定方法（forward_auth が 401 以外であることの検出）は未確認のため入力変数とする。
 - 判定順（Host → OAuth2 パス → 認証 → trial → upstream）は R1 要約に従う。
-/
def gate (g : Gate) (H O A T : Bool) : Response :=
  if !H then
    Response.badHost
  else if O then
    Response.outOfScope
  else if !A then
    Response.redirect
  else if needsTrial g && !T then
    Response.outOfScope
  else
    Response.upstream

/-! ## 命題1: ¬H → R400（Bool モデルでは H = false として表現） -/

theorem proposition1 (g : Gate) (H O A T : Bool) (hH : H = false) :
    R400 (gate g H O A T) = true := by
  simp [gate, hH, R400]

/-! ## 命題2: H ∧ ¬O ∧ ¬A → R302 -/

theorem proposition2 (g : Gate) (H O A T : Bool)
    (hH : H = true) (hO : O = false) (hA : A = false) :
    R302 (gate g H O A T) = true := by
  simp [gate, hH, hO, hA, R302]

/-! ## 命題3: H ∧ A ∧ ¬O → U（:18888 のみ T 必須、他は T 不要） -/

theorem proposition3_18888 (H O A T : Bool)
    (hH : H = true) (hO : O = false) (hA : A = true) (hT : T = true) :
    U (gate Gate.g18888 H O A T) = true := by
  simp [gate, needsTrial, hH, hO, hA, hT, U]

theorem proposition3_18888_requires_T (H O A T : Bool)
    (hH : H = true) (hO : O = false) (hA : A = true) (hT : T = false) :
    U (gate Gate.g18888 H O A T) = false := by
  simp [gate, needsTrial, hH, hO, hA, hT, U]

theorem proposition3_18889 (H O A T : Bool)
    (hH : H = true) (hO : O = false) (hA : A = true) :
    U (gate Gate.g18889 H O A T) = true := by
  simp [gate, needsTrial, hH, hO, hA, U]

theorem proposition3_18890 (H O A T : Bool)
    (hH : H = true) (hO : O = false) (hA : A = true) :
    U (gate Gate.g18890 H O A T) = true := by
  simp [gate, needsTrial, hH, hO, hA, U]

theorem proposition3_18891 (H O A T : Bool)
    (hH : H = true) (hO : O = false) (hA : A = true) :
    U (gate Gate.g18891 H O A T) = true := by
  simp [gate, needsTrial, hH, hO, hA, U]

theorem proposition3_nonTrial (g : Gate) (hg : g ≠ Gate.g18888) (H O A T : Bool)
    (hH : H = true) (hO : O = false) (hA : A = true) :
    U (gate g H O A T) = true := by
  cases g <;> simp [gate, needsTrial, hH, hO, hA, U] at hg ⊢

/-! ## 互斥性: R400 / R302 / U は同時に真にならない -/

theorem not_R400_and_R302 (g : Gate) (H O A T : Bool) :
    ¬ (R400 (gate g H O A T) = true ∧ R302 (gate g H O A T) = true) := by
  cases gate g H O A T <;> simp [R400, R302]

theorem not_R400_and_U (g : Gate) (H O A T : Bool) :
    ¬ (R400 (gate g H O A T) = true ∧ U (gate g H O A T) = true) := by
  cases gate g H O A T <;> simp [R400, U]

theorem not_R302_and_U (g : Gate) (H O A T : Bool) :
    ¬ (R302 (gate g H O A T) = true ∧ U (gate g H O A T) = true) := by
  cases gate g H O A T <;> simp [R302, U]

theorem response_mutually_exclusive (g : Gate) (H O A T : Bool) :
    ¬ (R400 (gate g H O A T) = true ∧
       R302 (gate g H O A T) = true ∧
       U (gate g H O A T) = true) := by
  cases gate g H O A T <;> simp [R400, R302, U]
