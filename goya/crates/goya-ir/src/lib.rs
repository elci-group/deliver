//! GY-IR/1 — Typed intermediate representation and capability schema.
//!
//! Contract: stable within Wave 1. Changes require architecture approval
//! (Candidate stability level, see docs/contracts.md).

use std::fmt;

/// Semantic version of the IR contract this crate implements.
pub const IR_CONTRACT: &str = "GY-IR/1";

/// Unique identifier for an IR node within a module.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NodeId(pub u64);

/// A typed UI element in the Goya IR.
#[derive(Debug, Clone, PartialEq)]
pub enum Node {
    /// A rectangular container that lays out children along its axis.
    Column {
        children: Vec<NodeId>,
    },
    Row {
        children: Vec<NodeId>,
    },
    /// A leaf displaying text.
    Text {
        content: String,
    },
    /// A clickable region producing `Event::Activated`.
    Button {
        label: String,
        on_activate: ActionId,
    },
    /// Fixed-size spacer.
    Spacer {
        size: f64,
    },
}

/// Identifies an action the runtime dispatches when an event fires.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ActionId(pub u64);

/// Capabilities a module may request. Denied unless granted in the manifest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Capability {
    Window,
    TextInput,
    Network,
    FileSystem,
    TextToSpeech,
    Motion,
    Sensors,
    Graphics3d,
}

impl fmt::Display for Capability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            Capability::Window => "window",
            Capability::TextInput => "text-input",
            Capability::Network => "network",
            Capability::FileSystem => "filesystem",
            Capability::TextToSpeech => "tts",
            Capability::Motion => "motion",
            Capability::Sensors => "sensors",
            Capability::Graphics3d => "graphics3d",
        };
        f.write_str(s)
    }
}

/// A parsed module: nodes plus the root of the tree.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Module {
    pub nodes: Vec<(NodeId, Node)>,
    pub root: Option<NodeId>,
}

impl Module {
    pub fn node(&self, id: NodeId) -> Option<&Node> {
        self.nodes
            .iter()
            .find(|(n, _)| *n == id)
            .map(|(_, node)| node)
    }
}

/// A minimal declarative source parser for the Wave 1 Goya surface.
///
/// Grammar (line-oriented, indentation-free for Wave 1):
/// `column <id> <child-ids...>` | `row <id> <child-ids...>`
/// | `text <id> <string...>` | `button <id> <action> <label...>`
/// | `spacer <id> <size>` | `root <id>`
pub fn parse(source: &str) -> Result<Module, ParseError> {
    let mut module = Module::default();
    for (line_no, raw) in source.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split_whitespace();
        let kind = parts.next().ok_or(ParseError { line: line_no + 1 })?;
        let id = parts
            .next()
            .and_then(|s| s.parse::<u64>().ok())
            .map(NodeId)
            .ok_or(ParseError { line: line_no + 1 })?;
        let rest: Vec<&str> = parts.collect();
        let node = match kind {
            "column" => Node::Column {
                children: parse_ids(&rest).ok_or(ParseError { line: line_no + 1 })?,
            },
            "row" => Node::Row {
                children: parse_ids(&rest).ok_or(ParseError { line: line_no + 1 })?,
            },
            "text" => Node::Text {
                content: rest.join(" "),
            },
            "button" => {
                let action = rest
                    .first()
                    .and_then(|s| s.parse::<u64>().ok())
                    .map(ActionId)
                    .ok_or(ParseError { line: line_no + 1 })?;
                Node::Button {
                    label: rest[1..].join(" "),
                    on_activate: action,
                }
            }
            "spacer" => Node::Spacer {
                size: rest
                    .first()
                    .and_then(|s| s.parse::<f64>().ok())
                    .ok_or(ParseError { line: line_no + 1 })?,
            },
            "root" => {
                module.root = Some(id);
                continue;
            }
            _ => return Err(ParseError { line: line_no + 1 }),
        };
        module.nodes.push((id, node));
    }
    if module.root.is_none() {
        return Err(ParseError { line: 0 });
    }
    Ok(module)
}

fn parse_ids(parts: &[&str]) -> Option<Vec<NodeId>> {
    parts
        .iter()
        .map(|s| s.parse::<u64>().ok().map(NodeId))
        .collect()
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParseError {
    pub line: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    const HELLO: &str = "\
# goya-hello module
column 1 2 3
text 2 Hello, Goya!
button 3 1 Increment
root 1
";

    #[test]
    fn parses_hello_module() {
        let m = parse(HELLO).unwrap();
        assert_eq!(m.root, Some(NodeId(1)));
        assert_eq!(m.nodes.len(), 3);
        assert_eq!(
            m.node(NodeId(2)),
            Some(&Node::Text {
                content: "Hello, Goya!".into()
            })
        );
    }

    #[test]
    fn rejects_missing_root() {
        assert_eq!(parse("text 1 hi"), Err(ParseError { line: 0 }));
    }

    #[test]
    fn rejects_bad_line() {
        let src = "text 1 hi\nfrobnicate 2\nroot 1";
        assert_eq!(parse(src), Err(ParseError { line: 2 }));
    }
}
