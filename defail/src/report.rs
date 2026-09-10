//! The structured failure report: DEFAIL's answer to a bare exception string.

use std::fmt;

use crate::declare::FailureClass;
use crate::json;
use crate::trace::TraceEvent;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Disposition {
    /// A permitted remediation satisfied its constraints, the operation
    /// resumed, and verification held.
    Recovered,
    /// No permitted remediation satisfied the declared constraints.
    /// DEFAIL escalated instead of guessing.
    Escalated,
}

impl Disposition {
    pub fn as_str(&self) -> &'static str {
        match self {
            Disposition::Recovered => "recovered",
            Disposition::Escalated => "escalated",
        }
    }
}

impl fmt::Display for Disposition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One failure and its resolution, in structured, attributable form:
///
/// ```text
/// Failure: provider request rejected
/// Class: capacity/rate-limit
/// Evidence: http_status=429 + quota_header=present
/// Confidence: 0.98
/// Resolution: switch to provider B
/// Constraint: preserve capability generation at tier >= 3
/// Verification: provider B accepted an equivalent request
/// Disposition: recovered
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct FailureReport {
    pub failure: String,
    pub class: FailureClass,
    pub evidence: Vec<String>,
    pub confidence: f32,
    pub resolution: String,
    pub constraint: String,
    pub verification: String,
    pub disposition: Disposition,
    pub attempts: u32,
    /// Remediations that were considered and rejected, with reasons.
    pub rejected: Vec<String>,
    /// The ordered pipeline decisions that produced this report (see
    /// [`crate::trace`]). Events carry addresses and decision data only,
    /// never signal payloads.
    pub trace: Vec<TraceEvent>,
}

impl FailureReport {
    /// Render the report as versioned JSON (`"schema": "defail-report/1"`),
    /// including the full decision trace. Field order is stable, so the
    /// output is byte-for-byte deterministic.
    pub fn to_json(&self) -> String {
        json::object(&[
            ("schema", json::quote("defail-report/1")),
            ("failure", json::quote(&self.failure)),
            ("class", json::quote(self.class.as_str())),
            (
                "evidence",
                json::array(self.evidence.iter().map(|e| json::quote(e))),
            ),
            ("confidence", json::f32(self.confidence)),
            ("resolution", json::quote(&self.resolution)),
            ("constraint", json::quote(&self.constraint)),
            ("verification", json::quote(&self.verification)),
            ("disposition", json::quote(self.disposition.as_str())),
            ("attempts", self.attempts.to_string()),
            (
                "rejected",
                json::array(self.rejected.iter().map(|r| json::quote(r))),
            ),
            (
                "trace",
                json::array(self.trace.iter().map(TraceEvent::to_json)),
            ),
        ])
    }
}

impl fmt::Display for FailureReport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "Failure: {}", self.failure)?;
        writeln!(f, "Class: {}", self.class)?;
        writeln!(f, "Evidence: {}", self.evidence.join(" + "))?;
        writeln!(f, "Confidence: {:.2}", self.confidence)?;
        writeln!(f, "Resolution: {}", self.resolution)?;
        writeln!(f, "Constraint: {}", self.constraint)?;
        writeln!(f, "Verification: {}", self.verification)?;
        if !self.rejected.is_empty() {
            writeln!(f, "Rejected: {}", self.rejected.join("; "))?;
        }
        write!(f, "Disposition: {}", self.disposition.as_str())
    }
}
