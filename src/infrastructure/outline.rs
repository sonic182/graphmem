use std::{collections::HashSet, path::Path, sync::OnceLock};

use ast_grep_core::{
    AstGrep, Doc, Language, Node,
    matcher::{Pattern, PatternBuilder, PatternError},
    tree_sitter::{LanguageExt, StrDoc, TSLanguage},
};
use ast_grep_language::SupportLang;
use ast_grep_outline::{
    DEFAULT_OUTLINE_RULES,
    combined_extractor::CombinedExtractors,
    extractor::parse_outline_rules,
    model::{OutlineEntry, SymbolType},
};
#[cfg(windows)]
use tree_sitter_scss_windows as tree_sitter_scss;

use crate::domain::{CodeSymbol, Coverage, SourcePoint, nest_symbols};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodeLanguage {
    Bash,
    C,
    Cpp,
    Css,
    Eex,
    Elixir,
    Go,
    Heex,
    Html,
    JavaScript,
    Php,
    Python,
    Ruby,
    Rust,
    Scss,
    Sql,
    Tsx,
    TypeScript,
    Zig,
}

const LANGUAGE_NAMES: [(CodeLanguage, &str); 19] = [
    (CodeLanguage::Bash, "bash"),
    (CodeLanguage::C, "c"),
    (CodeLanguage::Cpp, "cpp"),
    (CodeLanguage::Css, "css"),
    (CodeLanguage::Eex, "eex"),
    (CodeLanguage::Elixir, "elixir"),
    (CodeLanguage::Go, "go"),
    (CodeLanguage::Heex, "heex"),
    (CodeLanguage::Html, "html"),
    (CodeLanguage::JavaScript, "javascript"),
    (CodeLanguage::Php, "php"),
    (CodeLanguage::Python, "python"),
    (CodeLanguage::Ruby, "ruby"),
    (CodeLanguage::Rust, "rust"),
    (CodeLanguage::Scss, "scss"),
    (CodeLanguage::Sql, "sql"),
    (CodeLanguage::Tsx, "tsx"),
    (CodeLanguage::TypeScript, "typescript"),
    (CodeLanguage::Zig, "zig"),
];

impl CodeLanguage {
    /// Routes by the last extension, so `.html.heex` is HEEx and `.html.eex`
    /// is EEx rather than HTML. `.h` headers go to C++, whose grammar also
    /// accepts nearly all C.
    pub fn for_path(path: &Path) -> Option<Self> {
        let name = path.file_name()?.to_str()?;
        if matches!(name, "Gemfile" | "Rakefile") {
            return Some(Self::Ruby);
        }
        let (_, extension) = name.rsplit_once('.')?;
        Some(match extension {
            "sh" | "bash" => Self::Bash,
            "c" => Self::C,
            "h" | "cc" | "cpp" | "cxx" | "c++" | "hh" | "hpp" | "hxx" | "h++" | "ipp" | "tpp" => {
                Self::Cpp
            }
            "css" => Self::Css,
            "eex" | "leex" => Self::Eex,
            "ex" | "exs" => Self::Elixir,
            "go" => Self::Go,
            "heex" => Self::Heex,
            "html" | "htm" => Self::Html,
            "js" | "mjs" | "cjs" | "jsx" => Self::JavaScript,
            "php" => Self::Php,
            "py" | "pyi" => Self::Python,
            "rb" | "rake" | "gemspec" => Self::Ruby,
            "rs" => Self::Rust,
            "scss" => Self::Scss,
            "sql" => Self::Sql,
            "tsx" => Self::Tsx,
            "ts" | "mts" | "cts" => Self::TypeScript,
            "zig" | "zon" => Self::Zig,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        LANGUAGE_NAMES
            .iter()
            .find(|(language, _)| *language == self)
            .map(|(_, name)| *name)
            .expect("every language has a name")
    }

    pub fn from_name(name: &str) -> Option<Self> {
        LANGUAGE_NAMES
            .iter()
            .find(|(_, candidate)| *candidate == name)
            .map(|(language, _)| *language)
    }
}

pub struct Outline {
    pub symbols: Vec<CodeSymbol>,
    pub coverage: Coverage,
}

pub fn outline(language: CodeLanguage, source: &str) -> Outline {
    let mut symbols = Vec::new();
    let origin = Origin::default();
    let errors = match language {
        CodeLanguage::Bash => bash(source, &mut symbols),
        CodeLanguage::C => bundled(SupportLang::C, source, origin, &mut symbols),
        CodeLanguage::Cpp => {
            cpp_namespaces(source, &mut symbols);
            bundled(SupportLang::Cpp, source, origin, &mut symbols)
        }
        CodeLanguage::Css => styles(
            SupportLang::Css.ast_grep(source).root(),
            origin,
            &mut symbols,
        ),
        CodeLanguage::Eex => {
            eex(source, &mut symbols);
            false
        }
        CodeLanguage::Elixir => elixir(source, &mut symbols),
        CodeLanguage::Go => bundled(SupportLang::Go, source, origin, &mut symbols),
        CodeLanguage::Heex => heex(source, origin, &mut symbols),
        CodeLanguage::Html => html(source, &mut symbols),
        CodeLanguage::JavaScript => {
            javascript(SupportLang::JavaScript, source, origin, &mut symbols)
        }
        CodeLanguage::Php => bundled(SupportLang::Php, source, origin, &mut symbols),
        CodeLanguage::Python => bundled(SupportLang::Python, source, origin, &mut symbols),
        CodeLanguage::Ruby => bundled(SupportLang::Ruby, source, origin, &mut symbols),
        CodeLanguage::Rust => bundled(SupportLang::Rust, source, origin, &mut symbols),
        CodeLanguage::Scss => styles(Scss.ast_grep(source).root(), origin, &mut symbols),
        CodeLanguage::Sql => sql(source, &mut symbols),
        CodeLanguage::Tsx => javascript(SupportLang::Tsx, source, origin, &mut symbols),
        CodeLanguage::TypeScript => {
            javascript(SupportLang::TypeScript, source, origin, &mut symbols)
        }
        CodeLanguage::Zig => zig(source, &mut symbols),
    };
    nest_symbols(&mut symbols);
    if language == CodeLanguage::Elixir {
        qualify_nested_modules(&mut symbols);
    }
    let coverage = if language == CodeLanguage::Eex {
        Coverage::Partial("EEx directives only".to_owned())
    } else if errors {
        Coverage::Partial("syntax errors".to_owned())
    } else {
        Coverage::Complete
    };
    Outline { symbols, coverage }
}

/// Zero-based position of an embedded document inside its file.
#[derive(Debug, Clone, Copy, Default)]
struct Origin {
    line: usize,
    column: usize,
}

impl Origin {
    fn point(self, line: usize, column: usize) -> SourcePoint {
        SourcePoint {
            line: self.line + line + 1,
            column: if line == 0 { self.column } else { 0 } + column + 1,
        }
    }

    fn inside<D: Doc>(self, node: &Node<'_, D>) -> Self {
        let start = node.start_pos();
        Self {
            line: self.line + start.line(),
            column: if start.line() == 0 { self.column } else { 0 } + start.column(node),
        }
    }

    fn after<D: Doc>(self, node: &Node<'_, D>) -> Self {
        let end = node.end_pos();
        Self {
            line: self.line + end.line(),
            column: if end.line() == 0 { self.column } else { 0 } + end.column(node),
        }
    }
}

const BUNDLED: [SupportLang; 10] = [
    SupportLang::C,
    SupportLang::Cpp,
    SupportLang::Go,
    SupportLang::JavaScript,
    SupportLang::Php,
    SupportLang::Python,
    SupportLang::Ruby,
    SupportLang::Rust,
    SupportLang::Tsx,
    SupportLang::TypeScript,
];

/// Only rules for compiled-in grammars are compiled: ast-grep panics when it
/// builds a matcher for a grammar whose feature is off.
fn extractors(language: SupportLang) -> &'static CombinedExtractors<SupportLang> {
    static EXTRACTORS: OnceLock<Vec<(SupportLang, CombinedExtractors<SupportLang>)>> =
        OnceLock::new();
    EXTRACTORS
        .get_or_init(|| {
            let mut grouped = BUNDLED.map(|language| (language, Vec::new()));
            for rule in parse_outline_rules::<SupportLang>(DEFAULT_OUTLINE_RULES)
                .expect("bundled outline rules deserialize")
            {
                if let Some((_, rules)) = grouped
                    .iter_mut()
                    .find(|(language, _)| *language == rule.common().language)
                {
                    rules.push(rule);
                }
            }
            grouped
                .into_iter()
                .map(|(language, rules)| {
                    let extractors = CombinedExtractors::try_from(rules, &Default::default())
                        .expect("bundled outline rules compile");
                    (language, extractors)
                })
                .collect()
        })
        .iter()
        .find(|(candidate, _)| *candidate == language)
        .map(|(_, extractors)| extractors)
        .expect("bundled language has extractors")
}

fn bundled(
    language: SupportLang,
    source: &str,
    origin: Origin,
    symbols: &mut Vec<CodeSymbol>,
) -> bool {
    let grep = language.ast_grep(source);
    let root = grep.root();
    let mut found = Vec::new();
    bundled_items(
        extractors(language),
        &grep,
        root.clone(),
        origin,
        &mut found,
    );
    // A nested item is found both as its parent's member and as an item.
    let mut seen = HashSet::new();
    symbols.extend(
        found
            .into_iter()
            .filter(|symbol| seen.insert((symbol.name.clone(), symbol.end))),
    );
    has_errors(&root)
}

/// ast-grep outlines stop at item -> member, so `module A; class B; def c`
/// would lose `c`; outlining each item's children again recovers nesting.
fn bundled_items<'r>(
    extractors: &CombinedExtractors<SupportLang>,
    grep: &'r AstGrep<StrDoc<SupportLang>>,
    node: Node<'r, StrDoc<SupportLang>>,
    origin: Origin,
    symbols: &mut Vec<CodeSymbol>,
) {
    for item in extractors.extract(node.clone()) {
        let kind = if item.is_import {
            "import".to_owned()
        } else if item.entry.ast_kind == "impl_item" {
            "impl".to_owned()
        } else {
            symbol_kind(item.entry.symbol_type)
        };
        symbols.push(entry_symbol(&item.entry, kind, origin));
        for member in &item.members {
            let kind = symbol_kind(member.entry.symbol_type);
            symbols.push(entry_symbol(&member.entry, kind, origin));
        }
        let range = &item.entry.range.byte_offset;
        let Some(item_node) = node
            .get_inner_node()
            .descendant_for_byte_range(range.start, range.end)
        else {
            continue;
        };
        for child in grep.adopt(item_node).children() {
            bundled_items(extractors, grep, child, origin, symbols);
        }
    }
}

fn symbol_kind(symbol_type: SymbolType) -> String {
    serde_json::to_value(symbol_type)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_default()
}

fn entry_symbol(entry: &OutlineEntry<'_>, kind: String, origin: Origin) -> CodeSymbol {
    let range = &entry.range;
    CodeSymbol {
        name: symbol_name(&entry.name),
        kind,
        parent: None,
        start: origin.point(range.start.line, range.start.column),
        end: origin.point(range.end.line, range.end.column),
        signature: signature(&entry.signature),
    }
}

fn node_symbol<D: Doc>(node: &Node<'_, D>, name: String, kind: &str, origin: Origin) -> CodeSymbol {
    let start = node.start_pos();
    let end = node.end_pos();
    CodeSymbol {
        name: symbol_name(&name),
        kind: kind.to_owned(),
        parent: None,
        start: origin.point(start.line(), start.column(node)),
        end: origin.point(end.line(), end.column(node)),
        signature: signature(&node.text()),
    }
}

/// A multi-line name such as `use a::{b, c}` becomes one line.
fn symbol_name(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(160)
        .collect()
}

fn signature(text: &str) -> String {
    let line = text.lines().next().unwrap_or_default().trim();
    line.chars().take(160).collect()
}

fn has_errors<D: Doc>(root: &Node<'_, D>) -> bool {
    root.dfs().any(|node| node.is_error() || node.is_missing())
}

macro_rules! grammar {
    ($name:ident, $language:expr) => {
        #[derive(Clone, Copy)]
        struct $name;

        impl Language for $name {
            fn kind_to_id(&self, kind: &str) -> u16 {
                self.get_ts_language().id_for_node_kind(kind, true)
            }

            fn field_to_id(&self, field: &str) -> Option<u16> {
                self.get_ts_language()
                    .field_id_for_name(field)
                    .map(|field| field.get())
            }

            fn build_pattern(&self, builder: &PatternBuilder) -> Result<Pattern, PatternError> {
                builder.build(|source| StrDoc::try_new(source, *self))
            }
        }

        impl LanguageExt for $name {
            fn get_ts_language(&self) -> TSLanguage {
                $language.into()
            }
        }
    };
}

grammar!(Heex, tree_sitter_heex::LANGUAGE);
grammar!(Scss, tree_sitter_scss::language());
grammar!(Sql, tree_sitter_sequel::LANGUAGE);
grammar!(Zig, tree_sitter_zig::LANGUAGE);

fn elixir(source: &str, symbols: &mut Vec<CodeSymbol>) -> bool {
    let grep = SupportLang::Elixir.ast_grep(source);
    let root = grep.root();
    let mut errors = has_errors(&root);
    let mut last_definition: Option<usize> = None;
    for node in root.dfs() {
        if let Some(content) = heex_sigil(&node) {
            errors |= heex(&content.text(), Origin::default().inside(&content), symbols);
            continue;
        }
        let Some((name, kind)) = elixir_definition(&node) else {
            continue;
        };
        let symbol = node_symbol(&node, name, kind, Origin::default());
        // Consecutive clauses of one function become one symbol.
        if let Some(previous) = last_definition.map(|index| &mut symbols[index])
            && matches!(kind, "function" | "macro" | "guard")
            && previous.kind == symbol.kind
            && previous.name == symbol.name
        {
            previous.end = symbol.end;
            continue;
        }
        last_definition = Some(symbols.len());
        symbols.push(symbol);
    }
    errors
}

fn elixir_definition<D: Doc>(node: &Node<'_, D>) -> Option<(String, &'static str)> {
    if node.kind() != "call" {
        return None;
    }
    let target = node.field("target")?;
    if target.kind() != "identifier" {
        return None;
    }
    let kind = match target.text().as_ref() {
        "defmodule" | "defprotocol" => "module",
        "defimpl" => "impl",
        "def" | "defp" | "defdelegate" => "function",
        "defmacro" | "defmacrop" => "macro",
        "defguard" | "defguardp" => "guard",
        "alias" | "import" | "require" | "use" => "import",
        _ => return None,
    };
    let arguments = node.children().find(|child| child.kind() == "arguments")?;
    let first = arguments.children().find(|child| child.is_named())?;
    let name = match kind {
        "module" | "impl" | "import" => first.text().into_owned(),
        _ => elixir_function_name(&first)?,
    };
    Some((name, kind))
}

fn qualify_nested_modules(symbols: &mut [CodeSymbol]) {
    for index in 0..symbols.len() {
        let Some(parent) = symbols[index].parent else {
            continue;
        };
        if symbols[index].kind == "module" && symbols[parent].kind == "module" {
            symbols[index].name = format!("{}.{}", symbols[parent].name, symbols[index].name);
        }
    }
}

/// `name/arity` for `name(args)`, `name(args) when guard`, or bare `name`.
fn elixir_function_name<D: Doc>(head: &Node<'_, D>) -> Option<String> {
    let head = if head.kind() == "binary_operator"
        && head
            .field("operator")
            .is_some_and(|operator| operator.text() == "when")
    {
        head.field("left")?
    } else {
        head.clone()
    };
    let (name, arity) = match head.kind().as_ref() {
        "call" => (
            head.field("target")?.text().into_owned(),
            head.children()
                .find(|child| child.kind() == "arguments")
                .map_or(0, |arguments| {
                    arguments
                        .children()
                        .filter(|child| child.is_named())
                        .count()
                }),
        ),
        "identifier" => (head.text().into_owned(), 0),
        _ => return None,
    };
    Some(format!("{name}/{arity}"))
}

fn heex_sigil<'r, D: Doc>(node: &Node<'r, D>) -> Option<Node<'r, D>> {
    if node.kind() != "sigil" {
        return None;
    }
    node.children()
        .find(|child| child.kind() == "sigil_name")
        .filter(|name| name.text() == "H")?;
    node.children()
        .find(|child| child.kind() == "quoted_content")
}

fn heex(source: &str, origin: Origin, symbols: &mut Vec<CodeSymbol>) -> bool {
    let grep = Heex.ast_grep(source);
    let root = grep.root();
    for node in root.dfs() {
        let kind = match node.kind().as_ref() {
            "component" | "self_closing_component" => "component",
            "slot" | "self_closing_slot" => "slot",
            _ => continue,
        };
        // A self-closing tag can be wrapped in a same-range `component` node.
        if node.children().any(|child| {
            matches!(
                child.kind().as_ref(),
                "self_closing_component" | "self_closing_slot"
            )
        }) {
            continue;
        }
        let Some(name) = node
            .dfs()
            .find(|child| matches!(child.kind().as_ref(), "component_name" | "slot_name"))
        else {
            continue;
        };
        let name = if kind == "slot" {
            format!(":{}", name.text())
        } else {
            name.text().into_owned()
        };
        symbols.push(node_symbol(&node, name, kind, origin));
    }
    let mut errors = has_errors(&root);
    for tag in root.dfs().filter(|node| node.kind() == "tag") {
        let (Some(start), Some(end)) = (
            tag.children().find(|child| child.kind() == "start_tag"),
            tag.children().find(|child| child.kind() == "end_tag"),
        ) else {
            continue;
        };
        let Some(name) = start.children().find(|child| child.kind() == "tag_name") else {
            continue;
        };
        let content = &source[start.range().end..end.range().start];
        let origin = origin.after(&start);
        errors |= match name.text().as_ref() {
            "script" => javascript(SupportLang::JavaScript, content, origin, symbols),
            "style" => styles(SupportLang::Css.ast_grep(content).root(), origin, symbols),
            _ => false,
        };
    }
    errors
}

/// SCSS extends the CSS grammar, so one walk covers both.
fn styles<D: Doc>(root: Node<'_, D>, origin: Origin, symbols: &mut Vec<CodeSymbol>) -> bool {
    for node in root.dfs() {
        let child = |kind: &str| node.children().find(|child| child.kind() == kind);
        let node_kind = node.kind();
        let symbol = match node_kind.as_ref() {
            "rule_set" => child("selectors").map(|name| (name.text().into_owned(), "selector")),
            "keyframes_statement" => {
                child("keyframes_name").map(|name| (name.text().into_owned(), "keyframes"))
            }
            kind @ ("media_statement" | "supports_statement") => child("block").map(|block| {
                let text = node.text();
                let name = text[..block.range().start - node.range().start]
                    .trim()
                    .to_owned();
                (name, kind.trim_end_matches("_statement"))
            }),
            kind @ ("mixin_statement" | "function_statement") => node.field("name").map(|name| {
                (
                    name.text().into_owned(),
                    kind.trim_end_matches("_statement"),
                )
            }),
            "declaration" => node
                .children()
                .next()
                .filter(|name| {
                    name.kind() == "property_name"
                        && (name.text().starts_with("--") || name.text().starts_with('$'))
                })
                .map(|name| (name.text().into_owned(), "variable")),
            "import_statement" | "use_statement" | "forward_statement" => {
                let name = child("string_value").map_or_else(
                    || node.text().trim_end_matches(';').to_owned(),
                    |path| path.text().trim_matches(['"', '\'']).to_owned(),
                );
                Some((name, "import"))
            }
            _ => None,
        };
        if let Some((name, kind)) = symbol {
            symbols.push(node_symbol(&node, name, kind, origin));
        }
    }
    has_errors(&root)
}

fn sql(source: &str, symbols: &mut Vec<CodeSymbol>) -> bool {
    let grep = Sql.ast_grep(source);
    let root = grep.root();
    for node in root.dfs() {
        let node_kind = node.kind();
        let Some(kind) = node_kind.strip_prefix("create_") else {
            continue;
        };
        let Some(name) = node
            .children()
            .find(|child| matches!(child.kind().as_ref(), "object_reference" | "identifier"))
        else {
            continue;
        };
        symbols.push(node_symbol(
            &node,
            name.text().into_owned(),
            kind,
            Origin::default(),
        ));
    }
    root.dfs().any(|node| {
        node.kind() == "keyword_create"
            && !node
                .parent()
                .is_some_and(|parent| parent.kind().starts_with("create_"))
    })
}

/// Zig containers are values (`const Point = struct { ... }`), so a
/// declaration is a symbol only when its value is a container or an import;
/// other constants and variables are left out.
fn zig(source: &str, symbols: &mut Vec<CodeSymbol>) -> bool {
    let grep = Zig.ast_grep(source);
    let root = grep.root();
    for node in root.dfs() {
        let symbol = match node.kind().as_ref() {
            "function_declaration" => node
                .field("name")
                .map(|name| (name.text().into_owned(), "function")),
            "container_field" => node
                .field("name")
                .map(|name| (name.text().into_owned(), "field")),
            "test_declaration" => node
                .children()
                .find(|child| matches!(child.kind().as_ref(), "string" | "identifier"))
                .map(|name| (name.text().trim_matches('"').to_owned(), "test")),
            "variable_declaration" => zig_declaration(&node),
            _ => None,
        };
        if let Some((name, kind)) = symbol {
            symbols.push(node_symbol(&node, name, kind, Origin::default()));
        }
    }
    has_errors(&root)
}

fn zig_declaration<D: Doc>(node: &Node<'_, D>) -> Option<(String, &'static str)> {
    let name = node.children().find(|child| child.kind() == "identifier")?;
    let kind = node
        .children()
        .find_map(|child| match child.kind().as_ref() {
            "struct_declaration" => Some("struct"),
            "enum_declaration" => Some("enum"),
            "union_declaration" => Some("union"),
            "opaque_declaration" => Some("opaque"),
            "error_set_declaration" => Some("error_set"),
            "builtin_function" if child.text().starts_with("@import") => Some("import"),
            _ => None,
        })?;
    Some((name.text().into_owned(), kind))
}

fn javascript(
    language: SupportLang,
    source: &str,
    origin: Origin,
    symbols: &mut Vec<CodeSymbol>,
) -> bool {
    let errors = bundled(language, source, origin, symbols);
    if !source.contains("export default") {
        return errors;
    }
    let grep = language.ast_grep(source);
    for export in grep.root().children() {
        let Some(object) = export
            .field("value")
            .filter(|_| export.kind() == "export_statement")
            .filter(|value| value.kind() == "object")
        else {
            continue;
        };
        symbols.push(node_symbol(&export, "default".to_owned(), "object", origin));
        for member in object.children() {
            let name = match member.kind().as_ref() {
                "method_definition" => member.field("name"),
                "pair" => member
                    .field("value")
                    .filter(|value| {
                        matches!(
                            value.kind().as_ref(),
                            "function_expression" | "function" | "arrow_function"
                        )
                    })
                    .and(member.field("key")),
                _ => None,
            };
            if let Some(name) = name {
                symbols.push(node_symbol(
                    &member,
                    name.text().into_owned(),
                    "method",
                    origin,
                ));
            }
        }
    }
    errors
}

/// The bundled C++ rules skip namespaces, which leaves namespaced functions
/// without a parent.
fn cpp_namespaces(source: &str, symbols: &mut Vec<CodeSymbol>) {
    let grep = SupportLang::Cpp.ast_grep(source);
    for node in grep.root().dfs() {
        if node.kind() != "namespace_definition" {
            continue;
        }
        let name = node
            .field("name")
            .map_or_else(|| "(anonymous)".to_owned(), |name| name.text().into_owned());
        symbols.push(node_symbol(&node, name, "namespace", Origin::default()));
    }
}

fn bash(source: &str, symbols: &mut Vec<CodeSymbol>) -> bool {
    let grep = SupportLang::Bash.ast_grep(source);
    let root = grep.root();
    for node in root.dfs() {
        if node.kind() != "function_definition" {
            continue;
        }
        if let Some(name) = node.field("name") {
            symbols.push(node_symbol(
                &node,
                name.text().into_owned(),
                "function",
                Origin::default(),
            ));
        }
    }
    has_errors(&root)
}

/// Outlines the JavaScript inside `<script>` and the CSS inside `<style>`
/// elements; markup itself yields no symbols.
fn html(source: &str, symbols: &mut Vec<CodeSymbol>) -> bool {
    let grep = SupportLang::Html.ast_grep(source);
    let root = grep.root();
    let mut errors = has_errors(&root);
    for element in root.dfs() {
        let kind = element.kind();
        if !matches!(kind.as_ref(), "script_element" | "style_element") {
            continue;
        }
        let Some(content) = element.children().find(|child| child.kind() == "raw_text") else {
            continue;
        };
        let origin = Origin::default().inside(&content);
        let text = content.text();
        errors |= if kind == "script_element" {
            javascript(SupportLang::JavaScript, &text, origin, symbols)
        } else {
            styles(SupportLang::Css.ast_grep(&text).root(), origin, symbols)
        };
    }
    errors
}

/// tree-sitter-eex is not usable from Rust, and its grammar is only `<% %>`
/// directives around text, so a scan finds the same directives.
fn eex(source: &str, symbols: &mut Vec<CodeSymbol>) {
    let lines = source
        .match_indices('\n')
        .map(|(offset, _)| offset + 1)
        .collect::<Vec<_>>();
    let point = |offset: usize| {
        let line = lines.partition_point(|&start| start <= offset);
        let line_start = if line == 0 { 0 } else { lines[line - 1] };
        SourcePoint {
            line: line + 1,
            column: source[line_start..offset].chars().count() + 1,
        }
    };
    let mut cursor = 0;
    while let Some(found) = source[cursor..].find("<%") {
        let start = cursor + found;
        let Some(length) = source[start..].find("%>") else {
            break;
        };
        let end = start + length + 2;
        cursor = end;
        let tag = &source[start..end];
        if tag.starts_with("<%#") || tag.starts_with("<%%") || tag.starts_with("<%!--") {
            continue;
        }
        let code = tag
            .trim_start_matches("<%")
            .trim_start_matches('=')
            .trim_end_matches("%>")
            .trim();
        if code.is_empty() || code == "end" {
            continue;
        }
        symbols.push(CodeSymbol {
            name: signature(code),
            kind: "expression".to_owned(),
            parent: None,
            start: point(start),
            end: point(end),
            signature: signature(tag),
        });
    }
}
