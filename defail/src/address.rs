//! Addressable points in the execution graph.

use std::fmt;
use std::str::FromStr;

/// Address of an operation in the execution graph, e.g. `Baker.recipe.combine`.
///
/// Because every significant operation has an address, DEFAIL can retrieve the
/// relevant local state, upstream state, constraints, and downstream
/// consequences at the point of failure without understanding the whole
/// application equally.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct OpAddress(String);

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BadAddress {
    Empty,
    EmptySegment(String),
    BadCharacter(String),
}

impl fmt::Display for BadAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BadAddress::Empty => write!(f, "address is empty"),
            BadAddress::EmptySegment(raw) => write!(f, "empty segment in `{raw}`"),
            BadAddress::BadCharacter(raw) => write!(
                f,
                "invalid character in `{raw}` (allowed: alphanumeric, `_`, `-`, `.`)"
            ),
        }
    }
}

impl std::error::Error for BadAddress {}

impl OpAddress {
    pub fn new(raw: &str) -> Result<Self, BadAddress> {
        if raw.is_empty() {
            return Err(BadAddress::Empty);
        }
        for segment in raw.split('.') {
            if segment.is_empty() {
                return Err(BadAddress::EmptySegment(raw.to_string()));
            }
            let ok = segment
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
            if !ok {
                return Err(BadAddress::BadCharacter(raw.to_string()));
            }
        }
        Ok(OpAddress(raw.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Root component name (the first segment).
    pub fn component(&self) -> &str {
        self.0.split('.').next().unwrap_or(self.0.as_str())
    }

    /// Parent address: `Baker.recipe.combine` → `Baker.recipe`.
    /// Returns `None` for a root-level address.
    pub fn parent(&self) -> Option<OpAddress> {
        self.0
            .rfind('.')
            .map(|idx| OpAddress(self.0[..idx].to_string()))
    }

    /// Extend the address by one segment.
    pub fn join(&self, segment: &str) -> Result<OpAddress, BadAddress> {
        Self::new(&format!("{}.{}", self.0, segment))
    }
}

impl fmt::Display for OpAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl FromStr for OpAddress {
    type Err = BadAddress;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_malformed_addresses() {
        assert_eq!(OpAddress::new(""), Err(BadAddress::Empty));
        assert!(matches!(
            OpAddress::new("a..b"),
            Err(BadAddress::EmptySegment(_))
        ));
        assert!(matches!(
            OpAddress::new("a b"),
            Err(BadAddress::BadCharacter(_))
        ));
    }

    #[test]
    fn navigates_the_graph() {
        let addr = OpAddress::new("Baker.recipe.combine").unwrap();
        assert_eq!(addr.component(), "Baker");
        assert_eq!(addr.parent().unwrap().as_str(), "Baker.recipe");
        assert_eq!(
            addr.join("bake").unwrap().as_str(),
            "Baker.recipe.combine.bake"
        );
        assert!(OpAddress::new("Baker").unwrap().parent().is_none());
    }

    #[test]
    fn bad_address_is_a_std_error() {
        let err: Box<dyn std::error::Error> = Box::new(BadAddress::Empty);
        assert!(!err.to_string().is_empty());
    }
}
