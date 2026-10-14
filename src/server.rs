// SPDX-FileCopyrightText: Copyright Logically Magic Inc. <hello@logicallymagic.com>
// SPDX-FileCopyrightText: Copyright Hamid Jahanzad <Hamid@Jahanzad.com>
// SPDX-FileCopyrightText: Copyright Contributors to the Hotel8:POS
// SPDX-License-Identifier: Apache-2.0

//! Axum server setup for H8POS.
use axum::routing::{delete, get, post, put};
use std::sync::Arc;

use crate::handlers::{self, AppState};

pub async fn build_state(
    core: sentin8::shell::ServiceCore,
) -> anyhow::Result<crate::handlers::AppState> {
    let config = crate::config::POSConfig::from_env();

    // Cert-presenting Bus client (CERTFREE-4V): the `pos` workload
    // CERTIFICATE is the identity — no DCR, no token (auth DESIGN
    // §4.3f); the mint retries while the fleet boots.
    let bus_client = loop {
        match sentin8::bus::client::BusClient::for_workload(
            &config.sentin8_bus_url,
            &config.sentin8_auth_issuer,
            "pos",
        )
        .await
        {
            Ok(c) => break c,
            Err(e) => {
                tracing::debug!(error = %e, "pos bus client: certificate not ready yet");
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            }
        }
    };

    // PMS cross-service client (CERTFREE-4V): this service presents its
    // own `pos` workload certificate — PMS's TLS layer authenticates
    // the handshake and the registry map authorizes the scopes (the
    // `pms:reservations:read` / `pms:profiles:read` grants ride the
    // `pos` map row). The mint retries while the fleet boots.
    let pms_client = loop {
        match hotel8::pms::client::H8pmsClient::with_workload_certificate(
            &config.pms_url,
            &config.sentin8_auth_issuer,
            "pos",
        )
        .await
        {
            Ok(c) => break c,
            Err(e) => {
                tracing::debug!(error = %e, "pos pms client: certificate not ready yet");
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            }
        }
    };

    // ── Sentin8:Policy (Shield) ──
    // Load the service's policy sets; an unparsable policy text REFUSES TO
    // START (fail-closed at boot, design §3.3). Hot-reload consumes the
    // `policy` stream as a direct consumer (design §3.2); deny/would-deny
    // decisions emit to Stat (§4.2f). The engine is stored in AppState so
    // handlers gate via `policy::require` (step 5 is the ONLY gate).
    // ── Sentin8:Policy (Shield, shell-owned boot): consume the
    // engine the shell booted in step 8 — no second engine, consumer, or
    // DCR registration here.
    let policy_engine = core.admin.policy.clone();

    let state = AppState {
        config: Arc::new(config),
        pool: core.pool.clone(),
        bus_client,
        pms_client,
        policy: policy_engine.clone(),
        admin: sentin8::shield::admin::SharedAdminState {
            service: "hotel8:pos".to_string(),
            pool: core.pool.clone(),
            policy: policy_engine,
            auth_client: Arc::new(core.config.sentin8_auth_client.clone()),
        },
    };

    // Event-driven consumer — subscribes to the PMS stream family.
    crate::event_consumer::spawn(core.pool.clone(), state.bus_client.clone());

    Ok(state)
}

/// Add this service's routes onto the shell-provided router. The shell
/// owns the boot sequence, state binding, OpenAPI merge and the standard
/// middleware stack; this adds ONLY routes (including the service's own
/// `/v1/health` — custom payload).
pub fn routes(
    router: axum::Router<crate::handlers::AppState>,
) -> axum::Router<crate::handlers::AppState> {
    router
        .route("/v1/health", get(handlers::health))
        // ── Admin UI (RFW) ─────────────────────────────────────────────────
        .route("/v1/admin/ui/{*page}", get(crate::admin_ui::get_admin_ui))
        // ── Sentin8:Policy admin API (shared implementation,
        // handlers generic over this AppState).
        .route("/v1/admin/policy-sets",
            get(sentin8::shield::admin::list_policy_sets::<crate::handlers::AppState>))
        .route("/v1/admin/policy-sets/{name}",
            get(sentin8::shield::admin::get_policy_set::<crate::handlers::AppState>)
                .put(sentin8::shield::admin::put_policy_set::<crate::handlers::AppState>)
                .delete(sentin8::shield::admin::delete_policy_set::<crate::handlers::AppState>))
        .route("/v1/admin/policy-sets/{name}/test",
            post(sentin8::shield::admin::test_policy_set::<crate::handlers::AppState>))
        .route("/v1/admin/policy-sets/enforce",
            post(sentin8::shield::admin::admin_enforce_policy_set::<crate::handlers::AppState>))
        .route("/v1/admin/policy-sets/shadow",
            post(sentin8::shield::admin::admin_shadow_policy_set::<crate::handlers::AppState>))
        .route("/v1/admin/policy-sets/disable",
            post(sentin8::shield::admin::admin_disable_policy_set::<crate::handlers::AppState>))
        .route("/v1/admin/relations",
            get(sentin8::shield::admin::list_relations::<crate::handlers::AppState>)
                .put(sentin8::shield::admin::put_relation::<crate::handlers::AppState>)
                .delete(sentin8::shield::admin::delete_relation::<crate::handlers::AppState>))
        .route("/v1/admin/relations/check",
            get(sentin8::shield::admin::check_relation::<crate::handlers::AppState>))
        .route("/v1/admin/attributes",
            get(sentin8::shield::admin::list_attributes::<crate::handlers::AppState>)
                .put(sentin8::shield::admin::put_attribute::<crate::handlers::AppState>)
                .delete(sentin8::shield::admin::delete_attribute::<crate::handlers::AppState>))
        // ── Outlets (Revenue Centers) ─────────────────────────────────────
        .route("/v1/outlets", post(handlers::outlets::create_outlet).get(handlers::outlets::list_outlets))
        // ── Tables ──────────────────────────────────────────────────────────
        .route("/v1/outlets/{outlet_id}/tables", post(handlers::tables::create_table).get(handlers::tables::list_tables))
        .route("/v1/outlets/{outlet_id}/tables/combine", post(handlers::tables::combine_tables))
        .route("/v1/outlets/{outlet_id}/tables/separate", post(handlers::tables::separate_tables))
        .route("/v1/outlets/{outlet_id}/tables/floor-plan", get(handlers::tables::get_floor_plan))
        .route("/v1/outlets/{outlet_id}/tables/{table_id}", get(handlers::tables::get_table).put(handlers::tables::update_table).delete(handlers::tables::delete_table))
        .route("/v1/outlets/{outlet_id}/tables/{table_id}/status", put(handlers::tables::update_table_status))
        // ── Menu ────────────────────────────────────────────────────────────
        .route("/v1/outlets/{outlet_id}/menu-items", post(handlers::menu::create_menu_item).get(handlers::menu::list_menu_items))
        // ── Config: Service Areas ───────────────────────────────
        .route("/v1/outlets/{outlet_id}/service-areas", post(handlers::config::create_service_area).get(handlers::config::list_service_areas))
        .route("/v1/outlets/{outlet_id}/service-areas/{area_id}", put(handlers::config::update_service_area).delete(handlers::config::delete_service_area))
        // ── Config: Menu Categories ──────────────────────────────
        .route("/v1/outlets/{outlet_id}/menu/categories", post(handlers::config::create_menu_category).get(handlers::config::list_menu_categories))
        .route("/v1/outlets/{outlet_id}/menu/categories/{category_id}", put(handlers::config::update_menu_category).delete(handlers::config::delete_menu_category))
        // ── Config: Modifier Groups + Modifiers ─────────
        .route("/v1/outlets/{outlet_id}/menu-items/{item_id}/modifier-groups", post(handlers::config::create_modifier_group).get(handlers::config::list_modifier_groups))
        .route("/v1/outlets/{outlet_id}/menu-items/{item_id}/modifier-groups/{group_id}", delete(handlers::config::delete_modifier_group))
        .route("/v1/outlets/{outlet_id}/menu-items/{item_id}/modifier-groups/{group_id}/modifiers", post(handlers::config::create_modifier).get(handlers::config::list_modifiers))
        .route("/v1/outlets/{outlet_id}/menu-items/{item_id}/modifier-groups/{group_id}/modifiers/{modifier_id}", delete(handlers::config::delete_modifier))
        // ── Config: Item Availability + Bulk Price ─────
        .route("/v1/outlets/{outlet_id}/menu-items/{item_id}/availability", put(handlers::config::update_item_availability))
        .route("/v1/outlets/{outlet_id}/menu-items/prices", put(handlers::config::bulk_update_prices))
        // ── Config: Tax Groups ──────────────────────────────────
        .route("/v1/outlets/{outlet_id}/tax-groups", post(handlers::config::create_tax_group).get(handlers::config::list_tax_groups))
        .route("/v1/outlets/{outlet_id}/tax-groups/{tax_id}", put(handlers::config::update_tax_group).delete(handlers::config::delete_tax_group))
        // ── Config: Printer Groups ─────────────────────────────
        .route("/v1/outlets/{outlet_id}/printer-groups", post(handlers::config::create_printer_group).get(handlers::config::list_printer_groups))
        .route("/v1/outlets/{outlet_id}/printer-groups/{printer_id}", put(handlers::config::update_printer_group).delete(handlers::config::delete_printer_group))
        // ── Config: Payment Methods ────────────────────────────
        .route("/v1/outlets/{outlet_id}/payment-methods", get(handlers::config::get_payment_methods).put(handlers::config::update_payment_methods))
        // ── Checks + Orders ─────────────────────────────────────────────────
        .route("/v1/outlets/{outlet_id}/checks", post(handlers::orders::create_check).get(handlers::orders::search_checks))
        .route("/v1/outlets/{outlet_id}/checks/{check_id}", get(handlers::orders::get_check))
        .route("/v1/outlets/{outlet_id}/checks/{check_id}/orders", post(handlers::orders::add_order_item))
        .route("/v1/outlets/{outlet_id}/checks/{check_id}/transfer", post(handlers::orders::transfer_check))
        .route("/v1/outlets/{outlet_id}/checks/{check_id}/split", post(handlers::orders::split_check))
        .route("/v1/outlets/{outlet_id}/checks/{check_id}/hold", post(handlers::orders::hold_check))
        .route("/v1/outlets/{outlet_id}/checks/{check_id}/recall", post(handlers::orders::recall_check))
        .route("/v1/outlets/{outlet_id}/checks/{check_id}/void", post(handlers::orders::void_check))
        .route("/v1/outlets/{outlet_id}/checks/{check_id}/orders/{order_id}/seat", post(handlers::orders::assign_seat))
        .route("/v1/outlets/{outlet_id}/orders/void", post(handlers::orders::void_order_item))
        // ── Payments ────────────────────────────────────────────────────────
        .route("/v1/outlets/{outlet_id}/checks/{check_id}/payments", post(handlers::payments::post_payment))
        .route("/v1/outlets/{outlet_id}/checks/{check_id}/payments-process", post(handlers::payments::bill8_payments_process_payment))
        .route("/v1/outlets/{outlet_id}/checks/{check_id}/split-payment", post(handlers::payments::split_payment))
        .route("/v1/outlets/{outlet_id}/checks/{check_id}/discount", post(handlers::payments::apply_discount))
        .route("/v1/payments/room-charge", post(handlers::payments::room_charge))
        .route("/v1/guests/lookup-by-room", get(handlers::payments::lookup_guest_by_room))
        .route("/v1/outlets/{outlet_id}/checks/{check_id}/redeem-package", post(handlers::payments::redeem_package))
        // ── Reports ────────────────────────────────────────────────
        .route("/v1/outlets/{outlet_id}/reports/sales", get(handlers::reports::daily_sales_summary))
        .route("/v1/outlets/{outlet_id}/reports/items", get(handlers::reports::item_sales_report))
        .route("/v1/outlets/{outlet_id}/reports/labor", get(handlers::reports::labor_report))
        .route("/v1/outlets/{outlet_id}/reports/end-of-day", get(handlers::reports::end_of_day_report))
        .route("/v1/outlets/{outlet_id}/reports/export/{report_type}", get(handlers::reports::export_report))
        // ── Hardening ──────────────────────────────────────────────
        .route("/v1/permissions", get(handlers::hardening::get_user_permissions))
        .route("/v1/manager-approve", post(handlers::hardening::manager_approve))
        .route("/v1/offline-status", get(handlers::hardening::get_offline_status))
        .route("/v1/offline-sync", post(handlers::hardening::sync_offline_data))
        .route("/v1/outlets/{outlet_id}/checks/{check_id}/orders/{order_id}/route-kitchen", post(handlers::hardening::route_order_to_kitchen))
        // ── Guest ordering (room service) — eGuest surface ──
        .route("/v1/guest/{property_id}/menu", get(handlers::guest_ordering::guest_menu))
        .route("/v1/guest/{property_id}/orders", post(handlers::guest_ordering::guest_create_order))
        .route("/v1/guest/{property_id}/orders/{check_id}", get(handlers::guest_ordering::guest_get_order))
}

/// Shared policy-admin surface accessor — the shared
/// handlers extract this service's admin state via `HasPolicyAdmin`.
impl sentin8::shield::admin::HasPolicyAdmin for crate::handlers::AppState {
    fn policy_admin(&self) -> &sentin8::shield::admin::SharedAdminState {
        &self.admin
    }
}
