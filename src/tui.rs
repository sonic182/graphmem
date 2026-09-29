use std::{
    error::Error,
    fs::{self, OpenOptions},
    io::{IsTerminal, Write},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

use graphmem::{
    Entity, EntityReference, GraphDirection, Memory,
    application::{GraphRequest, MemoryService},
    infrastructure::config::ConfigOverrides,
};
use pulldown_cmark::{Event as MarkdownEvent, Parser as MarkdownParser, Tag, TagEnd};
use ratatui::{
    DefaultTerminal, Frame,
    crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    layout::{Constraint, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Block, Clear, List, ListItem, ListState, Paragraph, Wrap},
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum View {
    Memories,
    Graph,
}

#[derive(Clone, Copy)]
enum DeleteTarget {
    Memory(i64),
    Entity(i64),
}

struct App {
    service: MemoryService,
    all_memories: Vec<Memory>,
    memories: Vec<Memory>,
    entities: Vec<Entity>,
    memory_state: ListState,
    entity_state: ListState,
    view: View,
    detail: Text<'static>,
    detail_scroll: u16,
    confirmation: Option<DeleteTarget>,
    message: String,
    filter: String,
    filter_before_edit: Option<String>,
}

impl App {
    fn new(service: MemoryService) -> Result<Self, Box<dyn Error>> {
        let mut app = Self {
            service,
            all_memories: Vec::new(),
            memories: Vec::new(),
            entities: Vec::new(),
            memory_state: ListState::default(),
            entity_state: ListState::default(),
            view: View::Memories,
            detail: Text::default(),
            detail_scroll: 0,
            confirmation: None,
            message: String::new(),
            filter: String::new(),
            filter_before_edit: None,
        };
        app.reload()?;
        Ok(app)
    }

    fn reload(&mut self) -> Result<(), Box<dyn Error>> {
        self.all_memories = self.service.list_all()?;
        self.apply_filter();
        self.entities = self.service.entities()?;
        self.entity_state
            .select((!self.entities.is_empty()).then(|| {
                self.entity_state
                    .selected()
                    .unwrap_or(0)
                    .min(self.entities.len() - 1)
            }));
        self.refresh_detail()
    }

    fn apply_filter(&mut self) {
        let needle = self.filter.trim().to_lowercase();
        self.memories = self
            .all_memories
            .iter()
            .filter(|memory| {
                needle.is_empty()
                    || memory.content.to_lowercase().contains(&needle)
                    || memory.memory_type.to_lowercase().contains(&needle)
            })
            .cloned()
            .collect();
        self.memory_state
            .select((!self.memories.is_empty()).then(|| {
                self.memory_state
                    .selected()
                    .unwrap_or(0)
                    .min(self.memories.len() - 1)
            }));
    }

    fn refresh_detail(&mut self) -> Result<(), Box<dyn Error>> {
        self.detail_scroll = 0;
        self.detail = match self.view {
            View::Memories => match self
                .memory_state
                .selected()
                .and_then(|index| self.memories.get(index))
            {
                Some(memory) => {
                    let details = self.service.show(memory.id, None)?;
                    let scopes = details
                        .scopes
                        .iter()
                        .map(|scope| scope.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ");
                    let mut detail = Text::from(vec![
                        Line::styled(
                            format!("id: {}", memory.id),
                            Style::default().fg(Color::Gray),
                        ),
                        Line::styled(
                            format!("type: {}", memory.memory_type),
                            Style::default().fg(Color::Gray),
                        ),
                        Line::styled(
                            format!("importance: {}", memory.importance),
                            Style::default().fg(Color::Gray),
                        ),
                        Line::styled(
                            format!("scopes: {scopes}"),
                            Style::default().fg(Color::Gray),
                        ),
                        Line::default(),
                    ]);
                    detail.lines.extend(markdown_lines(&memory.content));
                    detail
                }
                None => Text::from("No matching memories"),
            },
            View::Graph => match self
                .entity_state
                .selected()
                .and_then(|index| self.entities.get(index))
            {
                Some(entity) => {
                    let graph = self.service.graph(GraphRequest {
                        entity: EntityReference {
                            kind: entity.kind.clone(),
                            name: entity.name.clone(),
                        },
                        direction: GraphDirection::Both,
                        max_depth: 1,
                        limit: 100,
                    })?;
                    let mut detail = format!(
                        "id: {}\nkind: {}\nname: {}\n\nRelations (up to 100):\n",
                        entity.id, entity.kind, entity.name
                    );
                    for path in graph.paths {
                        if let Some(hop) = path.hops.first() {
                            let arrow = match hop.direction {
                                GraphDirection::Incoming => "<-",
                                GraphDirection::Outgoing => "->",
                                GraphDirection::Both => unreachable!(),
                            };
                            detail.push_str(&format!(
                                "{} {} {}:{}\n",
                                arrow, hop.edge.relation, hop.entity.kind, hop.entity.name
                            ));
                        }
                    }
                    Text::from(detail)
                }
                None => Text::from("No graph nodes"),
            },
        };
        Ok(())
    }

    fn move_selection(&mut self, step: isize) -> Result<(), Box<dyn Error>> {
        let (state, len) = match self.view {
            View::Memories => (&mut self.memory_state, self.memories.len()),
            View::Graph => (&mut self.entity_state, self.entities.len()),
        };
        if len > 0 {
            let index = state.selected().unwrap_or(0);
            state.select(Some(index.saturating_add_signed(step).min(len - 1)));
            self.refresh_detail()?;
        }
        Ok(())
    }

    fn draw(&mut self, frame: &mut Frame<'_>) {
        let rows = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(3),
            Constraint::Length(2),
        ])
        .split(frame.area());
        let header = match self.view {
            View::Memories => format!(
                " Memories  [Graph: Tab]  /{}{}",
                self.filter,
                if self.filter_before_edit.is_some() {
                    "▌"
                } else {
                    ""
                }
            ),
            View::Graph => " Graph  [Memories: Tab]".to_owned(),
        };
        frame.render_widget(
            Paragraph::new(header).style(
                Style::default()
                    .fg(Color::Cyan)
                    .add_modifier(Modifier::BOLD),
            ),
            rows[0],
        );
        let columns = Layout::horizontal([Constraint::Percentage(40), Constraint::Percentage(60)])
            .split(rows[1]);
        match self.view {
            View::Memories => {
                let items = self.memories.iter().map(|memory| {
                    let preview = memory.content.lines().next().unwrap_or("");
                    ListItem::new(format!(
                        "{}  [{}] {}",
                        memory.id, memory.memory_type, preview
                    ))
                });
                let list = List::new(items)
                    .block(
                        Block::bordered()
                            .title("Memories")
                            .border_style(Style::default().fg(Color::Blue)),
                    )
                    .highlight_style(
                        Style::default()
                            .fg(Color::Black)
                            .bg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    )
                    .highlight_symbol("› ");
                frame.render_stateful_widget(list, columns[0], &mut self.memory_state);
            }
            View::Graph => {
                let items = self.entities.iter().map(|entity| {
                    ListItem::new(format!("{}  {}:{}", entity.id, entity.kind, entity.name))
                });
                let list = List::new(items)
                    .block(
                        Block::bordered()
                            .title("Graph nodes")
                            .border_style(Style::default().fg(Color::Blue)),
                    )
                    .highlight_style(
                        Style::default()
                            .fg(Color::Black)
                            .bg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    )
                    .highlight_symbol("› ");
                frame.render_stateful_widget(list, columns[0], &mut self.entity_state);
            }
        }
        frame.render_widget(
            Paragraph::new(self.detail.clone())
                .block(
                    Block::bordered()
                        .title("Details")
                        .border_style(Style::default().fg(Color::Blue)),
                )
                .wrap(Wrap { trim: false })
                .scroll((self.detail_scroll, 0)),
            columns[1],
        );
        let actions = match self.view {
            View::Memories => {
                "j/k move  PgUp/Dn scroll  / filter  Tab view  e edit  d delete  r reload  q quit"
            }
            View::Graph => "j/k move  PgUp/Dn scroll  Tab view  d delete  r reload  q quit",
        };
        let footer = if self.filter_before_edit.is_some() {
            "Type to filter  Enter apply  Esc cancel  Ctrl-U clear".to_owned()
        } else {
            actions.to_owned()
        };
        frame.render_widget(
            Paragraph::new(vec![
                Line::styled(footer, Style::default().fg(Color::Yellow)),
                Line::styled(self.message.as_str(), Style::default().fg(Color::Green)),
            ]),
            rows[2],
        );

        if let Some(target) = self.confirmation {
            let area = frame.area();
            let width = area.width.min(68);
            let height = 7.min(area.height);
            let popup = ratatui::layout::Rect::new(
                area.x + (area.width - width) / 2,
                area.y + (area.height - height) / 2,
                width,
                height,
            );
            let prompt = match target {
                DeleteTarget::Memory(id) => format!("Delete memory {id}? This cannot be undone."),
                DeleteTarget::Entity(id) => format!(
                    "Delete entity {id}? Its edges and memory links will be removed; memories stay."
                ),
            };
            frame.render_widget(Clear, popup);
            frame.render_widget(
                Paragraph::new(format!("{prompt}\n\ny confirm   n/Esc cancel"))
                    .block(
                        Block::bordered()
                            .title("Confirm delete")
                            .border_style(Style::default().fg(Color::Red)),
                    )
                    .wrap(Wrap { trim: false }),
                popup,
            );
        }
    }

    fn handle_key(
        &mut self,
        key: KeyEvent,
        terminal: &mut DefaultTerminal,
    ) -> Result<bool, Box<dyn Error>> {
        if key.kind != KeyEventKind::Press {
            return Ok(false);
        }
        if let Some(previous) = self.filter_before_edit.as_ref() {
            match key.code {
                KeyCode::Enter => self.filter_before_edit = None,
                KeyCode::Esc => {
                    self.filter = previous.clone();
                    self.filter_before_edit = None;
                    self.apply_filter();
                    self.refresh_detail()?;
                }
                KeyCode::Backspace => {
                    self.filter.pop();
                    self.apply_filter();
                    self.refresh_detail()?;
                }
                KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.filter.clear();
                    self.apply_filter();
                    self.refresh_detail()?;
                }
                KeyCode::Char(character) => {
                    self.filter.push(character);
                    self.apply_filter();
                    self.refresh_detail()?;
                }
                _ => {}
            }
            return Ok(false);
        }
        if let Some(target) = self.confirmation {
            match key.code {
                KeyCode::Char('y') => {
                    self.confirmation = None;
                    let result = match target {
                        DeleteTarget::Memory(id) => self.service.forget(id, None),
                        DeleteTarget::Entity(id) => self.service.delete_entity(id),
                    };
                    match result {
                        Ok(()) => {
                            self.reload()?;
                            self.message = "Deleted".to_owned();
                        }
                        Err(error) => self.message = format!("Delete failed: {error}"),
                    }
                }
                KeyCode::Char('n') | KeyCode::Esc => self.confirmation = None,
                _ => {}
            }
            return Ok(false);
        }
        match key.code {
            KeyCode::Char('q') => return Ok(true),
            KeyCode::Char('/') => {
                self.view = View::Memories;
                self.filter_before_edit = Some(self.filter.clone());
                self.refresh_detail()?;
            }
            KeyCode::Tab => {
                self.view = match self.view {
                    View::Memories => View::Graph,
                    View::Graph => View::Memories,
                };
                self.refresh_detail()?;
            }
            KeyCode::Char('j') | KeyCode::Down => self.move_selection(1)?,
            KeyCode::Char('k') | KeyCode::Up => self.move_selection(-1)?,
            KeyCode::PageDown => self.detail_scroll = self.detail_scroll.saturating_add(5),
            KeyCode::PageUp => self.detail_scroll = self.detail_scroll.saturating_sub(5),
            KeyCode::Char('r') => {
                self.reload()?;
                self.message = "Reloaded".to_owned();
            }
            KeyCode::Char('d') => {
                self.confirmation = match self.view {
                    View::Memories => self
                        .memory_state
                        .selected()
                        .and_then(|index| self.memories.get(index))
                        .map(|memory| DeleteTarget::Memory(memory.id)),
                    View::Graph => self
                        .entity_state
                        .selected()
                        .and_then(|index| self.entities.get(index))
                        .map(|entity| DeleteTarget::Entity(entity.id)),
                };
            }
            KeyCode::Char('e') if self.view == View::Memories => self.edit_memory(terminal)?,
            _ => {}
        }
        Ok(false)
    }

    fn edit_memory(&mut self, terminal: &mut DefaultTerminal) -> Result<(), Box<dyn Error>> {
        let Some(memory) = self
            .memory_state
            .selected()
            .and_then(|index| self.memories.get(index))
        else {
            return Ok(());
        };
        let Some(editor) = std::env::var("VISUAL")
            .ok()
            .filter(|value| !value.trim().is_empty())
            .or_else(|| {
                std::env::var("EDITOR")
                    .ok()
                    .filter(|value| !value.trim().is_empty())
            })
        else {
            self.message = "Set VISUAL or EDITOR to edit memories".to_owned();
            return Ok(());
        };
        let id = memory.id;
        let previous = memory.content.clone();
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let path =
            std::env::temp_dir().join(format!("gmem-edit-{}-{id}-{nonce}", std::process::id()));
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&path)?;
        file.write_all(previous.as_bytes())?;
        drop(file);

        ratatui::restore();
        let edited = (|| -> Result<String, Box<dyn Error>> {
            let status = Command::new("sh")
                .arg("-c")
                .arg(format!("exec {editor} \"$1\""))
                .arg("gmem-editor")
                .arg(&path)
                .status()?;
            if !status.success() {
                return Err(format!("editor exited with {status}").into());
            }
            Ok(fs::read_to_string(&path)?)
        })();
        *terminal = ratatui::try_init().map_err(|error| {
            format!(
                "terminal recovery failed; edit kept at {}: {error}",
                path.display()
            )
        })?;
        match edited {
            Ok(content) if content != previous => {
                match self.service.update(id, Some(content), None, None, None) {
                    Ok(_) => {
                        let _ = fs::remove_file(&path);
                        self.reload()?;
                        self.message = "Memory updated".to_owned();
                    }
                    Err(error) => {
                        self.message =
                            format!("Update failed: {error}; edit kept at {}", path.display());
                    }
                }
            }
            Ok(_) => {
                let _ = fs::remove_file(&path);
                self.message = "No changes".to_owned();
            }
            Err(error) => {
                self.message = format!("Edit failed: {error}; edit kept at {}", path.display());
            }
        }
        Ok(())
    }
}

fn finish_line(lines: &mut Vec<Line<'static>>, spans: &mut Vec<Span<'static>>) {
    lines.push(Line::from(std::mem::take(spans)));
}

fn markdown_style(heading: bool, code: bool, bold: bool, italic: bool, link: bool) -> Style {
    let mut style = Style::default().fg(if heading {
        Color::Cyan
    } else if code {
        Color::Yellow
    } else if link {
        Color::Blue
    } else {
        Color::Reset
    });
    if heading || bold {
        style = style.add_modifier(Modifier::BOLD);
    }
    if italic {
        style = style.add_modifier(Modifier::ITALIC);
    }
    if link {
        style = style.add_modifier(Modifier::UNDERLINED);
    }
    style
}

fn push_markdown_text(
    lines: &mut Vec<Line<'static>>,
    spans: &mut Vec<Span<'static>>,
    value: &str,
    style: Style,
) {
    for (index, part) in value.split('\n').enumerate() {
        if index > 0 {
            finish_line(lines, spans);
        }
        if !part.is_empty() {
            spans.push(Span::styled(part.to_owned(), style));
        }
    }
}

fn markdown_lines(input: &str) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let mut spans = Vec::new();
    let mut heading = false;
    let mut code_block = false;
    let mut bold = false;
    let mut italic = false;
    let mut link: Option<String> = None;
    let mut lists: Vec<Option<u64>> = Vec::new();
    for event in MarkdownParser::new(input) {
        match event {
            MarkdownEvent::Start(Tag::Heading { .. }) => heading = true,
            MarkdownEvent::End(TagEnd::Heading(_)) => {
                heading = false;
                finish_line(&mut lines, &mut spans);
                lines.push(Line::default());
            }
            MarkdownEvent::Start(Tag::CodeBlock(_)) => {
                if !spans.is_empty() {
                    finish_line(&mut lines, &mut spans);
                }
                code_block = true;
            }
            MarkdownEvent::End(TagEnd::CodeBlock) => {
                if !spans.is_empty() {
                    finish_line(&mut lines, &mut spans);
                }
                code_block = false;
                lines.push(Line::default());
            }
            MarkdownEvent::Start(Tag::Strong) => bold = true,
            MarkdownEvent::End(TagEnd::Strong) => bold = false,
            MarkdownEvent::Start(Tag::Emphasis) => italic = true,
            MarkdownEvent::End(TagEnd::Emphasis) => italic = false,
            MarkdownEvent::Start(Tag::Link { dest_url, .. }) => link = Some(dest_url.to_string()),
            MarkdownEvent::End(TagEnd::Link) => {
                if let Some(url) = link.take() {
                    spans.push(Span::styled(
                        format!(" <{url}>"),
                        Style::default().fg(Color::Gray),
                    ));
                }
            }
            MarkdownEvent::Start(Tag::List(first)) => lists.push(first),
            MarkdownEvent::End(TagEnd::List(_)) => {
                lists.pop();
                if lists.is_empty() {
                    lines.push(Line::default());
                }
            }
            MarkdownEvent::Start(Tag::Item) => {
                let marker = match lists.last_mut() {
                    Some(Some(number)) => {
                        let marker = format!("{number}. ");
                        *number += 1;
                        marker
                    }
                    _ => "• ".to_owned(),
                };
                spans.push(Span::styled(
                    format!("{}{marker}", "  ".repeat(lists.len().saturating_sub(1))),
                    Style::default().fg(Color::Magenta),
                ));
            }
            MarkdownEvent::End(TagEnd::Item) => {
                if !spans.is_empty() {
                    finish_line(&mut lines, &mut spans);
                }
            }
            MarkdownEvent::End(TagEnd::Paragraph) => {
                if !spans.is_empty() {
                    finish_line(&mut lines, &mut spans);
                }
                if lists.is_empty() {
                    lines.push(Line::default());
                }
            }
            MarkdownEvent::Text(value) => push_markdown_text(
                &mut lines,
                &mut spans,
                &value,
                markdown_style(heading, code_block, bold, italic, link.is_some()),
            ),
            MarkdownEvent::Code(value) => push_markdown_text(
                &mut lines,
                &mut spans,
                &value,
                markdown_style(heading, true, bold, italic, link.is_some()),
            ),
            MarkdownEvent::Html(value) | MarkdownEvent::InlineHtml(value) => push_markdown_text(
                &mut lines,
                &mut spans,
                &value,
                Style::default().fg(Color::Gray),
            ),
            MarkdownEvent::SoftBreak => spans.push(Span::raw(" ")),
            MarkdownEvent::HardBreak => finish_line(&mut lines, &mut spans),
            MarkdownEvent::Rule => {
                lines.push(Line::styled("────────", Style::default().fg(Color::Gray)))
            }
            _ => {}
        }
    }
    if !spans.is_empty() {
        finish_line(&mut lines, &mut spans);
    }
    lines
}

pub fn run(overrides: ConfigOverrides) -> Result<(), Box<dyn Error>> {
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        return Err("gmem tui requires an interactive terminal".into());
    }
    let mut app = App::new(MemoryService::open_default(overrides)?)?;
    let mut terminal = ratatui::try_init()?;
    let result = (|| -> Result<(), Box<dyn Error>> {
        loop {
            terminal.draw(|frame| app.draw(frame))?;
            if let Event::Key(key) = event::read()?
                && app.handle_key(key, &mut terminal)?
            {
                break;
            }
        }
        Ok(())
    })();
    ratatui::restore();
    result
}

#[cfg(test)]
mod tests {
    use ratatui::style::Color;

    use super::markdown_lines;

    #[test]
    fn markdown_details_keep_heading_list_and_code_styles() {
        let lines = markdown_lines("# Heading\n\n- **bold** and `code`\n");
        assert_eq!(lines[0].spans[0].content, "Heading");
        assert_eq!(lines[0].spans[0].style.fg, Some(Color::Cyan));
        let item = lines
            .iter()
            .find(|line| line.spans.iter().any(|span| span.content == "bold"))
            .unwrap();
        assert_eq!(item.spans[0].content, "• ");
        assert_eq!(
            item.spans
                .iter()
                .find(|span| span.content == "code")
                .unwrap()
                .style
                .fg,
            Some(Color::Yellow)
        );
    }
}
