//! The addressable model of the application's execution state.

use std::collections::{BTreeMap, HashMap};

use crate::address::OpAddress;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpStatus {
    Pending,
    InProgress,
    Done,
    Failed,
}

/// One step of the declared plan. Steps execute in order; `requires` adds
/// explicit prerequisites beyond the preceding step.
#[derive(Debug, Clone)]
pub struct PlanStep {
    pub address: OpAddress,
    pub description: String,
    pub requires: Vec<OpAddress>,
}

impl PlanStep {
    pub fn new(address: OpAddress, description: impl Into<String>) -> Self {
        Self {
            address,
            description: description.into(),
            requires: Vec::new(),
        }
    }
}

/// Where the application is, what it has done, and what remains possible.
/// Addressable: state is queried by [`OpAddress`], so a failure at
/// `Baker.recipe.combine` can retrieve upstream state and downstream
/// consequences without understanding the whole application.
#[derive(Debug, Clone)]
pub struct ExecutionState {
    steps: Vec<PlanStep>,
    statuses: HashMap<OpAddress, OpStatus>,
    capabilities: BTreeMap<String, u32>,
    params: BTreeMap<String, String>,
    notes: Vec<String>,
}

impl ExecutionState {
    pub fn new(steps: Vec<PlanStep>) -> Self {
        let statuses = steps
            .iter()
            .map(|s| (s.address.clone(), OpStatus::Pending))
            .collect();
        Self {
            steps,
            statuses,
            capabilities: BTreeMap::new(),
            params: BTreeMap::new(),
            notes: Vec::new(),
        }
    }

    pub fn steps(&self) -> &[PlanStep] {
        &self.steps
    }

    pub fn addresses(&self) -> Vec<OpAddress> {
        self.steps.iter().map(|s| s.address.clone()).collect()
    }

    pub fn index(&self, address: &OpAddress) -> Option<usize> {
        self.steps.iter().position(|s| &s.address == address)
    }

    pub fn status(&self, address: &OpAddress) -> OpStatus {
        self.statuses
            .get(address)
            .copied()
            .unwrap_or(OpStatus::Pending)
    }

    pub fn mark(&mut self, address: &OpAddress, status: OpStatus) {
        if let Some(slot) = self.statuses.get_mut(address) {
            *slot = status;
        }
    }

    pub fn reached(&self, address: &OpAddress) -> bool {
        self.status(address) == OpStatus::Done
    }

    /// True if `a` comes after `b` in plan order.
    pub fn is_downstream(&self, a: &OpAddress, b: &OpAddress) -> bool {
        match (self.index(a), self.index(b)) {
            (Some(ia), Some(ib)) => ia > ib,
            _ => false,
        }
    }

    /// The last step marked Done; `None` before the first completion.
    pub fn last_done(&self) -> Option<&OpAddress> {
        self.steps
            .iter()
            .rev()
            .find(|s| self.statuses.get(&s.address) == Some(&OpStatus::Done))
            .map(|s| &s.address)
    }

    pub fn set_capability(&mut self, capability: &str, tier: u32) {
        self.capabilities.insert(capability.to_string(), tier);
    }

    pub fn capability_tier(&self, capability: &str) -> u32 {
        self.capabilities.get(capability).copied().unwrap_or(0)
    }

    pub fn set_param(&mut self, key: &str, value: impl Into<String>) {
        self.params.insert(key.to_string(), value.into());
    }

    pub fn param(&self, key: &str) -> Option<&str> {
        self.params.get(key).map(|s| s.as_str())
    }

    pub fn params(&self) -> &BTreeMap<String, String> {
        &self.params
    }

    pub fn note(&mut self, note: impl Into<String>) {
        self.notes.push(note.into());
    }

    pub fn notes(&self) -> &[String] {
        &self.notes
    }

    /// Deterministic context signature for the knowledge base: where in the
    /// plan the failure occurred, in terms of the last completed address.
    pub fn context_signature(&self, at: &OpAddress) -> String {
        let stage = self
            .last_done()
            .map(|a| a.as_str().to_string())
            .unwrap_or_else(|| "start".to_string());
        format!("after:{stage};at:{at}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn addr(s: &str) -> OpAddress {
        OpAddress::new(s).unwrap()
    }

    #[test]
    fn tracks_order_and_downstream() {
        let mut state = ExecutionState::new(vec![
            PlanStep::new(addr("app.mix"), "mix"),
            PlanStep::new(addr("app.bake"), "bake"),
        ]);
        assert!(state.is_downstream(&addr("app.bake"), &addr("app.mix")));
        assert!(!state.is_downstream(&addr("app.mix"), &addr("app.bake")));
        state.mark(&addr("app.mix"), OpStatus::Done);
        assert_eq!(state.last_done(), Some(&addr("app.mix")));
        assert_eq!(
            state.context_signature(&addr("app.bake")),
            "after:app.mix;at:app.bake"
        );
    }
}
