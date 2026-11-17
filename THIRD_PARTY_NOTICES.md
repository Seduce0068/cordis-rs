# Third-party license notices

以下は、本リポジトリを `AGPL-3.0-or-later` に変更する前に適用されていたライセンス本文です。
上流 Cordis（MIT、Copyright (c) 2026 DeepSeek）の著作権表示・許諾通知を保持するため、
変更後もここに残します。

---

MIT License

Copyright (c) 2026 DeepSeek (upstream cordis), 2026 Seduce0068 (Rust port)

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.

---

## @deepseek-ai/dsh-fs-patch (Codex-style patch parsing)

`src/ports/fs_patch.rs` is a Rust port of `packages/fs/fs-patch` in
deepseek-ai/deepseek-harness. Per that package's LICENSE:

- DeepSeek contributions are MIT (Copyright (c) 2026 DeepSeek), reproduced
  above.
- Patch parsing and text-update logic derives from OpenAI Codex
  rust-v0.158.0 (`codex-rs/apply-patch/src/{parser,streaming_parser,
  seek_sequence,file_update}.rs`), Copyright 2025 OpenAI, licensed under the
  Apache License, Version 2.0. Those portions remain under Apache-2.0.

Modifications in this Rust port: std-only implementation, `Result`-based
errors, and ownership by `cordis-rs` (AGPL-3.0-or-later). Apache-2.0 is
GPLv3-compatible, so inclusion in this AGPL project is permitted while the
attribution above is retained.

The full Apache License 2.0 text is available at
<http://www.apache.org/licenses/LICENSE-2.0>.
