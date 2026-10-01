#[derive(Debug, Clone, PartialEq)]
pub struct Memory {
    pub id: i64,
    pub content: String,
    pub memory_type: String,
    pub importance: f64,
    pub created_at: i64,
    pub updated_at: i64,
    pub last_accessed_at: Option<i64>,
    pub access_count: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SearchResult {
    pub memory: Memory,
    pub score: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Scope {
    pub id: i64,
    pub name: String,
    pub created_at: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Entity {
    pub id: i64,
    pub kind: String,
    pub name: String,
    pub canonical_name: String,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Edge {
    pub id: i64,
    pub source_id: i64,
    pub relation: String,
    pub target_id: i64,
    pub created_at: i64,
    pub metadata: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntityReference {
    pub kind: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Relation {
    pub source: EntityReference,
    pub relation: String,
    pub target: EntityReference,
    pub metadata: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GraphDirection {
    Incoming,
    Outgoing,
    Both,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GraphHop {
    pub entity: Entity,
    pub edge: Edge,
    pub direction: GraphDirection,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GraphPath {
    pub hops: Vec<GraphHop>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StoreStats {
    pub memories: i64,
    pub scopes: i64,
    pub entities: i64,
    pub edges: i64,
}

pub fn text_mentions(haystack: &str, needle: &str) -> bool {
    let needle = needle.trim();
    !needle.is_empty() && haystack.to_lowercase().contains(&needle.to_lowercase())
}

pub fn personalized_pagerank(
    adjacency: &[Vec<usize>],
    seeds: &[(usize, f64)],
    damping: f64,
    iterations: usize,
) -> Vec<f64> {
    let mut restart = vec![0.0; adjacency.len()];
    for &(node, weight) in seeds {
        if node < restart.len() && weight.is_finite() && weight > 0.0 {
            restart[node] += weight;
        }
    }
    let total = restart.iter().sum::<f64>();
    if total == 0.0 {
        return restart;
    }
    for weight in &mut restart {
        *weight /= total;
    }

    let mut rank = restart.clone();
    for _ in 0..iterations {
        let mut next = restart
            .iter()
            .map(|weight| weight * (1.0 - damping))
            .collect::<Vec<_>>();
        let mut dangling = 0.0;
        for (node, neighbors) in adjacency.iter().enumerate() {
            if neighbors.is_empty() {
                dangling += rank[node];
            } else {
                let share = rank[node] * damping / neighbors.len() as f64;
                for &neighbor in neighbors {
                    if neighbor < next.len() {
                        next[neighbor] += share;
                    }
                }
            }
        }
        for (target, weight) in restart.iter().enumerate() {
            next[target] += dangling * damping * weight;
        }
        let delta = rank
            .iter()
            .zip(&next)
            .map(|(previous, current)| (previous - current).abs())
            .sum::<f64>();
        rank = next;
        if delta < 1e-9 {
            break;
        }
    }
    rank
}

/// Text embedded for an entity. Changing it requires bumping
/// `DOCUMENT_FORMAT` in `application.rs`.
pub fn entity_document(entity: &Entity) -> String {
    format!("{} {}", entity.kind, entity.name)
}

/// Text embedded for an edge; same versioning rule as `entity_document`.
pub fn edge_document(edge: &Edge, source: &Entity, target: &Entity) -> String {
    format!(
        "{} {} {}",
        source.name,
        edge.relation.replace('_', " "),
        target.name
    )
}

/// One-based line and character column.
#[cfg(feature = "code")]
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SourcePoint {
    pub line: usize,
    pub column: usize,
}

#[cfg(feature = "code")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CodeSymbol {
    pub name: String,
    pub kind: String,
    /// Index of the syntactically enclosing symbol in the same outline.
    pub parent: Option<usize>,
    pub start: SourcePoint,
    pub end: SourcePoint,
    pub signature: String,
}

#[cfg(feature = "code")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Coverage {
    Complete,
    /// Symbols were extracted, but some of the file may be missing from them.
    Partial(String),
    Skipped(String),
}

#[cfg(feature = "code")]
impl Coverage {
    pub fn as_text(&self) -> String {
        match self {
            Self::Complete => "complete".to_owned(),
            Self::Partial(reason) => format!("partial: {reason}"),
            Self::Skipped(reason) => format!("skipped: {reason}"),
        }
    }

    pub fn from_text(text: &str) -> Self {
        if let Some(reason) = text.strip_prefix("partial: ") {
            Self::Partial(reason.to_owned())
        } else if let Some(reason) = text.strip_prefix("skipped: ") {
            Self::Skipped(reason.to_owned())
        } else {
            Self::Complete
        }
    }
}

#[cfg(feature = "code")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Freshness {
    Fresh,
    Stale,
    Missing,
}

#[cfg(feature = "code")]
impl Freshness {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Fresh => "fresh",
            Self::Stale => "stale",
            Self::Missing => "missing",
        }
    }
}

/// Orders symbols by position and points each at the innermost symbol whose
/// range contains it.
#[cfg(feature = "code")]
pub fn nest_symbols(symbols: &mut [CodeSymbol]) {
    symbols.sort_by(|a, b| a.start.cmp(&b.start).then(b.end.cmp(&a.end)));
    let mut open: Vec<usize> = Vec::new();
    for index in 0..symbols.len() {
        while open
            .last()
            .is_some_and(|&top| symbols[top].end < symbols[index].end)
        {
            open.pop();
        }
        symbols[index].parent = open.last().copied();
        open.push(index);
    }
}

#[cfg(feature = "code")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolChangeKind {
    Added,
    Removed,
    Modified,
}

#[cfg(feature = "code")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SymbolChange {
    pub change: SymbolChangeKind,
    pub symbol: CodeSymbol,
}

#[cfg(feature = "code")]
pub fn diff_symbols(
    base_source: &str,
    base: &[CodeSymbol],
    head_source: &str,
    head: &[CodeSymbol],
) -> Vec<SymbolChange> {
    use std::collections::{HashMap, HashSet};

    let base_keys = symbol_keys(base);
    let head_keys = symbol_keys(head);
    let base_index = base_keys
        .iter()
        .enumerate()
        .map(|(index, key)| (key, index))
        .collect::<HashMap<_, _>>();
    let head_set = head_keys.iter().collect::<HashSet<_>>();
    let base_text = SourceText::new(base_source);
    let head_text = SourceText::new(head_source);
    let change = |change, symbols: &[CodeSymbol], index: usize| SymbolChange {
        change,
        symbol: symbols[index].clone(),
    };

    let mut changes = Vec::new();
    for (index, key) in head_keys.iter().enumerate() {
        match base_index.get(key) {
            None => changes.push(change(SymbolChangeKind::Added, head, index)),
            Some(&old)
                if base[old].signature != head[index].signature
                    || base_text.own_text(base, old) != head_text.own_text(head, index) =>
            {
                changes.push(change(SymbolChangeKind::Modified, head, index));
            }
            Some(_) => {}
        }
    }
    for (index, key) in base_keys.iter().enumerate() {
        if !head_set.contains(key) {
            changes.push(change(SymbolChangeKind::Removed, base, index));
        }
    }
    changes
}

#[cfg(feature = "code")]
fn symbol_keys(symbols: &[CodeSymbol]) -> Vec<(String, usize)> {
    let mut seen = std::collections::HashMap::<String, usize>::new();
    symbols
        .iter()
        .map(|symbol| {
            let mut key = format!("{}\0{}", symbol.kind, symbol.name);
            for parent in std::iter::successors(symbol.parent, |&parent| symbols[parent].parent) {
                key.push('\0');
                key.push_str(&symbols[parent].name);
            }
            let occurrence = seen.entry(key.clone()).or_default();
            *occurrence += 1;
            (key, *occurrence)
        })
        .collect()
}

#[cfg(feature = "code")]
struct SourceText<'a> {
    source: &'a str,
    lines: Vec<&'a str>,
    line_starts: Vec<usize>,
}

#[cfg(feature = "code")]
impl<'a> SourceText<'a> {
    fn new(source: &'a str) -> Self {
        Self {
            source,
            lines: source.lines().collect(),
            line_starts: std::iter::once(0)
                .chain(source.match_indices('\n').map(|(offset, _)| offset + 1))
                .collect(),
        }
    }

    fn offset(&self, point: SourcePoint) -> usize {
        let Some(&start) = self.line_starts.get(point.line.saturating_sub(1)) else {
            return self.source.len();
        };
        self.source[start..]
            .char_indices()
            .nth(point.column.saturating_sub(1))
            .map_or(self.source.len(), |(offset, _)| start + offset)
    }

    fn own_text(&self, symbols: &[CodeSymbol], index: usize) -> String {
        let span = self.span(symbols, index);
        let mut text = String::new();
        let mut cursor = span.start;
        for child in (0..symbols.len()).filter(|&child| symbols[child].parent == Some(index)) {
            let child = self.span(symbols, child);
            let start = child.start.clamp(cursor, span.end);
            text.push_str(&self.source[cursor..start]);
            cursor = child.end.clamp(cursor, span.end);
        }
        text.push_str(&self.source[cursor..span.end]);
        text.retain(|character| !matches!(character, ',' | ';'));
        text.lines()
            .filter_map(|line| {
                let indent = line.len() - line.trim_start().len();
                let words = line.split_whitespace().collect::<Vec<_>>();
                (!words.is_empty()).then(|| format!("{}{}", &line[..indent], words.join(" ")))
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn span(&self, symbols: &[CodeSymbol], index: usize) -> std::ops::Range<usize> {
        let symbol = &symbols[index];
        let ancestors = std::iter::successors(symbol.parent, |&parent| symbols[parent].parent)
            .collect::<Vec<_>>();
        let taken = |line: usize| {
            ancestors
                .first()
                .is_some_and(|&parent| symbols[parent].start.line >= line)
                || symbols.iter().enumerate().any(|(other, candidate)| {
                    other != index
                        && !ancestors.contains(&other)
                        && (candidate.start.line..=candidate.end.line).contains(&line)
                })
        };
        let start = self.offset(symbol.start);
        let line_start = self
            .line_starts
            .get(symbol.start.line.saturating_sub(1))
            .copied()
            .unwrap_or(start)
            .min(start);
        let mut lead = symbol.start.line;
        if self.source[line_start..start].trim().is_empty() {
            let mut line = lead;
            let mut depth = 0_isize;
            let mut heredoc = false;
            while line > 1 && !taken(line - 1) {
                let Some(text) = self.lines.get(line - 2).map(|text| text.trim()) else {
                    break;
                };
                if text.is_empty() && !heredoc {
                    break;
                }
                line -= 1;
                if text.matches("\"\"\"").count() % 2 == 1 {
                    heredoc = !heredoc;
                }
                if heredoc {
                    continue;
                }
                let comment = ["//", "/*", "*", "--", "<!--"]
                    .iter()
                    .any(|marker| text.starts_with(marker))
                    || (text.starts_with('#') && !text.starts_with("#["));
                if !comment {
                    depth += text.matches([')', ']', '}']).count().cast_signed()
                        - text.matches(['(', '[', '{']).count().cast_signed();
                }
                if depth > 0 {
                    continue;
                }
                if !comment && !text.starts_with('#') && !text.starts_with('@') {
                    break;
                }
                depth = 0;
                lead = line;
            }
        }
        let start = if lead < symbol.start.line {
            self.line_starts[lead - 1]
        } else {
            start
        };
        let end = self.offset(symbol.end).max(start);
        start..end
    }
}

#[cfg(test)]
mod tests {
    use super::{personalized_pagerank, text_mentions};

    #[cfg(feature = "code")]
    #[test]
    fn nest_symbols_points_each_symbol_at_its_innermost_container() {
        use super::{CodeSymbol, SourcePoint, nest_symbols};
        let symbol = |name: &str, start: usize, end: usize| CodeSymbol {
            name: name.to_owned(),
            kind: "module".to_owned(),
            parent: None,
            start: SourcePoint {
                line: start,
                column: 1,
            },
            end: SourcePoint {
                line: end,
                column: 1,
            },
            signature: String::new(),
        };
        let mut symbols = vec![
            symbol("after", 9, 9),
            symbol("inner_fn", 4, 4),
            symbol("outer", 1, 8),
            symbol("inner", 3, 5),
            symbol("outer_fn", 6, 7),
        ];
        nest_symbols(&mut symbols);
        let parents = symbols
            .iter()
            .map(|s| (s.name.as_str(), s.parent.map(|p| symbols[p].name.as_str())))
            .collect::<Vec<_>>();
        assert_eq!(
            parents,
            [
                ("outer", None),
                ("inner", Some("outer")),
                ("inner_fn", Some("inner")),
                ("outer_fn", Some("outer")),
                ("after", None),
            ]
        );
    }

    #[test]
    fn matches_case_insensitively() {
        assert!(text_mentions("Retry the SQLite connection", "sqlite"));
    }

    #[test]
    fn matches_substring() {
        assert!(text_mentions("circuit breaker tripped", "circuit breaker"));
    }

    #[test]
    fn rejects_non_match() {
        assert!(!text_mentions("retry the connection", "sqlite"));
    }

    #[test]
    fn rejects_empty_needle() {
        assert!(!text_mentions("retry the connection", "   "));
    }

    #[test]
    fn pagerank_carries_a_seed_across_multiple_hops() {
        let rank = personalized_pagerank(
            &[vec![1], vec![0, 2], vec![1], vec![]],
            &[(0, 1.0)],
            0.5,
            32,
        );
        assert!(rank[2] > rank[3]);
    }

    #[test]
    fn pagerank_redistributes_dangling_mass_over_the_restart_vector() {
        let adjacency = [vec![1], vec![0], vec![], vec![]];
        let rank = personalized_pagerank(&adjacency, &[(0, 0.75), (2, 0.25)], 0.5, 64);
        assert!(rank[0] > rank[1]);
        assert!(rank[2] > rank[3]);
        assert!((rank.iter().sum::<f64>() - 1.0).abs() < 1e-9);
    }
}
