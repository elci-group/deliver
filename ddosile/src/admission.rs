//! Signed, short-lived admission grants for the broker boundary.
//!
//! `GrantIssuer` belongs in the trusted policy/broker domain. The facade gets
//! only a `BrokerVerifier` public key and cannot mint grants from it.

use crate::resources::{CapacityClass, Reservation};
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

const MAX_GRANT_TTL_SECS: u64 = 30;
const MAX_LIVE_GRANTS: usize = 100_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Authorization {
    Granted,
    Denied,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PolicyPermission {
    Allowed,
    Denied,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AdmissionRequest {
    pub request_id: u128,
    pub policy_epoch: u64,
    pub expires_at_unix: u64,
    pub authorization: Authorization,
    pub policy_permission: PolicyPermission,
    pub backend_healthy: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Claims {
    request_id: u128,
    policy_epoch: u64,
    expires_at_unix: u64,
    reservation_id: u64,
    authorization_granted: bool,
    policy_allowed: bool,
    backend_healthy: bool,
}

#[derive(Debug)]
pub struct AdmissionGrant {
    claims: Claims,
    signature: Signature,
}

const GRANT_CLAIMS_LEN: usize = 51;
const GRANT_SIGNATURE_LEN: usize = 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GrantError {
    InvalidPublicKey,
    NotProductionReservation,
    AuthorizationDenied,
    PolicyDenied,
    BackendUnhealthy,
    InvalidExpiry,
    InvalidSignature,
    StalePolicyEpoch,
    Expired,
    WrongRequest,
    ReservationMismatch,
    ReservationInactive,
    Replay,
    ReplayCapacity,
    ReplayLedgerUnavailable,
    MalformedGrant,
}

pub struct GrantIssuer {
    key: SigningKey,
    replay: Arc<Mutex<HashMap<u128, u64>>>,
}
#[derive(Clone)]
pub struct BrokerVerifier {
    key: VerifyingKey,
    replay: Arc<Mutex<HashMap<u128, u64>>>,
}

impl GrantIssuer {
    /// Construct from a secret seed supplied by the trusted control plane.
    /// Never store this seed in policy files or source control.
    pub fn from_secret_seed(seed: [u8; 32]) -> Self {
        Self {
            key: SigningKey::from_bytes(&seed),
            replay: Arc::new(Mutex::new(HashMap::new())),
        }
    }
    pub fn verifier(&self) -> BrokerVerifier {
        BrokerVerifier {
            key: self.key.verifying_key(),
            replay: Arc::clone(&self.replay),
        }
    }
    pub fn verifying_key_bytes(&self) -> [u8; 32] {
        self.key.verifying_key().to_bytes()
    }

    pub fn issue(
        &self,
        request: AdmissionRequest,
        reservation: &Reservation,
        now_unix: u64,
    ) -> Result<AdmissionGrant, GrantError> {
        if reservation.class() != CapacityClass::Production {
            return Err(GrantError::NotProductionReservation);
        }
        if !reservation.is_active() {
            return Err(GrantError::ReservationInactive);
        }
        if request.authorization != Authorization::Granted {
            return Err(GrantError::AuthorizationDenied);
        }
        if request.policy_permission != PolicyPermission::Allowed {
            return Err(GrantError::PolicyDenied);
        }
        if !request.backend_healthy {
            return Err(GrantError::BackendUnhealthy);
        }
        if request.expires_at_unix <= now_unix
            || request.expires_at_unix - now_unix > MAX_GRANT_TTL_SECS
        {
            return Err(GrantError::InvalidExpiry);
        }
        let claims = Claims {
            request_id: request.request_id,
            policy_epoch: request.policy_epoch,
            expires_at_unix: request.expires_at_unix,
            reservation_id: reservation.id(),
            authorization_granted: true,
            policy_allowed: true,
            backend_healthy: true,
        };
        let signature = self.key.sign(&claims.message());
        Ok(AdmissionGrant { claims, signature })
    }
}

impl BrokerVerifier {
    /// Construct inside the broker from the trusted issuer's public key.
    pub fn from_public_key(bytes: [u8; 32]) -> Result<Self, GrantError> {
        let key = VerifyingKey::from_bytes(&bytes).map_err(|_| GrantError::InvalidPublicKey)?;
        Ok(Self {
            key,
            replay: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    pub fn verify(
        &self,
        grant: &AdmissionGrant,
        current_epoch: u64,
        now_unix: u64,
        request_id: u128,
        reservation: &Reservation,
        backend_healthy_now: bool,
    ) -> Result<ProductionPermit, GrantError> {
        self.key
            .verify_strict(&grant.claims.message(), &grant.signature)
            .map_err(|_| GrantError::InvalidSignature)?;
        if grant.claims.policy_epoch != current_epoch {
            return Err(GrantError::StalePolicyEpoch);
        }
        if now_unix >= grant.claims.expires_at_unix {
            return Err(GrantError::Expired);
        }
        if grant.claims.request_id != request_id {
            return Err(GrantError::WrongRequest);
        }
        if reservation.id() != grant.claims.reservation_id {
            return Err(GrantError::ReservationMismatch);
        }
        if reservation.class() != CapacityClass::Production {
            return Err(GrantError::NotProductionReservation);
        }
        if !reservation.is_active() {
            return Err(GrantError::ReservationInactive);
        }
        if !grant.claims.authorization_granted {
            return Err(GrantError::AuthorizationDenied);
        }
        if !grant.claims.policy_allowed {
            return Err(GrantError::PolicyDenied);
        }
        if !grant.claims.backend_healthy || !backend_healthy_now {
            return Err(GrantError::BackendUnhealthy);
        }
        let replay_key = grant.claims.request_id;
        let mut replay = self
            .replay
            .lock()
            .map_err(|_| GrantError::ReplayLedgerUnavailable)?;
        replay.retain(|_, expiry| *expiry > now_unix);
        if replay.contains_key(&replay_key) {
            return Err(GrantError::Replay);
        }
        if replay.len() >= MAX_LIVE_GRANTS {
            return Err(GrantError::ReplayCapacity);
        }
        replay.insert(replay_key, grant.claims.expires_at_unix);
        Ok(ProductionPermit {
            request_id,
            policy_epoch: current_epoch,
            reservation_id: reservation.id(),
        })
    }
}

impl AdmissionGrant {
    /// Fixed-width wire encoding for transport between isolated services.
    pub fn to_bytes(&self) -> [u8; GRANT_CLAIMS_LEN + GRANT_SIGNATURE_LEN] {
        let mut token = [0u8; GRANT_CLAIMS_LEN + GRANT_SIGNATURE_LEN];
        token[..GRANT_CLAIMS_LEN].copy_from_slice(&self.claims.message());
        token[GRANT_CLAIMS_LEN..].copy_from_slice(&self.signature.to_bytes());
        token
    }

    pub fn from_bytes(token: &[u8]) -> Result<Self, GrantError> {
        if token.len() != GRANT_CLAIMS_LEN + GRANT_SIGNATURE_LEN || &token[..8] != b"DDOSILE1" {
            return Err(GrantError::MalformedGrant);
        }
        let flag = |byte: u8| match byte {
            0 => Some(false),
            1 => Some(true),
            _ => None,
        };
        let request_id = u128::from_be_bytes(
            token[8..24]
                .try_into()
                .map_err(|_| GrantError::MalformedGrant)?,
        );
        let policy_epoch = u64::from_be_bytes(
            token[24..32]
                .try_into()
                .map_err(|_| GrantError::MalformedGrant)?,
        );
        let expires_at_unix = u64::from_be_bytes(
            token[32..40]
                .try_into()
                .map_err(|_| GrantError::MalformedGrant)?,
        );
        let reservation_id = u64::from_be_bytes(
            token[40..48]
                .try_into()
                .map_err(|_| GrantError::MalformedGrant)?,
        );
        let claims = Claims {
            request_id,
            policy_epoch,
            expires_at_unix,
            reservation_id,
            authorization_granted: flag(token[48]).ok_or(GrantError::MalformedGrant)?,
            policy_allowed: flag(token[49]).ok_or(GrantError::MalformedGrant)?,
            backend_healthy: flag(token[50]).ok_or(GrantError::MalformedGrant)?,
        };
        let signature = Signature::from_slice(&token[GRANT_CLAIMS_LEN..])
            .map_err(|_| GrantError::MalformedGrant)?;
        Ok(Self { claims, signature })
    }
}

#[derive(Debug)]
pub struct ProductionPermit {
    request_id: u128,
    policy_epoch: u64,
    reservation_id: u64,
}
impl ProductionPermit {
    pub fn request_id(&self) -> u128 {
        self.request_id
    }
    pub fn policy_epoch(&self) -> u64 {
        self.policy_epoch
    }
    pub fn reservation_id(&self) -> u64 {
        self.reservation_id
    }
}

impl Claims {
    fn message(self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(8 + 16 + 8 * 3 + 3);
        bytes.extend_from_slice(b"DDOSILE1");
        bytes.extend_from_slice(&self.request_id.to_be_bytes());
        bytes.extend_from_slice(&self.policy_epoch.to_be_bytes());
        bytes.extend_from_slice(&self.expires_at_unix.to_be_bytes());
        bytes.extend_from_slice(&self.reservation_id.to_be_bytes());
        bytes.push(u8::from(self.authorization_granted));
        bytes.push(u8::from(self.policy_allowed));
        bytes.push(u8::from(self.backend_healthy));
        bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resources::{ReservationLedger, ResourceBudget, ResourceVector};

    fn setup() -> (GrantIssuer, ReservationLedger) {
        let issuer = GrantIssuer::from_secret_seed([42; 32]);
        let v = ResourceVector {
            cpu_millis: 100,
            memory_bytes: 1000,
            connection_slots: 10,
            downstream_slots: 5,
        };
        let budget = ResourceBudget {
            total: ResourceVector {
                cpu_millis: v.cpu_millis + 1,
                memory_bytes: v.memory_bytes + 1,
                connection_slots: v.connection_slots + 1,
                downstream_slots: v.downstream_slots + 1,
            },
            production: v,
            synthetic: ResourceVector {
                cpu_millis: 1,
                memory_bytes: 1,
                connection_slots: 1,
                downstream_slots: 1,
            },
            headroom: ResourceVector::default(),
            max_production_reservations: 100,
            max_synthetic_reservations: 1,
        };
        (issuer, ReservationLedger::new(budget).unwrap())
    }
    fn request() -> AdmissionRequest {
        AdmissionRequest {
            request_id: 9,
            policy_epoch: 3,
            expires_at_unix: 120,
            authorization: Authorization::Granted,
            policy_permission: PolicyPermission::Allowed,
            backend_healthy: true,
        }
    }
    fn amount() -> ResourceVector {
        ResourceVector {
            cpu_millis: 1,
            memory_bytes: 1,
            connection_slots: 1,
            downstream_slots: 1,
        }
    }

    #[test]
    fn broker_accepts_only_signed_current_live_grants_with_active_reservations() {
        let (issuer, ledger) = setup();
        let reservation = ledger.reserve(CapacityClass::Production, amount()).unwrap();
        let grant = issuer.issue(request(), &reservation, 100).unwrap();
        let wire = grant.to_bytes();
        let grant = AdmissionGrant::from_bytes(&wire).unwrap();
        let verifier = issuer.verifier();
        let permit = verifier
            .verify(&grant, 3, 101, 9, &reservation, true)
            .unwrap();
        assert_eq!(permit.request_id(), 9);
        assert_eq!(permit.reservation_id(), reservation.id());
        assert_eq!(
            verifier
                .verify(&grant, 3, 101, 9, &reservation, true)
                .unwrap_err(),
            GrantError::Replay
        );
    }

    #[test]
    fn altered_wire_grants_fail_signature_validation() {
        let (issuer, ledger) = setup();
        let reservation = ledger.reserve(CapacityClass::Production, amount()).unwrap();
        let grant = issuer.issue(request(), &reservation, 100).unwrap();
        let mut wire = grant.to_bytes();
        wire[24] ^= 1;
        let altered = AdmissionGrant::from_bytes(&wire).unwrap();
        assert_eq!(
            issuer
                .verifier()
                .verify(&altered, 3, 101, 9, &reservation, true)
                .unwrap_err(),
            GrantError::InvalidSignature
        );
    }

    #[test]
    fn request_flow_requires_broker_permit_for_production() {
        let (issuer, ledger) = setup();
        let reservation = ledger.reserve(CapacityClass::Production, amount()).unwrap();
        let grant = issuer.issue(request(), &reservation, 100).unwrap();
        let permit = issuer
            .verifier()
            .verify(&grant, 3, 101, 9, &reservation, true)
            .unwrap();
        let mut flow = crate::RequestFlow::for_request(crate::RequestClass::PublicReadOnly, 9);
        flow.classify().unwrap();
        flow.reserve().unwrap();
        assert_eq!(
            flow.execute(crate::ExecutionClass::Production),
            Err(crate::TransitionError::ProductionGrantRequired)
        );
        flow.execute_production(permit, &reservation).unwrap();
        assert_eq!(flow.state, crate::RequestState::Admitted);
    }

    #[test]
    fn stale_expired_wrong_request_and_unhealthy_grants_fail_closed() {
        let (issuer, ledger) = setup();
        let reservation = ledger.reserve(CapacityClass::Production, amount()).unwrap();
        let grant = issuer.issue(request(), &reservation, 100).unwrap();
        let verifier = issuer.verifier();
        assert_eq!(
            verifier
                .verify(&grant, 4, 101, 9, &reservation, true)
                .unwrap_err(),
            GrantError::StalePolicyEpoch
        );
        assert_eq!(
            verifier
                .verify(&grant, 3, 120, 9, &reservation, true)
                .unwrap_err(),
            GrantError::Expired
        );
        assert_eq!(
            verifier
                .verify(&grant, 3, 101, 10, &reservation, true)
                .unwrap_err(),
            GrantError::WrongRequest
        );
        assert_eq!(
            verifier
                .verify(&grant, 3, 101, 9, &reservation, false)
                .unwrap_err(),
            GrantError::BackendUnhealthy
        );
    }

    #[test]
    fn grant_issuer_requires_all_admission_conditions() {
        let (issuer, ledger) = setup();
        let reservation = ledger.reserve(CapacityClass::Production, amount()).unwrap();
        let denied = AdmissionRequest {
            authorization: Authorization::Denied,
            ..request()
        };
        assert_eq!(
            issuer.issue(denied, &reservation, 100).unwrap_err(),
            GrantError::AuthorizationDenied
        );
        let wrong_pool = ledger.reserve(CapacityClass::Synthetic, amount()).unwrap();
        assert_eq!(
            issuer.issue(request(), &wrong_pool, 100).unwrap_err(),
            GrantError::NotProductionReservation
        );
    }
}
