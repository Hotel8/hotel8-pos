// SPDX-FileCopyrightText: Copyright Logically Magic Inc. <hello@logicallymagic.com>
// SPDX-FileCopyrightText: Copyright Hamid Jahanzad <Hamid@Jahanzad.com>
// SPDX-FileCopyrightText: Copyright Contributors to the Hotel8:POS
// SPDX-License-Identifier: Apache-2.0

//! Service self-registration data for Hotel8:POS.
//!
//! Output via `--print-registration` flag. See `sentin8/design/sentin8-service-self-registration-design.md`.

use sentin8::registration::{
    AdminScope, AdminUi, AdminUiPage, M2mGrant, PgRole, PolicySetDecl, Registration, RelationDecl,
    SqlMigration,
};
use utoipa::OpenApi;

const PG_ROLES: &[PgRole] = &[PgRole {
    scope: "hotel8:pos:read,hotel8:pos:write",
    role: "hotel8_pos_service",
    bypass_rls: false,
    // DBX (§6.6): grants + scope-map rows live in the hotel8 vertical
    // database only (database-per-vertical, schema-per-service).
    database: "hotel8",
    // This service owns all its reads - no cross-schema grants.
    cross_schema_grants: &[],
}];

/// M2M DCR scope grants (self-registration design §3.11). One entry per
/// workload this service registers via DCR; `vertical enroll` upserts
/// `certless:<workload>` rows into `sentin8.auth_m2m_scope_grants` (alignment
/// doc §13). DBX (§6.7): `sentin8:bus:produce` rides the `pos` workload -
/// the shell's producer relay shares its DCR client; `pos-audit` ships the
/// local audit journal to Sys; `pos-policy`/`pos-tls` are the shell's
/// Shield hot-reload and DB-free TLS host-context workloads.
const M2M_GRANTS: &[M2mGrant] = &[
    M2mGrant {
        workload: "hotel8:pos",
        scopes: &[
            "sentin8:bus:produce",
            "sentin8:bus:consume",
            "sentin8:stat:measure",
        ],
    },
    M2mGrant {
        workload: "pos",
        scopes: &[
            "sentin8:bus:produce",
            "sentin8:bus:consume",
            "sentin8:stat:measure",
        ],
    },
    M2mGrant {
        workload: "pos-audit",
        scopes: &["sentin8:sys:audit:write"],
    },
    M2mGrant {
        workload: "hotel8:pos-policy",
        scopes: &["sentin8:bus:consume", "sentin8:stat:measure"],
    },
    M2mGrant {
        workload: "pos-policy",
        scopes: &["sentin8:stat:measure"],
    },
    M2mGrant {
        workload: "hotel8-pos-tls",
        scopes: &["sentin8:sys:instances:read"],
    },
    // Settlement lane (bill8): card capture via charge + token consume.
    M2mGrant {
        workload: "pos-payments",
        scopes: &[
            "bill8:payments:charges:write",
            "bill8:payments:tokens:write",
        ],
    },
];

const ADMIN_SCOPES: &[AdminScope] = &[
    AdminScope::Typed {
        role: "hotel8:pos:admin",
        scope: "hotel8:pos:admin",
    },
    AdminScope::Typed {
        role: "hotel8:pos:admin",
        scope: "hotel8:pos:read",
    },
    AdminScope::Typed {
        role: "hotel8:pos:admin",
        scope: "hotel8:pos:order:write",
    },
    AdminScope::Typed {
        role: "hotel8:pos:admin",
        scope: "hotel8:pos:order:read",
    },
    AdminScope::Typed {
        role: "hotel8:pos:admin",
        scope: "hotel8:pos:payment:write",
    },
    AdminScope::Typed {
        role: "hotel8:pos:admin",
        scope: "hotel8:pos:report:read",
    },
    // Guest ordering (room service) - eGuest calls through the BFF with the
    // guest session carrying these scopes (pos DESIGN.md §4 Guest ordering).
    AdminScope::Typed {
        role: "hotel8:pos:admin",
        scope: "hotel8:pos:guest:read",
    },
    AdminScope::Typed {
        role: "hotel8:pos:admin",
        scope: "hotel8:pos:guest:write",
    },
    // Policy-admin surface (policy design §3.6; principles §6.0b per-service scope).
    AdminScope::Typed {
        role: "hotel8:pos:admin",
        scope: "hotel8:pos:policy:read",
    },
    AdminScope::Typed {
        role: "hotel8:pos:admin",
        scope: "hotel8:pos:policy:write",
    },
];

const ADMIN_UI_PAGES: &[AdminUiPage] = &[
    AdminUiPage {
        route: "overview",
        label: "Overview",
        ui_endpoint: "/v1/admin/ui/overview",
        data_endpoint: "/v1/admin/overview",
        icon: "0xe1b1",
    },
    AdminUiPage {
        route: "orders",
        label: "Orders",
        ui_endpoint: "/v1/admin/ui/orders",
        data_endpoint: "/v1/admin/orders",
        icon: "0xe59c",
    },
    AdminUiPage {
        route: "payments",
        label: "Payments",
        ui_endpoint: "/v1/admin/ui/payments",
        data_endpoint: "/v1/admin/payments",
        icon: "0xe482",
    },
    AdminUiPage {
        route: "reports",
        label: "Reports",
        ui_endpoint: "/v1/admin/ui/reports",
        data_endpoint: "/v1/admin/reports",
        icon: "0xe0a4",
    },
    AdminUiPage {
        route: "settings",
        label: "Settings",
        ui_endpoint: "/v1/admin/ui/settings",
        data_endpoint: "/v1/admin/settings",
        icon: "0xe57f",
    }, // Sentin8:Policy admin surface (Shield pattern).
    AdminUiPage {
        route: "policy-sets",
        label: "Policy Sets",
        ui_endpoint: "/v1/admin/ui/policy-sets",
        data_endpoint: "/v1/admin/policy-sets",
        icon: "0xe8b8",
    },
    AdminUiPage {
        route: "policy-decisions",
        label: "Policy Decisions",
        ui_endpoint: "/v1/admin/ui/policy-decisions",
        data_endpoint: "/v1/admin/policy-decisions",
        icon: "0xf070",
    },
    AdminUiPage {
        route: "relations",
        label: "Relations",
        ui_endpoint: "/v1/admin/ui/relations",
        data_endpoint: "/v1/admin/relations",
        icon: "0xe572",
    },
    AdminUiPage {
        route: "attributes",
        label: "Attributes",
        ui_endpoint: "/v1/admin/ui/attributes",
        data_endpoint: "/v1/admin/attributes",
        icon: "0xe8d3",
    },
];

/// The Shield text of the declared scope-mirror set - public so the
/// policy wiring tests assert against the REAL seed (drift
/// fails the suite instead of silently diverging from enrollment).
pub const SCOPE_MIRROR_POLICIES: &str = "permit(principal, action == Action::\"hotel8:pos:admin\", resource) when { principal.scopes.contains(\"hotel8:pos:admin\") };\npermit(principal, action == Action::\"hotel8:pos:admin\", resource) when { principal.scopes.contains(\"hotel8:pos:order:write\") };\npermit(principal, action == Action::\"hotel8:pos:admin\", resource) when { principal.scopes.contains(\"hotel8:pos:read\") };\npermit(principal, action == Action::\"hotel8:pos:admin\", resource) when { principal.scopes.contains(\"sentin8:sys:admin\") };\npermit(principal, action == Action::\"hotel8:pos:admin\", resource) when { principal.scopes.contains(\"sentin8:sys:env:admin\") };\npermit(principal, action == Action::\"hotel8:pos:admin\", resource) when { principal.scopes.contains(\"sentin8:sys:org:admin\") };\npermit(principal, action == Action::\"hotel8:pos:order:read\", resource) when { principal.scopes.contains(\"hotel8:pos:order:read\") };\npermit(principal, action == Action::\"hotel8:pos:order:read\", resource) when { principal.scopes.contains(\"hotel8:pos:read\") };\npermit(principal, action == Action::\"hotel8:pos:order:read\", resource) when { principal.scopes.contains(\"sentin8:sys:admin\") };\npermit(principal, action == Action::\"hotel8:pos:order:read\", resource) when { principal.scopes.contains(\"sentin8:sys:env:admin\") };\npermit(principal, action == Action::\"hotel8:pos:order:read\", resource) when { principal.scopes.contains(\"sentin8:sys:org:admin\") };\npermit(principal, action == Action::\"hotel8:pos:order:write\", resource) when { principal.scopes.contains(\"hotel8:pos:order:write\") };\npermit(principal, action == Action::\"hotel8:pos:order:write\", resource) when { principal.scopes.contains(\"hotel8:pos:payment:write\") };\npermit(principal, action == Action::\"hotel8:pos:order:write\", resource) when { principal.scopes.contains(\"sentin8:sys:admin\") };\npermit(principal, action == Action::\"hotel8:pos:order:write\", resource) when { principal.scopes.contains(\"sentin8:sys:env:admin\") };\npermit(principal, action == Action::\"hotel8:pos:order:write\", resource) when { principal.scopes.contains(\"sentin8:sys:org:admin\") };\npermit(principal, action == Action::\"hotel8:pos:payment:write\", resource) when { principal.scopes.contains(\"hotel8:pos:payment:write\") };\npermit(principal, action == Action::\"hotel8:pos:payment:write\", resource) when { principal.scopes.contains(\"hotel8:pos:read\") };\npermit(principal, action == Action::\"hotel8:pos:payment:write\", resource) when { principal.scopes.contains(\"sentin8:sys:admin\") };\npermit(principal, action == Action::\"hotel8:pos:payment:write\", resource) when { principal.scopes.contains(\"sentin8:sys:env:admin\") };\npermit(principal, action == Action::\"hotel8:pos:payment:write\", resource) when { principal.scopes.contains(\"sentin8:sys:org:admin\") };\npermit(principal, action == Action::\"hotel8:pos:read\", resource) when { principal.scopes.contains(\"hotel8:pos:read\") };\npermit(principal, action == Action::\"hotel8:pos:read\", resource) when { principal.scopes.contains(\"sentin8:sys:admin\") };\npermit(principal, action == Action::\"hotel8:pos:read\", resource) when { principal.scopes.contains(\"sentin8:sys:env:admin\") };\npermit(principal, action == Action::\"hotel8:pos:read\", resource) when { principal.scopes.contains(\"sentin8:sys:org:admin\") };\npermit(principal, action == Action::\"hotel8:pos:report:read\", resource) when { principal.scopes.contains(\"hotel8:pos:read\") };\npermit(principal, action == Action::\"hotel8:pos:report:read\", resource) when { principal.scopes.contains(\"hotel8:pos:report:read\") };\npermit(principal, action == Action::\"hotel8:pos:report:read\", resource) when { principal.scopes.contains(\"sentin8:sys:admin\") };\npermit(principal, action == Action::\"hotel8:pos:report:read\", resource) when { principal.scopes.contains(\"sentin8:sys:env:admin\") };\npermit(principal, action == Action::\"hotel8:pos:report:read\", resource) when { principal.scopes.contains(\"sentin8:sys:org:admin\") };\npermit(principal, action == Action::\"hotel8:pos:guest:read\", resource) when { principal.scopes.contains(\"hotel8:pos:guest:read\") };\npermit(principal, action == Action::\"hotel8:pos:guest:read\", resource) when { principal.scopes.contains(\"hotel8:pos:read\") };\npermit(principal, action == Action::\"hotel8:pos:guest:read\", resource) when { principal.scopes.contains(\"sentin8:sys:admin\") };\npermit(principal, action == Action::\"hotel8:pos:guest:read\", resource) when { principal.scopes.contains(\"sentin8:sys:env:admin\") };\npermit(principal, action == Action::\"hotel8:pos:guest:read\", resource) when { principal.scopes.contains(\"sentin8:sys:org:admin\") };\npermit(principal, action == Action::\"hotel8:pos:guest:write\", resource) when { principal.scopes.contains(\"hotel8:pos:guest:write\") };\npermit(principal, action == Action::\"hotel8:pos:guest:write\", resource) when { principal.scopes.contains(\"hotel8:pos:order:write\") };\npermit(principal, action == Action::\"hotel8:pos:guest:write\", resource) when { principal.scopes.contains(\"sentin8:sys:admin\") };\npermit(principal, action == Action::\"hotel8:pos:guest:write\", resource) when { principal.scopes.contains(\"sentin8:sys:env:admin\") };\npermit(principal, action == Action::\"hotel8:pos:guest:write\", resource) when { principal.scopes.contains(\"sentin8:sys:org:admin\") };";

/// Default declarative policy set (Sentin8:Policy design §3.2/§3.4,
/// seeded by `vertical enroll` as `origin='platform'`, `mode='enforced'`):
/// action-granular scope permits over the LIVE effective scope set - the
/// exact mirror of the gates this service's handlers evaluate in step 5
/// (policy design §4.3). Enforced: step 5 is the
/// only gate; shadow is the emergency one-UPDATE rollback lever only.
const POLICIES: &[PolicySetDecl] = &[PolicySetDecl {
    policy_set_name: "scope-mirror",
    description: "Action-granular scope permits for hotel8:pos over the LIVE effective scope set (design §3.1d). Enforced - step 5 is the only gate.",
    // Shield text lives in SCOPE_MIRROR_POLICIES (single source).
    policies: SCOPE_MIRROR_POLICIES,
    schema: None,
    mode: "enforced",
    default_enabled: true,
}];

/// Default ReBAC relation tuples (design §3.1c) - none declared by this
/// service today; env admins author their own graph (staff→station,
/// channel-partner, holder edges) via the relations admin API.
const RELATIONS: &[RelationDecl] = &[];

const SQL_1_INIT: &str = include_str!("../sql/init.sql");

fn openapi_spec() -> serde_json::Value {
    serde_json::to_value(crate::openapi::PosOpenApi::openapi()).unwrap_or(serde_json::Value::Null)
}

/// Print the registration JSON to stdout.
pub fn print(section: Option<String>) -> anyhow::Result<()> {
    let reg = Registration {
        service_type: "hotel8:pos",
        subdomain: "hotel8-pos",
        health_path: "/v1/health",
        admin_scopes: ADMIN_SCOPES,
        error_codes: &[],
        consent_providers: &[],
        consent_flows: &[],
        admin_ui: Some(AdminUi {
            label: "POS",
            icon: "0xe4d8",
            service_flag: "pos",
            pages: ADMIN_UI_PAGES,
        }),
        sql_migrations: vec![SqlMigration {
            version: 1,
            name: "init",
            sql: SQL_1_INIT,
        }],
        pg_roles: PG_ROLES,
        audit_findings: None,
        service_actions: None,
        m2m_grants: Some(M2M_GRANTS),
        openapi_url: "/openapi.json",
        openapi: openapi_spec,
        policies: POLICIES,
        relations: RELATIONS,
        event_triggers: &[],
        schedules: &[],
    };
    sentin8::registration::print_registration(section, &reg, openapi_spec)
}
