use crossterm::style::{Attribute, Color, SetAttribute, SetForegroundColor, ResetColor};
use scraper::{Html, Node};
use std::fmt::Write;

pub struct StyledLine {
    pub text: String,
}

pub fn html_to_terminal(html: &str) -> Vec<StyledLine> {
    let doc = Html::parse_document(html);
    let mut lines = Vec::new();
    let mut current_line = String::new();

    process_node(doc.root_element().id(), &doc, &mut lines, &mut current_line, &Context::default());

    if !current_line.trim().is_empty() {
        lines.push(StyledLine { text: current_line });
    }

    lines
}

#[derive(Default, Clone)]
struct Context {
    in_pre: bool,
    in_code: bool,
    in_bold: bool,
    in_italic: bool,
    in_heading: u8, // 0 = none, 1-6 = h1-h6
    list_depth: usize,
    ordered_list: bool,
    list_index: usize,
}

fn process_node(
    node_id: ego_tree::NodeId,
    doc: &Html,
    lines: &mut Vec<StyledLine>,
    current: &mut String,
    ctx: &Context,
) {
    let tree_node = doc.tree.get(node_id).unwrap();

    match tree_node.value() {
        Node::Text(text) => {
            let t = if ctx.in_pre {
                text.to_string()
            } else {
                // Collapse whitespace
                let collapsed: String = text
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ");
                collapsed
            };

            if !t.is_empty() {
                if ctx.in_pre {
                    // Preserve formatting in code blocks
                    for line in t.split('\n') {
                        if !current.is_empty() && current.ends_with('\n') {
                            lines.push(StyledLine {
                                text: std::mem::take(current),
                            });
                        }
                        write!(
                            current,
                            "{}{}{}",
                            SetForegroundColor(Color::Green),
                            line,
                            ResetColor
                        )
                        .ok();
                        current.push('\n');
                    }
                } else if ctx.in_code {
                    write!(
                        current,
                        "{}{}{}",
                        SetForegroundColor(Color::Yellow),
                        t,
                        ResetColor
                    )
                    .ok();
                } else if ctx.in_heading > 0 {
                    write!(
                        current,
                        "{}{}{}{}",
                        SetAttribute(Attribute::Bold),
                        SetForegroundColor(Color::Cyan),
                        t,
                        ResetColor
                    )
                    .ok();
                    write!(current, "{}", SetAttribute(Attribute::Reset)).ok();
                } else if ctx.in_bold {
                    write!(
                        current,
                        "{}{}{}",
                        SetAttribute(Attribute::Bold),
                        t,
                        SetAttribute(Attribute::Reset)
                    )
                    .ok();
                } else if ctx.in_italic {
                    write!(
                        current,
                        "{}{}{}",
                        SetAttribute(Attribute::Italic),
                        t,
                        SetAttribute(Attribute::Reset)
                    )
                    .ok();
                } else {
                    current.push_str(&t);
                }
            }
        }
        Node::Element(el) => {
            let tag = el.name();
            let mut child_ctx = ctx.clone();

            match tag {
                "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                    flush_line(current, lines);
                    lines.push(StyledLine { text: String::new() });
                    child_ctx.in_heading = tag.as_bytes()[1] - b'0';
                    let prefix = "#".repeat(child_ctx.in_heading as usize);
                    write!(
                        current,
                        "{}{}{} ",
                        SetAttribute(Attribute::Bold),
                        SetForegroundColor(Color::Cyan),
                        prefix,
                    )
                    .ok();
                }
                "p" => {
                    flush_line(current, lines);
                }
                "br" => {
                    flush_line(current, lines);
                }
                "pre" => {
                    flush_line(current, lines);
                    lines.push(StyledLine {
                        text: format!("{}---", SetForegroundColor(Color::DarkGreen)),
                    });
                    child_ctx.in_pre = true;
                }
                "code" if !ctx.in_pre => {
                    child_ctx.in_code = true;
                }
                "strong" | "b" => {
                    child_ctx.in_bold = true;
                }
                "em" | "i" => {
                    child_ctx.in_italic = true;
                }
                "ul" => {
                    flush_line(current, lines);
                    child_ctx.list_depth = ctx.list_depth + 1;
                    child_ctx.ordered_list = false;
                    child_ctx.list_index = 0;
                }
                "ol" => {
                    flush_line(current, lines);
                    child_ctx.list_depth = ctx.list_depth + 1;
                    child_ctx.ordered_list = true;
                    child_ctx.list_index = 0;
                }
                "li" => {
                    flush_line(current, lines);
                    let indent = "  ".repeat(ctx.list_depth);
                    if ctx.ordered_list {
                        child_ctx.list_index = ctx.list_index + 1;
                        write!(current, "{}{}. ", indent, child_ctx.list_index).ok();
                    } else {
                        write!(current, "{}\u{2022} ", indent).ok();
                    }
                }
                "blockquote" => {
                    flush_line(current, lines);
                    write!(
                        current,
                        "{}  \u{2502} ",
                        SetForegroundColor(Color::DarkGrey)
                    )
                    .ok();
                }
                "div" | "section" | "article" | "main" | "body" | "html" | "head" => {
                    // structural elements, just recurse
                }
                "a" => {
                    // render link text normally, could add URL
                }
                "img" => {
                    let alt = el.attr("alt").unwrap_or("[image]");
                    write!(
                        current,
                        "{}[{}]{}",
                        SetForegroundColor(Color::DarkGrey),
                        alt,
                        ResetColor
                    )
                    .ok();
                }
                "table" => {
                    flush_line(current, lines);
                    lines.push(StyledLine {
                        text: format!(
                            "{}[table]{}",
                            SetForegroundColor(Color::DarkGrey),
                            ResetColor
                        ),
                    });
                }
                "tr" => {
                    flush_line(current, lines);
                }
                "td" | "th" => {
                    current.push_str(" | ");
                }
                "script" | "style" | "link" | "meta" | "title" | "nav" | "footer" | "header" => {
                    return; // skip non-content elements
                }
                _ => {}
            }

            // Process children
            if let Some(first_child) = tree_node.first_child() {
                let mut child_id = first_child.id();
                loop {
                    process_node(child_id, doc, lines, current, &child_ctx);
                    match doc.tree.get(child_id).unwrap().next_sibling() {
                        Some(sibling) => child_id = sibling.id(),
                        None => break,
                    }
                }
            }

            // Post-processing for block elements
            match tag {
                "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
                    write!(current, "{}", ResetColor).ok();
                    write!(current, "{}", SetAttribute(Attribute::Reset)).ok();
                    flush_line(current, lines);
                    lines.push(StyledLine { text: String::new() });
                }
                "p" | "div" | "blockquote" => {
                    flush_line(current, lines);
                }
                "pre" => {
                    flush_line(current, lines);
                    lines.push(StyledLine {
                        text: format!(
                            "{}---{}",
                            SetForegroundColor(Color::DarkGreen),
                            ResetColor
                        ),
                    });
                }
                "ul" | "ol" => {
                    flush_line(current, lines);
                }
                _ => {}
            }
        }
        _ => {
            // Process children for other node types
            if let Some(first_child) = tree_node.first_child() {
                let mut child_id = first_child.id();
                loop {
                    process_node(child_id, doc, lines, current, ctx);
                    match doc.tree.get(child_id).unwrap().next_sibling() {
                        Some(sibling) => child_id = sibling.id(),
                        None => break,
                    }
                }
            }
        }
    }
}

fn flush_line(current: &mut String, lines: &mut Vec<StyledLine>) {
    let trimmed = current.trim().to_string();
    if !trimmed.is_empty() {
        lines.push(StyledLine { text: trimmed });
    }
    current.clear();
}
