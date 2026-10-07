//! The panel, built as a tree. Each function makes one node of the protocol's UI tree; see
//! `docs/plugins/protocol.md` for what Agentty draws.

use serde_json::{json, Value};

/// A column of nodes.
pub fn column(children: Vec<Value>) -> Value {
    json!({ "type": "column", "children": children })
}

/// A row of nodes, wrapping onto more lines when there is no room.
pub fn row(children: Vec<Value>) -> Value {
    json!({ "type": "row", "children": children, "wrap": true })
}

/// A titled group.
pub fn section(title: impl Into<String>, children: Vec<Value>) -> Value {
    json!({ "type": "section", "title": title.into(), "children": children })
}

pub fn text(text: impl Into<String>) -> Value {
    json!({ "type": "text", "text": text.into() })
}

/// `body`, `title`, `muted`, `small`, `code`, `error` or `success`.
pub fn styled_text(text: impl Into<String>, style: &str) -> Value {
    json!({ "type": "text", "text": text.into(), "style": style })
}

pub fn button(id: impl Into<String>, label: impl Into<String>) -> Value {
    json!({ "type": "button", "id": id.into(), "label": label.into() })
}

/// `primary`, `secondary`, `ghost` or `danger`.
pub fn styled_button(id: impl Into<String>, label: impl Into<String>, variant: &str) -> Value {
    json!({ "type": "button", "id": id.into(), "label": label.into(), "variant": variant })
}

pub fn input(id: impl Into<String>, placeholder: impl Into<String>, value: impl Into<String>) -> Value {
    json!({ "type": "input", "id": id.into(), "placeholder": placeholder.into(), "value": value.into() })
}

/// A field of several lines (a request body, a note): Enter adds a line and a paste keeps its
/// own. At most 24 rows.
pub fn textarea(id: impl Into<String>, placeholder: impl Into<String>, value: impl Into<String>, rows: usize) -> Value {
    json!({ "type": "input", "id": id.into(), "placeholder": placeholder.into(), "value": value.into(), "rows": rows.max(2) })
}

pub fn choice(id: impl Into<String>, options: &[(&str, &str)], value: impl Into<String>) -> Value {
    let options: Vec<Value> = options.iter().map(|(v, label)| json!({ "value": v, "label": label })).collect();
    json!({ "type": "choice", "id": id.into(), "options": options, "value": value.into() })
}

pub fn toggle(id: impl Into<String>, label: impl Into<String>, on: bool) -> Value {
    json!({ "type": "toggle", "id": id.into(), "label": label.into(), "value": on })
}

pub fn list(id: impl Into<String>, items: Vec<Value>, empty: impl Into<String>) -> Value {
    json!({ "type": "list", "id": id.into(), "items": items, "empty": empty.into() })
}

pub fn item(id: impl Into<String>, title: impl Into<String>, subtitle: impl Into<String>) -> Value {
    json!({ "id": id.into(), "title": title.into(), "subtitle": subtitle.into() })
}

/// `neutral`, `info`, `success`, `warning` or `error`.
pub fn badge(text: impl Into<String>, tone: &str) -> Value {
    json!({ "type": "badge", "text": text.into(), "tone": tone })
}

pub fn spinner(text: impl Into<String>) -> Value {
    json!({ "type": "spinner", "text": text.into() })
}

pub fn divider() -> Value {
    json!({ "type": "divider" })
}

/// Adds an optional field to a node: `ui::with(ui::stat("Passed", "128"), "tone", "success")`.
/// Every field the protocol lists for a node can be set this way.
pub fn with(mut node: Value, key: &str, value: impl Into<Value>) -> Value {
    node[key] = value.into();
    node
}

// API 4 — say `"apiVersion": 4` in agentty-plugin.json when using these.

/// A raised box around what belongs together, with a heading. Set `subtitle`, `icon` and `tone`
/// (it colors the icon and the edge) with [`with`].
pub fn card(title: impl Into<String>, children: Vec<Value>) -> Value {
    json!({ "type": "card", "title": title.into(), "children": children })
}

/// Children in equal columns (1 to 6), wrapping onto new rows.
pub fn grid(columns: usize, children: Vec<Value>) -> Value {
    json!({ "type": "grid", "columns": columns.clamp(1, 6), "children": children })
}

/// Columns of set widths: `"240px"` is fixed, `"2"` takes twice the share of a `"1"`. A sidebar
/// beside a page is `ui::split(&["240px", "1"], vec![sidebar, page])`.
pub fn split(widths: &[&str], children: Vec<Value>) -> Value {
    json!({ "type": "grid", "widths": widths, "children": children })
}

/// A tab strip of `(id, label)`. `children` is only the picked tab's content; picking another
/// sends `change` with its id.
pub fn tabs(id: impl Into<String>, tabs: &[(&str, &str)], value: impl Into<String>, children: Vec<Value>) -> Value {
    let tabs: Vec<Value> = tabs.iter().map(|(id, label)| json!({ "id": id, "label": label })).collect();
    json!({ "type": "tabs", "id": id.into(), "tabs": tabs, "value": value.into(), "children": children })
}

/// Tabs with a close button each: closing one sends `close` with its id as `value`, picking one
/// sends `change`.
pub fn closable_tabs(id: impl Into<String>, tabs: &[(&str, &str)], value: impl Into<String>, children: Vec<Value>) -> Value {
    let tabs: Vec<Value> = tabs.iter().map(|(id, label)| json!({ "id": id, "label": label, "closable": true })).collect();
    json!({ "type": "tabs", "id": id.into(), "tabs": tabs, "value": value.into(), "children": children })
}

/// A row of a tree in a [`list`]: indented `depth` levels (0 to 8), with a short colored `tag`
/// before the title (an HTTP method, a status) in `tone` — empty for none.
pub fn tree_item(id: impl Into<String>, title: impl Into<String>, depth: u8, tag: &str, tone: &str) -> Value {
    let mut item = json!({ "id": id.into(), "title": title.into(), "depth": depth.min(8) });
    if !tag.is_empty() {
        item["tag"] = json!(tag);
        item["tagTone"] = json!(tone);
    }
    item
}

/// A text area in the monospace font, for code, JSON or a script (at most 24 rows).
pub fn code_area(id: impl Into<String>, placeholder: impl Into<String>, value: impl Into<String>, rows: usize) -> Value {
    json!({ "type": "input", "id": id.into(), "placeholder": placeholder.into(), "value": value.into(), "rows": rows.clamp(2, 24), "mono": true })
}

/// Rows under column headings. A click on a row sends `select` with its id as `item`. Set a
/// column's `align` (`start`, `center`, `end`) or `grow` (its share of the width) with [`with`]
/// on [`column_heading`].
pub fn table(id: impl Into<String>, columns: Vec<Value>, rows: Vec<Value>, empty: impl Into<String>) -> Value {
    json!({ "type": "table", "id": id.into(), "columns": columns, "rows": rows, "empty": empty.into() })
}

/// A heading of a [`table`].
pub fn column_heading(label: impl Into<String>) -> Value {
    json!({ "label": label.into() })
}

/// A row of a [`table`]: one cell per column. Its `tone` marks the first cell.
pub fn table_row(id: impl Into<String>, cells: Vec<String>) -> Value {
    json!({ "id": id.into(), "cells": cells })
}

/// Label and value pairs, one per line. Set an item's `tone` or `mono` with [`with`] on
/// [`pair`].
pub fn key_value(items: Vec<Value>) -> Value {
    json!({ "type": "keyValue", "items": items })
}

/// One line of a [`key_value`].
pub fn pair(label: impl Into<String>, value: impl Into<String>) -> Value {
    json!({ "label": label.into(), "value": value.into() })
}

/// One number that matters, large. Set `detail`, `icon` and `tone` with [`with`].
pub fn stat(label: impl Into<String>, value: impl Into<String>) -> Value {
    json!({ "type": "stat", "label": label.into(), "value": value.into() })
}

/// A bar filled `value` of the way (0 to 1). `detail` replaces the percentage on the right.
pub fn progress(value: f64, label: impl Into<String>) -> Value {
    json!({ "type": "progress", "value": if value.is_finite() { value.clamp(0., 1.) } else { 0. }, "label": label.into() })
}

/// A tinted note: `info`, `success`, `warning`, `error` or `neutral`. Set `title` with [`with`].
pub fn callout(text: impl Into<String>, tone: &str) -> Value {
    json!({ "type": "callout", "text": text.into(), "tone": tone })
}

/// A drop-down of `(value, label)`; picking one sends `change` with its value.
pub fn select(id: impl Into<String>, options: &[(&str, &str)], value: impl Into<String>) -> Value {
    let options: Vec<Value> = options.iter().map(|(v, label)| json!({ "value": v, "label": label })).collect();
    json!({ "type": "select", "id": id.into(), "options": options, "value": value.into() })
}

/// A box to tick; sends `change` with the new boolean. Set `description` with [`with`].
pub fn checkbox(id: impl Into<String>, label: impl Into<String>, on: bool) -> Value {
    json!({ "type": "checkbox", "id": id.into(), "label": label.into(), "value": on })
}

/// Monospaced text colored by `language` (`rust`, `json`, `ts`, `sh`, …), with a copy button.
pub fn code(text: impl Into<String>, language: &str) -> Value {
    json!({ "type": "code", "text": text.into(), "language": language })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_builders() {
        let page = split(&["240px", "1"], vec![list("tree", vec![tree_item("r1", "Create vehicle", 2, "POST", "warning"), tree_item("f", "vehicles", 1, "", "")], ""), code_area("body", "{}", "", 40)]);
        assert_eq!(page["widths"][0], "240px");
        assert_eq!(page["children"][0]["items"][0]["tagTone"], "warning");
        assert!(page["children"][0]["items"][1].get("tag").is_none());
        assert_eq!((page["children"][1]["mono"].as_bool(), page["children"][1]["rows"].as_u64()), (Some(true), Some(24)));
        let t = closable_tabs("tabs", &[("a", "GET Users")], "a", vec![]);
        assert_eq!(t["tabs"][0]["closable"], true);
    }

    #[test]
    fn nodes_carry_their_type() {
        let tree = column(vec![section("Request", vec![input("url", "https://…", ""), button("go", "Send")]), divider()]);
        assert_eq!(tree["type"], "column");
        assert_eq!(tree["children"][0]["title"], "Request");
        assert_eq!(tree["children"][0]["children"][1]["id"], "go");
        assert_eq!(tree["children"][1]["type"], "divider");
        assert_eq!(choice("m", &[("GET", "GET")], "GET")["options"][0]["value"], "GET");
        assert_eq!(textarea("body", "…", "", 10)["rows"], 10);
    }

    #[test]
    fn api_4_nodes_carry_their_fields() {
        let tree = tabs(
            "t",
            &[("runs", "Runs"), ("settings", "Settings")],
            "runs",
            vec![
                grid(9, vec![with(stat("Passed", "128"), "tone", "success"), stat("Failed", "2")]),
                card("Latest", vec![key_value(vec![with(pair("Commit", "ebfe735"), "mono", true)]), progress(3.0, "Upload")]),
                table(
                    "runs",
                    vec![column_heading("Job"), with(column_heading("Time"), "align", "end")],
                    vec![table_row("a", vec!["build".into(), "2m".into()])],
                    "No runs",
                ),
                callout("Heads up", "warning"),
                select("region", &[("seoul", "Seoul")], "seoul"),
                checkbox("notify", "Notify me", true),
                code("fn main() {}", "rust"),
            ],
        );
        assert_eq!(tree["tabs"][1]["id"], "settings");
        let children = &tree["children"];
        assert_eq!(children[0]["columns"], 6, "a grid has at most six columns");
        assert_eq!(children[0]["children"][0]["tone"], "success");
        assert_eq!(children[1]["children"][0]["type"], "keyValue");
        assert_eq!(children[1]["children"][1]["value"], 1.0, "progress is clamped");
        assert_eq!(children[2]["columns"][1]["align"], "end");
        assert_eq!(children[2]["rows"][0]["cells"][1], "2m");
        assert_eq!(children[4]["options"][0]["value"], "seoul");
        assert_eq!(children[6]["language"], "rust");
    }
}
