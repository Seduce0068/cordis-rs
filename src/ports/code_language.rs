//! Pure Rust port of `@deepseek-ai/dsh-util-code-language`.
//!
//! Source: `packages/util/code-language/src/index.ts` in
//! deepseek-ai/deepseek-harness (MIT), read as untrusted reference data.
//! Extension-to-language mapping for syntax highlighting and read hints.

/// Representative extension → canonical grammar id table (lowercase suffixes).
/// Both `/` and `\` separators are recognized; dotfiles like `.env` map too.
const LANGUAGE_EXTENSIONS: &[(&str, &str)] = &[
    ("ts", "typescript"),
    ("tsx", "typescript"),
    ("mts", "typescript"),
    ("cts", "typescript"),
    ("js", "javascript"),
    ("jsx", "javascript"),
    ("mjs", "javascript"),
    ("cjs", "javascript"),
    ("sh", "shellscript"),
    ("bash", "shellscript"),
    ("zsh", "shellscript"),
    ("fish", "fish"),
    ("json", "json"),
    ("jsonc", "json"),
    ("jsonl", "json"),
    ("ndjson", "json"),
    ("ipynb", "json"),
    ("csv", "csv"),
    ("py", "python"),
    ("pyw", "python"),
    ("pyi", "python"),
    ("rb", "ruby"),
    ("rake", "ruby"),
    ("gemspec", "ruby"),
    ("go", "go"),
    ("rs", "rust"),
    ("java", "java"),
    ("c", "c"),
    ("h", "c"),
    ("cc", "cpp"),
    ("cpp", "cpp"),
    ("cxx", "cpp"),
    ("hh", "cpp"),
    ("hpp", "cpp"),
    ("hxx", "cpp"),
    ("cs", "csharp"),
    ("kt", "kotlin"),
    ("kts", "kotlin"),
    ("swift", "swift"),
    ("php", "php"),
    ("yaml", "yaml"),
    ("yml", "yaml"),
    ("toml", "toml"),
    ("ini", "ini"),
    ("properties", "ini"),
    ("env", "dotenv"),
    ("log", "log"),
    ("diff", "diff"),
    ("http", "http"),
    ("md", "markdown"),
    ("mdx", "mdx"),
    ("rst", "rst"),
    ("tex", "latex"),
    ("bib", "bibtex"),
    ("adoc", "asciidoc"),
    ("html", "html"),
    ("css", "css"),
    ("scss", "scss"),
    ("less", "less"),
    ("sql", "sql"),
    ("tf", "hcl"),
    ("tfvars", "hcl"),
    ("gradle", "groovy"),
    ("ps1", "powershell"),
    ("bat", "bat"),
    ("cmd", "bat"),
];

/// Suffixes whose own name is the better persisted short label.
const READ_LANG_BY_EXTENSION: &[(&str, &str)] = &[
    ("tsx", "tsx"),
    ("jsx", "jsx"),
    ("tf", "tf"),
    ("tfvars", "tfvars"),
    ("gradle", "gradle"),
];

/// Canonical language → short persisted id.
const SHORT_BY_LANGUAGE: &[(&str, &str)] = &[
    ("typescript", "ts"),
    ("javascript", "js"),
    ("shellscript", "sh"),
    ("python", "py"),
    ("ruby", "rb"),
    ("rust", "rs"),
    ("csharp", "cs"),
    ("dotenv", "env"),
    ("markdown", "md"),
    ("latex", "tex"),
    ("bibtex", "bib"),
    ("asciidoc", "adoc"),
    ("powershell", "ps1"),
    ("hcl", "tf"),
    ("groovy", "gradle"),
];

fn lookup(entries: &'static [(&'static str, &'static str)], key: &str) -> Option<&'static str> {
    entries
        .iter()
        .find(|(candidate, _)| *candidate == key)
        .map(|(_, value)| *value)
}

/// Extract the lowercase extension of the final path segment. Both `/` and
/// `\` separators are recognized; a leading dot is still the separator, so
/// `.env` yields `env` while an unlisted dotfile (`.gitignore`) yields
/// `gitignore` (which simply has no language entry).
pub fn extension_for_path(path: &str) -> Option<String> {
    let slash = path.rfind(['/', '\\']).map(|index| index + 1).unwrap_or(0);
    let base = &path[slash..];
    let dot = base.rfind('.')?;
    Some(base[dot + 1..].to_ascii_lowercase())
}

/// Canonical grammar id for a path, or `None` when unrecognized.
pub fn language_for_path(path: &str) -> Option<&'static str> {
    let extension = extension_for_path(path)?;
    lookup(LANGUAGE_EXTENSIONS, &extension)
}

/// Short `lang` hint persisted in a read card, or `None` when unknown.
pub fn read_lang_hint_for_path(path: &str) -> Option<&'static str> {
    let extension = extension_for_path(path)?;
    let canonical = lookup(LANGUAGE_EXTENSIONS, &extension)?;
    Some(
        lookup(READ_LANG_BY_EXTENSION, &extension)
            .or_else(|| lookup(SHORT_BY_LANGUAGE, canonical))
            .unwrap_or(canonical),
    )
}
