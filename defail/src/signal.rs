//! Diagnostic signals: what a component can observe at a point of failure.

use std::fmt;

use crate::address::OpAddress;

/// A single observable value, e.g. `http_status = 429`.
#[derive(Debug, Clone, PartialEq)]
pub enum SignalValue {
    Text(String),
    Int(i64),
    Bool(bool),
}

impl SignalValue {
    pub fn text(s: impl Into<String>) -> Self {
        SignalValue::Text(s.into())
    }

    pub fn int(i: i64) -> Self {
        SignalValue::Int(i)
    }

    pub fn matches(&self, other: &SignalValue) -> bool {
        self == other
    }
}

impl fmt::Display for SignalValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SignalValue::Text(s) => f.write_str(s),
            SignalValue::Int(i) => write!(f, "{i}"),
            SignalValue::Bool(b) => write!(f, "{b}"),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Signal {
    pub key: String,
    pub value: SignalValue,
}

impl Signal {
    pub fn new(key: impl Into<String>, value: SignalValue) -> Self {
        Self {
            key: key.into(),
            value,
        }
    }
}

impl fmt::Display for Signal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}={}", self.key, self.value)
    }
}

/// A failure observation captured at an address in the execution graph.
#[derive(Debug, Clone)]
pub struct Observation {
    pub at: OpAddress,
    pub summary: String,
    pub signals: Vec<Signal>,
}

impl Observation {
    pub fn new(at: OpAddress, summary: impl Into<String>, signals: Vec<Signal>) -> Self {
        Self {
            at,
            summary: summary.into(),
            signals,
        }
    }

    pub fn signal(&self, key: &str) -> Option<&Signal> {
        self.signals.iter().find(|s| s.key == key)
    }
}

/// A declared diagnostic pattern: the signal a failure mode recognises.
///
/// A pattern with a value matches that value exactly; a pattern without one
/// matches any value for the key (the signal's presence is the evidence).
#[derive(Debug, Clone)]
pub struct SignalPattern {
    pub key: String,
    pub value: Option<SignalValue>,
    pub weight: f32,
}

impl SignalPattern {
    pub fn exact(key: impl Into<String>, value: SignalValue, weight: f32) -> Self {
        Self {
            key: key.into(),
            value: Some(value),
            weight,
        }
    }

    pub fn any(key: impl Into<String>, weight: f32) -> Self {
        Self {
            key: key.into(),
            value: None,
            weight,
        }
    }

    pub fn matches(&self, obs: &Observation) -> bool {
        match obs.signal(&self.key) {
            Some(signal) => match &self.value {
                Some(v) => v.matches(&signal.value),
                None => true,
            },
            None => false,
        }
    }
}
