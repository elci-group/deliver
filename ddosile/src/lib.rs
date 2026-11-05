//! DDOSILE's initial, deterministic policy model.
//!
//! This crate does not open sockets or connect to production. It provides a
//! small typed foundation for validating routing policy and modeling the
//! reservation-before-execution rule.

use std::collections::BTreeSet;

pub mod admission;
pub mod resources;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RequestClass {
    PublicReadOnly,
    AuthenticatedReadOnly,
    Consequential,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SyntheticMode {
    Forbidden,
    StaticOnly,
    BoundedStateful,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Uncertainty {
    Challenge,
    Reject,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RoutePolicy {
    pub path_prefix: String,
    pub method: String,
    pub class: RequestClass,
    pub synthetic: SyntheticMode,
    pub uncertainty: Uncertainty,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Policy {
    pub epoch: u64,
    pub origin_public_access: bool,
    pub production_reserve_percent: u8,
    pub synthetic_capacity_percent: u8,
    pub operational_headroom_percent: u8,
    pub routes: Vec<RoutePolicy>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PolicyError {
    EmptyRoutePrefix,
    DuplicateRoutePrefix(String),
    CapacitySharesExceedLimit,
    PublicOriginEnabled,
    ConsequentialSyntheticAllowed(String),
    AuthenticatedSyntheticAllowed(String),
    InvalidPathPrefix(String),
    InvalidMethod(String),
}

impl Policy {
    /// Validate invariants that can be checked without deployment context.
    pub fn validate(&self) -> Result<(), Vec<PolicyError>> {
        let mut errors = Vec::new();
        if self.origin_public_access {
            errors.push(PolicyError::PublicOriginEnabled);
        }
        if u16::from(self.production_reserve_percent)
            + u16::from(self.synthetic_capacity_percent)
            + u16::from(self.operational_headroom_percent)
            > 100
        {
            errors.push(PolicyError::CapacitySharesExceedLimit);
        }
        let mut prefixes = BTreeSet::new();
        for route in &self.routes {
            if route.path_prefix.trim().is_empty() {
                errors.push(PolicyError::EmptyRoutePrefix);
            } else if !prefixes.insert((route.path_prefix.as_str(), route.method.as_str())) {
                errors.push(PolicyError::DuplicateRoutePrefix(format!(
                    "{} {}",
                    route.method, route.path_prefix
                )));
            }
            if route.class == RequestClass::Consequential
                && route.synthetic != SyntheticMode::Forbidden
            {
                errors.push(PolicyError::ConsequentialSyntheticAllowed(
                    route.path_prefix.clone(),
                ));
            }
            if route.class == RequestClass::AuthenticatedReadOnly
                && route.synthetic != SyntheticMode::Forbidden
            {
                errors.push(PolicyError::AuthenticatedSyntheticAllowed(
                    route.path_prefix.clone(),
                ));
            }
            if !route.path_prefix.starts_with('/') || route.path_prefix.contains("..") {
                errors.push(PolicyError::InvalidPathPrefix(route.path_prefix.clone()));
            }
            if route.method.is_empty()
                || !route.method.bytes().all(|byte| byte.is_ascii_uppercase())
            {
                errors.push(PolicyError::InvalidMethod(route.method.clone()));
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

/// The validated, deterministically ordered representation used by the runtime.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompiledPolicy {
    policy: Policy,
}

impl CompiledPolicy {
    pub fn policy(&self) -> &Policy {
        &self.policy
    }

    /// Longest matching prefix wins; equal prefixes are ordered by method.
    pub fn route_for(&self, method: &str, path: &str) -> Option<&RoutePolicy> {
        let method = method.to_ascii_uppercase();
        self.policy
            .routes
            .iter()
            .find(|route| route.method == method && path.starts_with(&route.path_prefix))
    }

    /// Stable representation suitable for review and hashing by a later signer.
    pub fn canonical_text(&self) -> String {
        let p = &self.policy;
        let mut out = format!(
            "ddosile=\"0.0.1\"\nepoch={}\norigin_public_access=false\nresources={},{},{}\n",
            p.epoch,
            p.production_reserve_percent,
            p.synthetic_capacity_percent,
            p.operational_headroom_percent
        );
        for route in &p.routes {
            out.push_str(&format!(
                "route={:?}|{}|{}|{}|{}\n",
                route.path_prefix,
                route.method,
                class_name(route.class),
                synthetic_name(route.synthetic),
                uncertainty_name(route.uncertainty)
            ));
        }
        out
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompileError(pub String);

/// Parse the supported TOML policy subset and reject unknown keys.
pub fn compile_policy(source: &str) -> Result<CompiledPolicy, Vec<CompileError>> {
    let value: toml::Value = source
        .parse()
        .map_err(|error: toml::de::Error| vec![CompileError(error.to_string())])?;
    let mut errors = Vec::new();
    check_keys(
        &value,
        &["ddosile", "epoch", "architecture", "resources", "routes"],
        "root",
        &mut errors,
    );
    let version = string_at(&value, "ddosile", "root", &mut errors);
    if version.as_deref() != Some("0.0.1") {
        errors.push(CompileError("root.ddosile must be \"0.0.1\"".into()));
    }
    let epoch = integer_at(&value, "epoch", "root", &mut errors).and_then(|n| {
        u64::try_from(n)
            .map_err(|_| {
                errors.push(CompileError(
                    "root.epoch must be a non-negative integer".into(),
                ))
            })
            .ok()
    });
    let architecture = table_at(&value, "architecture", "root", &mut errors);
    let mut origin_public_access = true;
    if let Some(table) = architecture {
        check_keys_table(
            table,
            &["mode", "origin_public_access"],
            "architecture",
            &mut errors,
        );
        let mode_valid =
            table.get("mode").and_then(toml::Value::as_str) == Some("permanent_facade");
        if !mode_valid {
            errors.push(CompileError(
                "architecture.mode must be \"permanent_facade\"".into(),
            ));
        }
        origin_public_access = table
            .get("origin_public_access")
            .and_then(toml::Value::as_str)
            != Some("forbidden");
        if origin_public_access {
            errors.push(CompileError(
                "architecture.origin_public_access must be \"forbidden\"".into(),
            ));
        }
    }
    let resources = table_at(&value, "resources", "root", &mut errors);
    let (mut prod, mut synth, mut headroom) = (None, None, None);
    if let Some(table) = resources {
        check_keys_table(
            table,
            &[
                "production_reserve_percent",
                "synthetic_capacity_percent",
                "operational_headroom_percent",
            ],
            "resources",
            &mut errors,
        );
        prod = percent(table, "production_reserve_percent", &mut errors);
        synth = percent(table, "synthetic_capacity_percent", &mut errors);
        headroom = percent(table, "operational_headroom_percent", &mut errors);
    }
    let mut routes = Vec::new();
    match value.get("routes").and_then(toml::Value::as_array) {
        Some(items) => {
            for (index, item) in items.iter().enumerate() {
                let label = format!("routes[{index}]");
                let Some(table) = item.as_table() else {
                    errors.push(CompileError(format!("{label} must be a table")));
                    continue;
                };
                check_keys_table(
                    table,
                    &["path", "method", "class", "synthetic", "uncertainty"],
                    &label,
                    &mut errors,
                );
                let path_prefix = field_string(table, "path", &label, &mut errors);
                let method = field_string(table, "method", &label, &mut errors)
                    .map(|m| m.to_ascii_uppercase());
                let class = field_string(table, "class", &label, &mut errors)
                    .and_then(|v| enum_value(parse_class(&v), "class", &v, &label, &mut errors));
                let synthetic =
                    field_string(table, "synthetic", &label, &mut errors).and_then(|v| {
                        enum_value(parse_synthetic(&v), "synthetic", &v, &label, &mut errors)
                    });
                let uncertainty =
                    field_string(table, "uncertainty", &label, &mut errors).and_then(|v| {
                        enum_value(
                            parse_uncertainty(&v),
                            "uncertainty",
                            &v,
                            &label,
                            &mut errors,
                        )
                    });
                if let (
                    Some(path_prefix),
                    Some(method),
                    Some(class),
                    Some(synthetic),
                    Some(uncertainty),
                ) = (path_prefix, method, class, synthetic, uncertainty)
                {
                    routes.push(RoutePolicy {
                        path_prefix,
                        method,
                        class,
                        synthetic,
                        uncertainty,
                    });
                }
            }
        }
        None => errors.push(CompileError(
            "root.routes must be an array of tables".into(),
        )),
    }
    if routes.is_empty() {
        errors.push(CompileError(
            "root.routes must contain at least one valid route".into(),
        ));
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    let mut policy = Policy {
        epoch: epoch.expect("validated epoch"),
        origin_public_access,
        production_reserve_percent: prod.expect("validated reserve"),
        synthetic_capacity_percent: synth.expect("validated synthetic capacity"),
        operational_headroom_percent: headroom.expect("validated headroom"),
        routes,
    };
    policy.routes.sort_by(|a, b| {
        b.path_prefix
            .len()
            .cmp(&a.path_prefix.len())
            .then_with(|| a.path_prefix.cmp(&b.path_prefix))
            .then_with(|| a.method.cmp(&b.method))
    });
    if let Err(policy_errors) = policy.validate() {
        return Err(policy_errors
            .into_iter()
            .map(|error| CompileError(format!("policy invariant: {error:?}")))
            .collect());
    }
    Ok(CompiledPolicy { policy })
}

fn check_keys(value: &toml::Value, allowed: &[&str], prefix: &str, errors: &mut Vec<CompileError>) {
    if let Some(table) = value.as_table() {
        check_keys_table(table, allowed, prefix, errors);
    }
}
fn check_keys_table(
    table: &toml::map::Map<String, toml::Value>,
    allowed: &[&str],
    prefix: &str,
    errors: &mut Vec<CompileError>,
) {
    for key in table.keys() {
        if !allowed.contains(&key.as_str()) {
            errors.push(CompileError(format!("unknown key {prefix}.{key}")));
        }
    }
}
fn string_at(
    value: &toml::Value,
    key: &str,
    prefix: &str,
    errors: &mut Vec<CompileError>,
) -> Option<String> {
    value
        .get(key)
        .and_then(toml::Value::as_str)
        .map(str::to_owned)
        .or_else(|| {
            errors.push(CompileError(format!("{prefix}.{key} must be a string")));
            None
        })
}
fn integer_at(
    value: &toml::Value,
    key: &str,
    prefix: &str,
    errors: &mut Vec<CompileError>,
) -> Option<i64> {
    value
        .get(key)
        .and_then(toml::Value::as_integer)
        .or_else(|| {
            errors.push(CompileError(format!("{prefix}.{key} must be an integer")));
            None
        })
}
fn table_at<'a>(
    value: &'a toml::Value,
    key: &str,
    prefix: &str,
    errors: &mut Vec<CompileError>,
) -> Option<&'a toml::map::Map<String, toml::Value>> {
    value.get(key).and_then(toml::Value::as_table).or_else(|| {
        errors.push(CompileError(format!("{prefix}.{key} must be a table")));
        None
    })
}
fn field_string(
    table: &toml::map::Map<String, toml::Value>,
    key: &str,
    prefix: &str,
    errors: &mut Vec<CompileError>,
) -> Option<String> {
    table
        .get(key)
        .and_then(toml::Value::as_str)
        .map(str::to_owned)
        .or_else(|| {
            errors.push(CompileError(format!("{prefix}.{key} must be a string")));
            None
        })
}
fn enum_value<T>(
    value: Option<T>,
    field: &str,
    actual: &str,
    prefix: &str,
    errors: &mut Vec<CompileError>,
) -> Option<T> {
    value.or_else(|| {
        errors.push(CompileError(format!(
            "{prefix}.{field} has unsupported value {actual:?}"
        )));
        None
    })
}
fn percent(
    table: &toml::map::Map<String, toml::Value>,
    key: &str,
    errors: &mut Vec<CompileError>,
) -> Option<u8> {
    match table.get(key).and_then(toml::Value::as_integer) {
        Some(n @ 0..=100) => Some(n as u8),
        _ => {
            errors.push(CompileError(format!(
                "resources.{key} must be an integer from 0 to 100"
            )));
            None
        }
    }
}
fn parse_class(value: &str) -> Option<RequestClass> {
    match value {
        "public_read" => Some(RequestClass::PublicReadOnly),
        "authenticated" => Some(RequestClass::AuthenticatedReadOnly),
        "consequential" => Some(RequestClass::Consequential),
        _ => None,
    }
}
fn parse_synthetic(value: &str) -> Option<SyntheticMode> {
    match value {
        "forbidden" => Some(SyntheticMode::Forbidden),
        "static_only" => Some(SyntheticMode::StaticOnly),
        "bounded_stateful" => Some(SyntheticMode::BoundedStateful),
        _ => None,
    }
}
fn parse_uncertainty(value: &str) -> Option<Uncertainty> {
    match value {
        "challenge" => Some(Uncertainty::Challenge),
        "reject" => Some(Uncertainty::Reject),
        _ => None,
    }
}
fn class_name(value: RequestClass) -> &'static str {
    match value {
        RequestClass::PublicReadOnly => "public_read",
        RequestClass::AuthenticatedReadOnly => "authenticated",
        RequestClass::Consequential => "consequential",
    }
}
fn synthetic_name(value: SyntheticMode) -> &'static str {
    match value {
        SyntheticMode::Forbidden => "forbidden",
        SyntheticMode::StaticOnly => "static_only",
        SyntheticMode::BoundedStateful => "bounded_stateful",
    }
}
fn uncertainty_name(value: Uncertainty) -> &'static str {
    match value {
        Uncertainty::Challenge => "challenge",
        Uncertainty::Reject => "reject",
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExecutionClass {
    Production,
    Synthetic,
    Challenge,
    Reject,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RequestState {
    Received,
    Classified,
    Reserved,
    Admitted,
    Synthetic,
    Challenged,
    Rejected,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TransitionError {
    InvalidTransition {
        from: RequestState,
        to: RequestState,
    },
    ProductionGrantRequired,
    ProductionPermitMismatch,
    ReservationInvalid,
    SyntheticReservationRequired,
    ChallengeReservationRequired,
    SyntheticNotAllowed,
}

/// Request state model. Execution is impossible until `reserve` succeeds.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RequestFlow {
    pub class: RequestClass,
    pub state: RequestState,
    pub execution: Option<ExecutionClass>,
    request_id: Option<u128>,
    reservation_held: bool,
}

impl RequestFlow {
    pub fn new(class: RequestClass) -> Self {
        Self {
            class,
            state: RequestState::Received,
            execution: None,
            request_id: None,
            reservation_held: false,
        }
    }

    pub fn for_request(class: RequestClass, request_id: u128) -> Self {
        let mut flow = Self::new(class);
        flow.request_id = Some(request_id);
        flow
    }

    pub fn classify(&mut self) -> Result<(), TransitionError> {
        self.transition(RequestState::Received, RequestState::Classified)
    }

    pub fn reserve(&mut self) -> Result<(), TransitionError> {
        self.transition(RequestState::Classified, RequestState::Reserved)?;
        self.reservation_held = true;
        Ok(())
    }

    pub fn execute(&mut self, execution: ExecutionClass) -> Result<(), TransitionError> {
        if self.state != RequestState::Reserved || !self.reservation_held {
            return Err(TransitionError::InvalidTransition {
                from: self.state,
                to: state_for(execution),
            });
        }
        if execution == ExecutionClass::Production {
            return Err(TransitionError::ProductionGrantRequired);
        }
        if execution == ExecutionClass::Synthetic {
            return Err(TransitionError::SyntheticReservationRequired);
        }
        if execution == ExecutionClass::Challenge {
            return Err(TransitionError::ChallengeReservationRequired);
        }
        self.execution = Some(execution);
        self.state = state_for(execution);
        self.reservation_held = false;
        Ok(())
    }

    pub fn execute_synthetic(
        &mut self,
        reservation: &resources::Reservation,
    ) -> Result<(), TransitionError> {
        if self.class != RequestClass::PublicReadOnly {
            return Err(TransitionError::SyntheticNotAllowed);
        }
        self.require_synthetic_reservation(reservation)?;
        self.finish_reserved(ExecutionClass::Synthetic)
    }

    pub fn execute_challenge(
        &mut self,
        reservation: &resources::Reservation,
    ) -> Result<(), TransitionError> {
        self.require_synthetic_reservation(reservation)?;
        self.finish_reserved(ExecutionClass::Challenge)
    }

    pub fn execute_production(
        &mut self,
        permit: admission::ProductionPermit,
        reservation: &resources::Reservation,
    ) -> Result<(), TransitionError> {
        self.require_internal_reservation(RequestState::Admitted)?;
        if self.request_id != Some(permit.request_id())
            || reservation.id() != permit.reservation_id()
            || reservation.class() != resources::CapacityClass::Production
        {
            return Err(TransitionError::ProductionPermitMismatch);
        }
        if !reservation.is_active() {
            return Err(TransitionError::ReservationInvalid);
        }
        self.finish_reserved(ExecutionClass::Production)
    }

    pub fn reject(&mut self) {
        self.reservation_held = false;
        self.execution = Some(ExecutionClass::Reject);
        self.state = RequestState::Rejected;
    }

    fn require_synthetic_reservation(
        &self,
        reservation: &resources::Reservation,
    ) -> Result<(), TransitionError> {
        self.require_internal_reservation(state_for(ExecutionClass::Synthetic))?;
        if reservation.class() != resources::CapacityClass::Synthetic || !reservation.is_active() {
            return Err(TransitionError::ReservationInvalid);
        }
        Ok(())
    }

    fn require_internal_reservation(&self, to: RequestState) -> Result<(), TransitionError> {
        if self.state != RequestState::Reserved || !self.reservation_held {
            return Err(TransitionError::InvalidTransition {
                from: self.state,
                to,
            });
        }
        Ok(())
    }

    fn finish_reserved(&mut self, execution: ExecutionClass) -> Result<(), TransitionError> {
        self.require_internal_reservation(state_for(execution))?;
        self.execution = Some(execution);
        self.state = state_for(execution);
        self.reservation_held = false;
        Ok(())
    }

    fn transition(&mut self, from: RequestState, to: RequestState) -> Result<(), TransitionError> {
        if self.state != from {
            return Err(TransitionError::InvalidTransition {
                from: self.state,
                to,
            });
        }
        self.state = to;
        Ok(())
    }
}

fn state_for(execution: ExecutionClass) -> RequestState {
    match execution {
        ExecutionClass::Production => RequestState::Admitted,
        ExecutionClass::Synthetic => RequestState::Synthetic,
        ExecutionClass::Challenge => RequestState::Challenged,
        ExecutionClass::Reject => RequestState::Rejected,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_ledger() -> resources::ReservationLedger {
        let vector = |cpu| resources::ResourceVector {
            cpu_millis: cpu,
            memory_bytes: cpu * 100,
            connection_slots: cpu,
            downstream_slots: cpu,
        };
        resources::ReservationLedger::new(resources::ResourceBudget {
            total: vector(100),
            production: vector(60),
            synthetic: vector(20),
            headroom: vector(20),
            max_production_reservations: 60,
            max_synthetic_reservations: 20,
        })
        .unwrap()
    }

    fn policy() -> Policy {
        Policy {
            epoch: 1,
            origin_public_access: false,
            production_reserve_percent: 65,
            synthetic_capacity_percent: 15,
            operational_headroom_percent: 20,
            routes: vec![RoutePolicy {
                path_prefix: "/docs/".into(),
                method: "GET".into(),
                class: RequestClass::PublicReadOnly,
                synthetic: SyntheticMode::StaticOnly,
                uncertainty: Uncertainty::Challenge,
            }],
        }
    }

    #[test]
    fn valid_policy_observes_capacity_and_origin_invariants() {
        assert_eq!(policy().validate(), Ok(()));
    }

    #[test]
    fn consequential_synthetic_route_is_rejected() {
        let mut p = policy();
        p.routes[0].class = RequestClass::Consequential;
        assert!(p
            .validate()
            .unwrap_err()
            .contains(&PolicyError::ConsequentialSyntheticAllowed("/docs/".into())));
    }

    #[test]
    fn authenticated_routes_cannot_return_synthetic_data() {
        let mut p = policy();
        p.routes[0].class = RequestClass::AuthenticatedReadOnly;
        assert!(p
            .validate()
            .unwrap_err()
            .contains(&PolicyError::AuthenticatedSyntheticAllowed("/docs/".into())));
    }

    #[test]
    fn execution_requires_classification_and_reservation() {
        let mut flow = RequestFlow::new(RequestClass::PublicReadOnly);
        assert!(matches!(
            flow.execute(ExecutionClass::Production),
            Err(TransitionError::InvalidTransition { .. })
        ));
        flow.classify().unwrap();
        flow.reserve().unwrap();
        assert_eq!(
            flow.execute(ExecutionClass::Production),
            Err(TransitionError::ProductionGrantRequired)
        );
        assert_eq!(
            flow.execute(ExecutionClass::Synthetic),
            Err(TransitionError::SyntheticReservationRequired)
        );
        let ledger = test_ledger();
        let held = ledger
            .reserve(
                resources::CapacityClass::Synthetic,
                resources::ResourceVector {
                    cpu_millis: 1,
                    memory_bytes: 1,
                    connection_slots: 1,
                    downstream_slots: 1,
                },
            )
            .unwrap();
        flow.execute_synthetic(&held).unwrap();
        assert_eq!(flow.state, RequestState::Synthetic);
        assert!(!flow.reservation_held);
    }

    #[test]
    fn consequential_requests_cannot_complete_synthetically() {
        let mut flow = RequestFlow::new(RequestClass::Consequential);
        flow.classify().unwrap();
        flow.reserve().unwrap();
        let ledger = test_ledger();
        let held = ledger
            .reserve(
                resources::CapacityClass::Synthetic,
                resources::ResourceVector {
                    cpu_millis: 1,
                    memory_bytes: 1,
                    connection_slots: 1,
                    downstream_slots: 1,
                },
            )
            .unwrap();
        assert_eq!(
            flow.execute_synthetic(&held),
            Err(TransitionError::SyntheticNotAllowed)
        );
        flow.reject();
        assert_eq!(flow.state, RequestState::Rejected);
    }

    const SOURCE: &str = r#"
ddosile = "0.0.1"
epoch = 7
[architecture]
mode = "permanent_facade"
origin_public_access = "forbidden"
[resources]
production_reserve_percent = 65
synthetic_capacity_percent = 15
operational_headroom_percent = 20
[[routes]]
path = "/docs/"
method = "GET"
class = "public_read"
synthetic = "static_only"
uncertainty = "challenge"
[[routes]]
path = "/"
method = "GET"
class = "public_read"
synthetic = "forbidden"
uncertainty = "reject"
"#;

    #[test]
    fn compiler_orders_routes_by_longest_prefix_and_matches_method() {
        let compiled = compile_policy(SOURCE).unwrap();
        assert_eq!(
            compiled
                .route_for("get", "/docs/start")
                .unwrap()
                .path_prefix,
            "/docs/"
        );
        assert_eq!(compiled.route_for("POST", "/docs/start"), None);
        assert!(
            compiled.canonical_text().find("route=\"/docs/\"").unwrap()
                < compiled.canonical_text().find("route=\"/\"").unwrap()
        );
    }

    #[test]
    fn compiler_rejects_origin_access_and_unknown_keys() {
        let unsafe_policy = SOURCE
            .replace(
                "origin_public_access = \"forbidden\"",
                "origin_public_access = \"allowed\"",
            )
            .replace("epoch = 7", "epoch = 7\nignored = true");
        let errors = compile_policy(&unsafe_policy).unwrap_err();
        assert!(errors
            .iter()
            .any(|error| error.0.contains("origin_public_access")));
        assert!(errors.iter().any(|error| error.0.contains("unknown key")));
    }

    #[test]
    fn compiler_rejects_unsupported_route_values() {
        let invalid = SOURCE.replace("class = \"public_read\"", "class = \"unknown\"");
        assert!(compile_policy(&invalid)
            .unwrap_err()
            .iter()
            .any(|e| e.0.contains("unsupported value")));
    }

    #[test]
    fn compiler_rejects_synthetic_data_for_authenticated_routes() {
        let invalid = SOURCE.replace(
            "class = \"public_read\"\nsynthetic = \"static_only\"",
            "class = \"authenticated\"\nsynthetic = \"static_only\"",
        );
        assert!(compile_policy(&invalid)
            .unwrap_err()
            .iter()
            .any(|error| error.0.contains("AuthenticatedSyntheticAllowed")));
    }

    #[test]
    fn compiler_reports_negative_epoch_without_panicking() {
        let invalid = SOURCE.replace("epoch = 7", "epoch = -1");
        assert!(compile_policy(&invalid)
            .unwrap_err()
            .iter()
            .any(|error| error.0.contains("non-negative integer")));
    }
}
