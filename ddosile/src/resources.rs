//! Atomic, in-process resource reservations with disjoint production and
//! synthetic pools. A reservation is released when its token is dropped.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};

static NEXT_RESERVATION_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ResourceVector {
    pub cpu_millis: u64,
    pub memory_bytes: u64,
    pub connection_slots: u64,
    pub downstream_slots: u64,
}

impl ResourceVector {
    fn checked_add(self, other: Self) -> Option<Self> {
        Some(Self {
            cpu_millis: self.cpu_millis.checked_add(other.cpu_millis)?,
            memory_bytes: self.memory_bytes.checked_add(other.memory_bytes)?,
            connection_slots: self.connection_slots.checked_add(other.connection_slots)?,
            downstream_slots: self.downstream_slots.checked_add(other.downstream_slots)?,
        })
    }

    fn fits(self, limit: Self) -> bool {
        self.cpu_millis <= limit.cpu_millis
            && self.memory_bytes <= limit.memory_bytes
            && self.connection_slots <= limit.connection_slots
            && self.downstream_slots <= limit.downstream_slots
    }

    fn checked_sub(self, other: Self) -> Option<Self> {
        Some(Self {
            cpu_millis: self.cpu_millis.checked_sub(other.cpu_millis)?,
            memory_bytes: self.memory_bytes.checked_sub(other.memory_bytes)?,
            connection_slots: self.connection_slots.checked_sub(other.connection_slots)?,
            downstream_slots: self.downstream_slots.checked_sub(other.downstream_slots)?,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResourceBudget {
    pub total: ResourceVector,
    pub production: ResourceVector,
    pub synthetic: ResourceVector,
    pub headroom: ResourceVector,
    pub max_production_reservations: usize,
    pub max_synthetic_reservations: usize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CapacityClass {
    Production,
    Synthetic,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReservationError {
    BudgetExceedsTotal,
    CapacityExhausted(CapacityClass),
    ZeroReservation,
    ReservationLedgerFull(CapacityClass),
    ArithmeticOverflow,
    LedgerUnavailable,
    ReservationIdsExhausted,
}

#[derive(Debug)]
struct LedgerState {
    production_used: ResourceVector,
    synthetic_used: ResourceVector,
    production_count: usize,
    synthetic_count: usize,
    active: HashMap<u64, (CapacityClass, ResourceVector)>,
}

#[derive(Clone, Debug)]
pub struct ReservationLedger {
    budget: ResourceBudget,
    state: Arc<Mutex<LedgerState>>,
}

impl ReservationLedger {
    pub fn new(budget: ResourceBudget) -> Result<Self, ReservationError> {
        let allocated = budget
            .production
            .checked_add(budget.synthetic)
            .and_then(|sum| sum.checked_add(budget.headroom))
            .ok_or(ReservationError::ArithmeticOverflow)?;
        if !allocated.fits(budget.total) {
            return Err(ReservationError::BudgetExceedsTotal);
        }
        Ok(Self {
            budget,
            state: Arc::new(Mutex::new(LedgerState {
                production_used: ResourceVector::default(),
                synthetic_used: ResourceVector::default(),
                production_count: 0,
                synthetic_count: 0,
                active: HashMap::new(),
            })),
        })
    }

    pub fn reserve(
        &self,
        class: CapacityClass,
        amount: ResourceVector,
    ) -> Result<Reservation, ReservationError> {
        if amount == ResourceVector::default() {
            return Err(ReservationError::ZeroReservation);
        }
        let mut state = self
            .state
            .lock()
            .map_err(|_| ReservationError::LedgerUnavailable)?;
        let (used, limit) = match class {
            CapacityClass::Production => (state.production_used, self.budget.production),
            CapacityClass::Synthetic => (state.synthetic_used, self.budget.synthetic),
        };
        let (count, count_limit) = match class {
            CapacityClass::Production => (
                state.production_count,
                self.budget.max_production_reservations,
            ),
            CapacityClass::Synthetic => (
                state.synthetic_count,
                self.budget.max_synthetic_reservations,
            ),
        };
        if count >= count_limit {
            return Err(ReservationError::ReservationLedgerFull(class));
        }
        let next = used
            .checked_add(amount)
            .ok_or(ReservationError::ArithmeticOverflow)?;
        if !next.fits(limit) {
            return Err(ReservationError::CapacityExhausted(class));
        }
        let id = NEXT_RESERVATION_ID
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .map_err(|_| ReservationError::ReservationIdsExhausted)?;
        match class {
            CapacityClass::Production => {
                state.production_used = next;
                state.production_count += 1;
            }
            CapacityClass::Synthetic => {
                state.synthetic_used = next;
                state.synthetic_count += 1;
            }
        }
        state.active.insert(id, (class, amount));
        Ok(Reservation {
            id,
            class,
            amount,
            ledger: Arc::downgrade(&self.state),
        })
    }

    pub fn active_reservations(&self) -> Result<usize, ReservationError> {
        self.state
            .lock()
            .map(|state| state.active.len())
            .map_err(|_| ReservationError::LedgerUnavailable)
    }

    pub fn is_active(&self, id: u64) -> Result<bool, ReservationError> {
        self.state
            .lock()
            .map(|state| state.active.contains_key(&id))
            .map_err(|_| ReservationError::LedgerUnavailable)
    }
}

#[derive(Debug)]
pub struct Reservation {
    id: u64,
    class: CapacityClass,
    amount: ResourceVector,
    ledger: Weak<Mutex<LedgerState>>,
}

impl Reservation {
    pub fn id(&self) -> u64 {
        self.id
    }
    pub fn class(&self) -> CapacityClass {
        self.class
    }
    pub fn amount(&self) -> ResourceVector {
        self.amount
    }
    pub fn is_active(&self) -> bool {
        self.ledger
            .upgrade()
            .and_then(|ledger| {
                ledger
                    .lock()
                    .ok()
                    .map(|state| state.active.contains_key(&self.id))
            })
            .unwrap_or(false)
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        let Some(ledger) = self.ledger.upgrade() else {
            return;
        };
        let Ok(mut state) = ledger.lock() else {
            return;
        };
        let Some((class, amount)) = state.active.remove(&self.id) else {
            return;
        };
        match class {
            CapacityClass::Production => {
                if let Some(used) = state.production_used.checked_sub(amount) {
                    state.production_used = used;
                }
                state.production_count = state.production_count.saturating_sub(1);
            }
            CapacityClass::Synthetic => {
                if let Some(used) = state.synthetic_used.checked_sub(amount) {
                    state.synthetic_used = used;
                }
                state.synthetic_count = state.synthetic_count.saturating_sub(1);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn vector(cpu: u64) -> ResourceVector {
        ResourceVector {
            cpu_millis: cpu,
            memory_bytes: cpu * 100,
            connection_slots: cpu,
            downstream_slots: cpu,
        }
    }
    fn ledger() -> ReservationLedger {
        ReservationLedger::new(ResourceBudget {
            total: vector(100),
            production: vector(60),
            synthetic: vector(20),
            headroom: vector(20),
            max_production_reservations: 60,
            max_synthetic_reservations: 20,
        })
        .unwrap()
    }

    #[test]
    fn pools_are_independent_and_bounded() {
        let ledger = ledger();
        let synthetic = ledger
            .reserve(CapacityClass::Synthetic, vector(20))
            .unwrap();
        assert_eq!(
            ledger
                .reserve(CapacityClass::Synthetic, vector(1))
                .unwrap_err(),
            ReservationError::CapacityExhausted(CapacityClass::Synthetic)
        );
        let production = ledger
            .reserve(CapacityClass::Production, vector(60))
            .unwrap();
        drop(synthetic);
        assert_eq!(ledger.active_reservations().unwrap(), 1);
        drop(production);
        assert!(ledger.reserve(CapacityClass::Synthetic, vector(20)).is_ok());
    }

    #[test]
    fn concurrent_reservations_never_exceed_pool_capacity() {
        let ledger = ledger();
        let mut workers = Vec::new();
        for _ in 0..100 {
            let ledger = ledger.clone();
            workers.push(std::thread::spawn(move || {
                ledger.reserve(CapacityClass::Synthetic, vector(1)).ok()
            }));
        }
        let reservations: Vec<_> = workers
            .into_iter()
            .filter_map(|worker| worker.join().unwrap())
            .collect();
        assert_eq!(reservations.len(), 20);
        assert_eq!(ledger.active_reservations().unwrap(), 20);
    }

    #[test]
    fn budget_must_fit_each_dimension() {
        let invalid = ResourceBudget {
            total: vector(100),
            production: vector(61),
            synthetic: vector(20),
            headroom: vector(20),
            max_production_reservations: 60,
            max_synthetic_reservations: 20,
        };
        assert_eq!(
            ReservationLedger::new(invalid).unwrap_err(),
            ReservationError::BudgetExceedsTotal
        );
    }

    #[test]
    fn synthetic_reservation_count_is_bounded() {
        let mut budget = ledger().budget;
        budget.max_synthetic_reservations = 1;
        let ledger = ReservationLedger::new(budget).unwrap();
        let held = ledger.reserve(CapacityClass::Synthetic, vector(1)).unwrap();
        assert_eq!(
            ledger
                .reserve(CapacityClass::Synthetic, vector(1))
                .unwrap_err(),
            ReservationError::ReservationLedgerFull(CapacityClass::Synthetic)
        );
        drop(held);
        assert!(ledger.reserve(CapacityClass::Synthetic, vector(1)).is_ok());
    }
}
