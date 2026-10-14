//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::HashSet,
    fmt::{Display, Formatter},
    str::FromStr,
};

use serde::{Deserialize, Serialize};
use tari_template_lib_types::{ComponentAddress, ResourceAddress};

/// A capability the wallet daemon recognises as granted by a caller or required
/// by a handler.
///
/// String grammar: `admin`, `webrtc`, or `<resource>:<action>[:<entity>]`.
/// Scope semantics: an unscoped grant (`None`) satisfies any required scope;
/// a scoped grant (`Some(addr)`) only satisfies an identical required scope.
/// Mutation actions (`Create`/`Update`/`Delete`) imply `Read` on the same
/// resource.
#[derive(Debug, Clone, Deserialize, Serialize, Hash, Eq, PartialEq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub enum Permission {
    Admin,
    Accounts(Crud, Option<ComponentAddress>),
    Keys(Crud),
    Transactions(Crud),
    TransactionRequests(TxRequestAction),
    /// Requests for the wallet to co-sign a transaction someone else seals.
    /// Approving one releases a signature, so `Approve` is honoured only for
    /// an interactive wallet session.
    SigningRequests(TxRequestAction),
    Transfer(Crud, Option<ComponentAddress>),
    Templates(Crud),
    Nfts(Crud, Option<ResourceAddress>),
    Confidential(Crud, Option<ComponentAddress>),
    StealthUtxos(Crud, Option<ComponentAddress>),
    Validators(Crud),
    Settings(Crud),
    AddressBook(Crud),
    Substates(ReadOnly),
    BurnProofs(ReadOnly),
    SwapPools(ReadOnly),
    Resources(ReadOnly),
    Webrtc,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, Hash, Eq, PartialEq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub enum Crud {
    Read,
    Create,
    Update,
    Delete,
}

/// Actions on a transaction request. Deliberately not [`Crud`]: approving is
/// not a mutation of the request, it is the act that authorises the value
/// transfer the request describes. Modelling it as `Update` would let any
/// grant carrying edit rights approve.
#[derive(Debug, Clone, Copy, Deserialize, Serialize, Hash, Eq, PartialEq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub enum TxRequestAction {
    Read,
    Create,
    Approve,
}

/// Marker scope for resources that only support reads. Kept as a one-variant
/// enum (rather than `()`) so a future read-side extension is a non-breaking
/// addition.
#[derive(Debug, Clone, Copy, Deserialize, Serialize, Hash, Eq, PartialEq)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub enum ReadOnly {
    Read,
}

#[derive(Debug, thiserror::Error)]
#[error("Invalid permission '{0}'")]
pub struct InvalidPermissionsFormat(String);

impl Permission {
    /// True if this granted permission satisfies the given required
    /// permission. `Admin` satisfies all; mutations imply `Read`; an
    /// unscoped grant satisfies any scope, a scoped grant matches only its
    /// own scope.
    pub fn satisfies(&self, required: &Permission) -> bool {
        if matches!(self, Permission::Admin) {
            return true;
        }
        match (self, required) {
            (Permission::Accounts(ga, gs), Permission::Accounts(ra, rs)) => {
                crud_satisfies(*ga, *ra) && scope_satisfies(gs, rs)
            },
            (Permission::Keys(ga), Permission::Keys(ra)) => crud_satisfies(*ga, *ra),
            (Permission::Transactions(ga), Permission::Transactions(ra)) => crud_satisfies(*ga, *ra),
            (Permission::TransactionRequests(ga), Permission::TransactionRequests(ra)) => {
                tx_request_action_satisfies(*ga, *ra)
            },
            (Permission::SigningRequests(ga), Permission::SigningRequests(ra)) => tx_request_action_satisfies(*ga, *ra),
            (Permission::Transfer(ga, gs), Permission::Transfer(ra, rs)) => {
                crud_satisfies(*ga, *ra) && scope_satisfies(gs, rs)
            },
            (Permission::Templates(ga), Permission::Templates(ra)) => crud_satisfies(*ga, *ra),
            (Permission::Nfts(ga, gs), Permission::Nfts(ra, rs)) => crud_satisfies(*ga, *ra) && scope_satisfies(gs, rs),
            (Permission::Confidential(ga, gs), Permission::Confidential(ra, rs)) => {
                crud_satisfies(*ga, *ra) && scope_satisfies(gs, rs)
            },
            (Permission::StealthUtxos(ga, gs), Permission::StealthUtxos(ra, rs)) => {
                crud_satisfies(*ga, *ra) && scope_satisfies(gs, rs)
            },
            (Permission::Validators(ga), Permission::Validators(ra)) => crud_satisfies(*ga, *ra),
            (Permission::Settings(ga), Permission::Settings(ra)) => crud_satisfies(*ga, *ra),
            (Permission::AddressBook(ga), Permission::AddressBook(ra)) => crud_satisfies(*ga, *ra),
            (Permission::Substates(_), Permission::Substates(_)) => true,
            (Permission::BurnProofs(_), Permission::BurnProofs(_)) => true,
            (Permission::SwapPools(_), Permission::SwapPools(_)) => true,
            (Permission::Resources(_), Permission::Resources(_)) => true,
            (Permission::Webrtc, Permission::Webrtc) => true,
            _ => false,
        }
    }
}

fn crud_satisfies(granted: Crud, required: Crud) -> bool {
    granted == required || (required == Crud::Read && matches!(granted, Crud::Create | Crud::Update | Crud::Delete))
}

/// Mirrors [`crud_satisfies`]: actions that do something imply `Read` on the
/// same resource, and no action implies any other. `Create` must never satisfy
/// `Approve` — that separation is what stops a tool approving the requests it
/// creates.
fn tx_request_action_satisfies(granted: TxRequestAction, required: TxRequestAction) -> bool {
    granted == required ||
        (required == TxRequestAction::Read && matches!(granted, TxRequestAction::Create | TxRequestAction::Approve))
}

fn scope_satisfies<T: PartialEq>(granted: &Option<T>, required: &Option<T>) -> bool {
    match (granted, required) {
        (None, _) => true,
        (Some(_), None) => false,
        (Some(g), Some(r)) => g == r,
    }
}

impl Display for TxRequestAction {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            TxRequestAction::Read => write!(f, "read"),
            TxRequestAction::Create => write!(f, "create"),
            TxRequestAction::Approve => write!(f, "approve"),
        }
    }
}

impl FromStr for TxRequestAction {
    type Err = InvalidPermissionsFormat;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "read" => Ok(TxRequestAction::Read),
            "create" => Ok(TxRequestAction::Create),
            "approve" => Ok(TxRequestAction::Approve),
            _ => Err(InvalidPermissionsFormat(format!("invalid action '{s}'"))),
        }
    }
}

impl Display for Crud {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Crud::Read => write!(f, "read"),
            Crud::Create => write!(f, "create"),
            Crud::Update => write!(f, "update"),
            Crud::Delete => write!(f, "delete"),
        }
    }
}

impl FromStr for Crud {
    type Err = InvalidPermissionsFormat;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "read" => Ok(Crud::Read),
            "create" => Ok(Crud::Create),
            "update" => Ok(Crud::Update),
            "delete" => Ok(Crud::Delete),
            _ => Err(InvalidPermissionsFormat(format!("invalid action '{s}'"))),
        }
    }
}

impl Display for Permission {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Permission::Admin => write!(f, "admin"),
            Permission::Webrtc => write!(f, "webrtc"),
            Permission::Accounts(a, s) => display_scoped(f, "accounts", *a, s),
            Permission::Keys(a) => write!(f, "keys:{a}"),
            Permission::Transactions(a) => write!(f, "transactions:{a}"),
            Permission::TransactionRequests(a) => write!(f, "transaction_requests:{a}"),
            Permission::SigningRequests(a) => write!(f, "signing_requests:{a}"),
            Permission::Transfer(a, s) => display_scoped(f, "transfer", *a, s),
            Permission::Templates(a) => write!(f, "templates:{a}"),
            Permission::Nfts(a, s) => display_scoped(f, "nfts", *a, s),
            Permission::Confidential(a, s) => display_scoped(f, "confidential", *a, s),
            Permission::StealthUtxos(a, s) => display_scoped(f, "stealth_utxos", *a, s),
            Permission::Validators(a) => write!(f, "validators:{a}"),
            Permission::Settings(a) => write!(f, "settings:{a}"),
            Permission::AddressBook(a) => write!(f, "address_book:{a}"),
            Permission::Substates(_) => write!(f, "substates:read"),
            Permission::BurnProofs(_) => write!(f, "burn_proofs:read"),
            Permission::SwapPools(_) => write!(f, "swap_pools:read"),
            Permission::Resources(_) => write!(f, "resources:read"),
        }
    }
}

fn display_scoped<E: Display>(
    f: &mut Formatter<'_>,
    resource: &str,
    action: Crud,
    scope: &Option<E>,
) -> std::fmt::Result {
    match scope {
        Some(e) => write!(f, "{resource}:{action}:{e}"),
        None => write!(f, "{resource}:{action}"),
    }
}

impl FromStr for Permission {
    type Err = InvalidPermissionsFormat;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.trim();
        match s {
            "admin" => return Ok(Permission::Admin),
            "webrtc" => return Ok(Permission::Webrtc),
            _ => {},
        }

        let mut parts = s.splitn(3, ':');
        let resource = parts.next().ok_or_else(|| InvalidPermissionsFormat(s.into()))?;
        let action_str = parts.next().ok_or_else(|| {
            InvalidPermissionsFormat(format!(
                "missing action in '{s}' (expected '<resource>:<action>[:<entity>]')"
            ))
        })?;
        let entity_str = parts.next();

        match resource {
            "accounts" => parse_scoped(action_str, entity_str, Permission::Accounts),
            "keys" => parse_unscoped(action_str, entity_str, Permission::Keys),
            "transactions" => parse_unscoped(action_str, entity_str, Permission::Transactions),
            "transaction_requests" => parse_unscoped(action_str, entity_str, Permission::TransactionRequests),
            "signing_requests" => parse_unscoped(action_str, entity_str, Permission::SigningRequests),
            "transfer" => parse_scoped(action_str, entity_str, Permission::Transfer),
            "templates" => parse_unscoped(action_str, entity_str, Permission::Templates),
            "nfts" => parse_scoped(action_str, entity_str, Permission::Nfts),
            "confidential" => parse_scoped(action_str, entity_str, Permission::Confidential),
            "stealth_utxos" => parse_scoped(action_str, entity_str, Permission::StealthUtxos),
            "validators" => parse_unscoped(action_str, entity_str, Permission::Validators),
            "settings" => parse_unscoped(action_str, entity_str, Permission::Settings),
            "address_book" => parse_unscoped(action_str, entity_str, Permission::AddressBook),
            "substates" => parse_read_only(action_str, entity_str, Permission::Substates),
            "burn_proofs" => parse_read_only(action_str, entity_str, Permission::BurnProofs),
            "swap_pools" => parse_read_only(action_str, entity_str, Permission::SwapPools),
            "resources" => parse_read_only(action_str, entity_str, Permission::Resources),
            other => Err(InvalidPermissionsFormat(format!("unknown resource '{other}'"))),
        }
    }
}

fn parse_unscoped<F, A>(
    action_str: &str,
    entity_str: Option<&str>,
    ctor: F,
) -> Result<Permission, InvalidPermissionsFormat>
where
    F: FnOnce(A) -> Permission,
    A: FromStr<Err = InvalidPermissionsFormat>,
{
    if let Some(e) = entity_str {
        return Err(InvalidPermissionsFormat(format!(
            "resource does not accept an entity scope (got ':{e}')"
        )));
    }
    Ok(ctor(action_str.parse()?))
}

fn parse_scoped<F, E>(
    action_str: &str,
    entity_str: Option<&str>,
    ctor: F,
) -> Result<Permission, InvalidPermissionsFormat>
where
    F: FnOnce(Crud, Option<E>) -> Permission,
    E: FromStr,
    <E as FromStr>::Err: Display,
{
    let action: Crud = action_str.parse()?;
    let entity = match entity_str {
        Some(e) => Some(E::from_str(e).map_err(|err| InvalidPermissionsFormat(format!("invalid scope '{e}': {err}")))?),
        None => None,
    };
    Ok(ctor(action, entity))
}

fn parse_read_only<F>(
    action_str: &str,
    entity_str: Option<&str>,
    ctor: F,
) -> Result<Permission, InvalidPermissionsFormat>
where
    F: FnOnce(ReadOnly) -> Permission,
{
    if action_str != "read" {
        return Err(InvalidPermissionsFormat(format!(
            "read-only resource only accepts ':read' (got '{action_str}')"
        )));
    }
    if let Some(e) = entity_str {
        return Err(InvalidPermissionsFormat(format!(
            "read-only resource does not accept an entity scope (got ':{e}')"
        )));
    }
    Ok(ctor(ReadOnly::Read))
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Permissions(HashSet<Permission>);

impl FromStr for Permissions {
    type Err = InvalidPermissionsFormat;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Permissions(
            s.split(',')
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .map(Permission::from_str)
                .collect::<Result<_, _>>()?,
        ))
    }
}

impl Permissions {
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Exact-membership check. Use [`Self::satisfies`] for the resource-aware
    /// matcher that handles Admin, write-implies-read, and scope coverage.
    pub fn has_permission(&self, permission: &Permission) -> bool {
        self.0.contains(permission)
    }

    /// True if any granted permission satisfies the given required
    /// permission per [`Permission::satisfies`].
    pub fn satisfies(&self, required: &Permission) -> bool {
        self.0.iter().any(|granted| granted.satisfies(required))
    }

    /// Check that every required permission is satisfied by some granted
    /// permission. Returns `Err(required)` with the first unsatisfied
    /// permission so the caller can surface a precise error.
    pub fn check(&self, required: &[Permission]) -> Result<(), Permission> {
        for perm in required {
            if !self.satisfies(perm) {
                return Err(perm.clone());
            }
        }
        Ok(())
    }

    pub fn iter(&self) -> impl Iterator<Item = &Permission> {
        self.0.iter()
    }

    pub fn into_vec(self) -> Vec<Permission> {
        self.0.into_iter().collect()
    }
}

impl TryFrom<&[String]> for Permissions {
    type Error = InvalidPermissionsFormat;

    fn try_from(value: &[String]) -> Result<Self, Self::Error> {
        let mut permissions = HashSet::with_capacity(value.len());
        for permission in value {
            permissions.insert(Permission::from_str(permission)?);
        }
        Ok(Permissions(permissions))
    }
}

impl From<Vec<Permission>> for Permissions {
    fn from(value: Vec<Permission>) -> Self {
        Permissions(value.into_iter().collect())
    }
}

impl FromIterator<Permission> for Permissions {
    fn from_iter<T: IntoIterator<Item = Permission>>(iter: T) -> Self {
        Permissions(iter.into_iter().collect())
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
pub struct Claims {
    pub permissions: Permissions,
    pub exp: u64,
    /// Set on a token minted by `webrtc.start` for a connected peer. A
    /// delegated token acts with its permissions on the peer's behalf but
    /// never counts as the wallet's own interactive session.
    #[serde(default)]
    pub delegated: bool,
    /// Random per `webrtc.start` call, so two connected peers are told apart:
    /// each may act only on the wallet locks its own session created.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delegation_id: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn component(seed: u8) -> ComponentAddress {
        ComponentAddress::new([seed; 32].into())
    }

    fn resource(seed: u8) -> ResourceAddress {
        ResourceAddress::new([seed; 32].into())
    }

    // ---------- FromStr / Display round-trip ----------

    #[track_caller]
    fn round_trip(p: Permission) {
        let s = p.to_string();
        let parsed: Permission = s.parse().unwrap_or_else(|e| panic!("re-parsing '{s}' failed: {e}"));
        assert_eq!(parsed, p, "round-trip mismatch for '{s}'");
    }

    #[test]
    fn round_trip_bare_capabilities() {
        round_trip(Permission::Admin);
        round_trip(Permission::Webrtc);
    }

    #[test]
    fn round_trip_crud_resources_all_actions_unscoped() {
        for action in [Crud::Read, Crud::Create, Crud::Update, Crud::Delete] {
            round_trip(Permission::Accounts(action, None));
            round_trip(Permission::Keys(action));
            round_trip(Permission::Transactions(action));
            round_trip(Permission::Transfer(action, None));
            round_trip(Permission::Templates(action));
            round_trip(Permission::Nfts(action, None));
            round_trip(Permission::Confidential(action, None));
            round_trip(Permission::StealthUtxos(action, None));
            round_trip(Permission::Validators(action));
            round_trip(Permission::Settings(action));
            round_trip(Permission::AddressBook(action));
        }
    }

    #[test]
    fn round_trip_transaction_request_actions() {
        for action in [TxRequestAction::Read, TxRequestAction::Create, TxRequestAction::Approve] {
            round_trip(Permission::TransactionRequests(action));
        }
    }

    #[test]
    fn transaction_requests_serde_shape_matches_ts_binding() {
        // `Permission` has two independent serialisations: the string grammar
        // above (API-key column, CLI) and this serde form, which is what
        // crosses the JSON-RPC wire in `AuthLoginRequest.permissions` and what
        // the committed `bindings/src/types/Permission.ts` declares. They can
        // drift apart silently, so pin the wire form to the binding.
        let json = serde_json::to_string(&Permission::TransactionRequests(TxRequestAction::Approve)).unwrap();
        assert_eq!(json, r#"{"TransactionRequests":"Approve"}"#);

        let parsed: Permission = serde_json::from_str(r#"{"TransactionRequests":"Create"}"#).unwrap();
        assert_eq!(parsed, Permission::TransactionRequests(TxRequestAction::Create));
    }

    #[test]
    fn transaction_requests_is_unscoped() {
        // Unscoped for the same reason as `Transactions`: a request wraps an
        // arbitrary instruction list, so the wallet cannot statically
        // determine which accounts it will touch. `Transfer` remains the
        // scoped narrow path.
        assert!("transaction_requests:approve".parse::<Permission>().is_ok());
        let c = component(1);
        let s = format!("transaction_requests:approve:{c}");
        assert!(s.parse::<Permission>().is_err());
    }

    #[test]
    fn rejects_unknown_transaction_request_action() {
        assert!("transaction_requests:update".parse::<Permission>().is_err());
        assert!("transaction_requests:delete".parse::<Permission>().is_err());
        assert!("transaction_requests:frobnicate".parse::<Permission>().is_err());
    }

    #[test]
    fn admin_satisfies_transaction_requests() {
        for action in [TxRequestAction::Read, TxRequestAction::Create, TxRequestAction::Approve] {
            assert!(Permission::Admin.satisfies(&Permission::TransactionRequests(action)));
        }
    }

    #[test]
    fn round_trip_scoped_component_resources() {
        let c = component(7);
        round_trip(Permission::Accounts(Crud::Read, Some(c)));
        round_trip(Permission::Transfer(Crud::Create, Some(c)));
        round_trip(Permission::Confidential(Crud::Update, Some(c)));
        round_trip(Permission::StealthUtxos(Crud::Read, Some(c)));
    }

    #[test]
    fn round_trip_scoped_nft_resource() {
        let r = resource(11);
        round_trip(Permission::Nfts(Crud::Read, Some(r)));
    }

    #[test]
    fn round_trip_read_only_resources() {
        round_trip(Permission::Substates(ReadOnly::Read));
        round_trip(Permission::BurnProofs(ReadOnly::Read));
        round_trip(Permission::SwapPools(ReadOnly::Read));
        round_trip(Permission::Resources(ReadOnly::Read));
    }

    // ---------- FromStr rejection cases ----------

    #[test]
    fn rejects_unknown_resource() {
        assert!("nope:read".parse::<Permission>().is_err());
    }

    #[test]
    fn rejects_unknown_action() {
        assert!("accounts:frobnicate".parse::<Permission>().is_err());
    }

    #[test]
    fn rejects_missing_action() {
        assert!("accounts".parse::<Permission>().is_err());
    }

    #[test]
    fn rejects_action_on_read_only_resource() {
        assert!("substates:create".parse::<Permission>().is_err());
        assert!("burn_proofs:update".parse::<Permission>().is_err());
        assert!("swap_pools:delete".parse::<Permission>().is_err());
    }

    #[test]
    fn rejects_entity_on_read_only_resource() {
        let c = component(1);
        let s = format!("substates:read:{c}");
        assert!(s.parse::<Permission>().is_err());
    }

    #[test]
    fn rejects_entity_on_unscoped_resource() {
        let c = component(1);
        let s = format!("keys:read:{c}");
        assert!(s.parse::<Permission>().is_err());
    }

    #[test]
    fn rejects_malformed_entity() {
        assert!("accounts:read:not_a_component".parse::<Permission>().is_err());
    }

    #[test]
    fn rejects_legacy_pascal_case() {
        // Old grammar (`AccountInfo`, `KeyList`, `TransactionSend_…`) is dead.
        assert!("AccountInfo".parse::<Permission>().is_err());
        assert!("KeyList".parse::<Permission>().is_err());
        assert!("Admin".parse::<Permission>().is_err());
    }

    // ---------- Permissions list parsing ----------

    #[test]
    fn permissions_list_parses_comma_separated_with_whitespace() {
        let p: Permissions = "admin, accounts:read , transfer:create".parse().unwrap();
        assert_eq!(p.len(), 3);
        assert!(p.has_permission(&Permission::Admin));
        assert!(p.has_permission(&Permission::Accounts(Crud::Read, None)));
        assert!(p.has_permission(&Permission::Transfer(Crud::Create, None)));
    }

    #[test]
    fn permissions_list_ignores_empty_segments() {
        let p: Permissions = "admin,,, accounts:read,".parse().unwrap();
        assert_eq!(p.len(), 2);
    }

    // ---------- satisfies(): Admin ----------

    #[test]
    fn admin_satisfies_everything() {
        let admin = Permission::Admin;
        assert!(admin.satisfies(&Permission::Accounts(Crud::Delete, Some(component(1)))));
        assert!(admin.satisfies(&Permission::Substates(ReadOnly::Read)));
        assert!(admin.satisfies(&Permission::Webrtc));
        assert!(admin.satisfies(&Permission::Admin));
    }

    #[test]
    fn non_admin_does_not_satisfy_admin() {
        let p = Permission::Accounts(Crud::Read, None);
        assert!(!p.satisfies(&Permission::Admin));
    }

    // ---------- satisfies(): same-action, write-implies-read ----------

    #[test]
    fn same_action_satisfies() {
        let p = Permission::Accounts(Crud::Update, None);
        assert!(p.satisfies(&Permission::Accounts(Crud::Update, None)));
    }

    #[test]
    fn mutations_imply_read_on_same_resource() {
        for write in [Crud::Create, Crud::Update, Crud::Delete] {
            assert!(
                Permission::Accounts(write, None).satisfies(&Permission::Accounts(Crud::Read, None)),
                "{write:?} should imply Read"
            );
        }
    }

    #[test]
    fn transaction_request_create_does_not_imply_approve() {
        // The separation this asserts *is* the feature: a tool granted
        // `transaction_requests:create` must not be able to approve the
        // requests it creates. Nothing else enforces that — there is no
        // creator-identity check on the approve path.
        let create = Permission::TransactionRequests(TxRequestAction::Create);
        assert!(!create.satisfies(&Permission::TransactionRequests(TxRequestAction::Approve)));

        let approve = Permission::TransactionRequests(TxRequestAction::Approve);
        assert!(!approve.satisfies(&Permission::TransactionRequests(TxRequestAction::Create)));
    }

    #[test]
    fn transactions_and_transaction_requests_are_separate_resources() {
        // `transactions:create` is unrestricted submit authority. If it
        // satisfied `transaction_requests:approve`, every principal that can
        // already submit could also approve, and the request flow would be
        // decorative.
        let submit = Permission::Transactions(Crud::Create);
        assert!(!submit.satisfies(&Permission::TransactionRequests(TxRequestAction::Approve)));
        assert!(!submit.satisfies(&Permission::TransactionRequests(TxRequestAction::Create)));
        assert!(!submit.satisfies(&Permission::TransactionRequests(TxRequestAction::Read)));

        let approve = Permission::TransactionRequests(TxRequestAction::Approve);
        assert!(!approve.satisfies(&Permission::Transactions(Crud::Create)));
        assert!(!approve.satisfies(&Permission::Transactions(Crud::Read)));
    }

    #[test]
    fn round_trip_signing_request_actions() {
        for action in [TxRequestAction::Read, TxRequestAction::Create, TxRequestAction::Approve] {
            round_trip(Permission::SigningRequests(action));
        }
        assert_eq!(
            "signing_requests:approve".parse::<Permission>().unwrap(),
            Permission::SigningRequests(TxRequestAction::Approve)
        );
        assert!("signing_requests:update".parse::<Permission>().is_err());
        let json = serde_json::to_string(&Permission::SigningRequests(TxRequestAction::Create)).unwrap();
        assert_eq!(json, r#"{"SigningRequests":"Create"}"#);
    }

    #[test]
    fn signing_requests_are_separate_from_transaction_requests() {
        // Approving a signing request releases a signature over an arbitrary
        // transaction, so neither resource's grants may stand in for the
        // other's.
        let tx_approve = Permission::TransactionRequests(TxRequestAction::Approve);
        assert!(!tx_approve.satisfies(&Permission::SigningRequests(TxRequestAction::Approve)));
        assert!(!tx_approve.satisfies(&Permission::SigningRequests(TxRequestAction::Read)));

        let sign_create = Permission::SigningRequests(TxRequestAction::Create);
        assert!(!sign_create.satisfies(&Permission::SigningRequests(TxRequestAction::Approve)));
        assert!(!sign_create.satisfies(&Permission::TransactionRequests(TxRequestAction::Create)));
        assert!(sign_create.satisfies(&Permission::SigningRequests(TxRequestAction::Read)));
    }

    #[test]
    fn transaction_request_create_and_approve_imply_read() {
        for action in [TxRequestAction::Create, TxRequestAction::Approve] {
            assert!(
                Permission::TransactionRequests(action)
                    .satisfies(&Permission::TransactionRequests(TxRequestAction::Read)),
                "{action:?} should imply Read"
            );
        }
    }

    #[test]
    fn no_implication_between_mutations() {
        assert!(!Permission::Accounts(Crud::Update, None).satisfies(&Permission::Accounts(Crud::Create, None)));
        assert!(!Permission::Accounts(Crud::Update, None).satisfies(&Permission::Accounts(Crud::Delete, None)));
        assert!(!Permission::Accounts(Crud::Create, None).satisfies(&Permission::Accounts(Crud::Update, None)));
    }

    #[test]
    fn read_does_not_imply_mutations() {
        assert!(!Permission::Accounts(Crud::Read, None).satisfies(&Permission::Accounts(Crud::Update, None)));
    }

    // ---------- satisfies(): scope semantics ----------

    #[test]
    fn unscoped_grant_satisfies_any_required_scope() {
        let granted = Permission::Accounts(Crud::Read, None);
        assert!(granted.satisfies(&Permission::Accounts(Crud::Read, Some(component(1)))));
        assert!(granted.satisfies(&Permission::Accounts(Crud::Read, None)));
    }

    #[test]
    fn scoped_grant_does_not_satisfy_unscoped_requirement() {
        let granted = Permission::Accounts(Crud::Read, Some(component(1)));
        assert!(!granted.satisfies(&Permission::Accounts(Crud::Read, None)));
    }

    #[test]
    fn scoped_grant_matches_only_identical_scope() {
        let granted = Permission::Accounts(Crud::Read, Some(component(1)));
        assert!(granted.satisfies(&Permission::Accounts(Crud::Read, Some(component(1)))));
        assert!(!granted.satisfies(&Permission::Accounts(Crud::Read, Some(component(2)))));
    }

    #[test]
    fn scoped_mutation_implies_scoped_read_same_entity() {
        let granted = Permission::Accounts(Crud::Update, Some(component(1)));
        assert!(granted.satisfies(&Permission::Accounts(Crud::Read, Some(component(1)))));
        assert!(!granted.satisfies(&Permission::Accounts(Crud::Read, Some(component(2)))));
    }

    // ---------- satisfies(): cross-resource ----------

    #[test]
    fn cross_resource_never_satisfies() {
        let granted = Permission::Accounts(Crud::Read, None);
        assert!(!granted.satisfies(&Permission::Keys(Crud::Read)));
        assert!(!granted.satisfies(&Permission::Substates(ReadOnly::Read)));
        assert!(!granted.satisfies(&Permission::Transactions(Crud::Read)));
    }

    #[test]
    fn transactions_create_does_not_imply_transfer_create() {
        // The split is intentional: a power-key with transactions:create
        // must still be granted transfer:create explicitly to use the
        // narrow transfer handlers. Verifies no implicit containment.
        let granted = Permission::Transactions(Crud::Create);
        assert!(!granted.satisfies(&Permission::Transfer(Crud::Create, None)));
    }

    // ---------- Permissions::satisfies set semantics ----------

    #[test]
    fn any_grant_in_set_satisfies() {
        let set: Permissions = vec![
            Permission::Accounts(Crud::Read, None),
            Permission::Transfer(Crud::Create, Some(component(1))),
        ]
        .into();
        assert!(set.satisfies(&Permission::Accounts(Crud::Read, Some(component(9)))));
        assert!(set.satisfies(&Permission::Transfer(Crud::Create, Some(component(1)))));
        assert!(!set.satisfies(&Permission::Transfer(Crud::Create, Some(component(2)))));
        assert!(!set.satisfies(&Permission::Keys(Crud::Read)));
    }

    #[test]
    fn transactions_is_unscoped() {
        // `Transactions` deliberately carries no entity scope: the wallet
        // cannot statically determine which accounts an arbitrary
        // instruction list will touch. Round-trip via `parse_scoped` would
        // accept a third segment — verify the unscoped parser rejects it.
        assert!("transactions:create".parse::<Permission>().is_ok());
        let c = component(1);
        let s = format!("transactions:create:{c}");
        assert!(s.parse::<Permission>().is_err());
    }

    #[test]
    fn empty_set_satisfies_nothing() {
        let set = Permissions::default();
        assert!(!set.satisfies(&Permission::Accounts(Crud::Read, None)));
        assert!(!set.satisfies(&Permission::Admin));
    }
}
