// SPDX-FileCopyrightText: Copyright Logically Magic Inc. <hello@logicallymagic.com>
// SPDX-FileCopyrightText: Copyright Hamid Jahanzad <Hamid@Jahanzad.com>
// SPDX-FileCopyrightText: Copyright Contributors to the Hotel8:POS
// SPDX-License-Identifier: Apache-2.0

//! H8POS HTTP handlers - re-exports from sub-modules.
pub mod config;
pub mod guest_ordering;
pub mod hardening;
pub mod menu;
pub mod orders;
pub mod outlets;
pub mod payments;
pub mod reports;
pub mod tables;

use crate::config::POSConfig;
use sentin8::auth::client::AuthContext;
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<POSConfig>,
    pub pool: sqlx::PgPool,
    pub bus_client: sentin8::bus::client::BusClient,
    /// Cross-service PMS client (B1) - replaces direct pms_* SQL.
    pub pms_client: hotel8::pms::client::H8pmsClient,
    /// Shield policy engine: step 5 is the ONLY gate -
    /// handlers verify the token (steps 1-4) then evaluate a
    /// `hotel8:pos:<resource>:<action>` target; no JWT scope check remains.
    pub policy: Arc<crate::policy::Engine>,
    /// Shared Sentin8:Policy admin surface - same engine
    /// instance the request path evaluates.
    pub admin: sentin8::shield::admin::SharedAdminState,
}

/// Resolve the property the POS env serves (B1 helper). Outlets are created
/// per property - use the most recent outlet's property. Errors are mapped
/// by the caller.
pub async fn property_id_for_env_pool(
    pool: &sqlx::PgPool,
    env_id: uuid::Uuid,
) -> Option<uuid::Uuid> {
    sqlx::query_scalar::<_, uuid::Uuid>(
        "SELECT property_id FROM pos.revenue_centers WHERE env_id = $1 LIMIT 1",
    )
    .bind(env_id)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
}

pub async fn set_tenant_context(conn: &mut sqlx::PgConnection, env_id: uuid::Uuid) {
    let _ = sqlx::query("SELECT sentin8_set_tenant_context($1)")
        .bind(env_id)
        .execute(conn)
        .await;
}

pub fn resolve_org_env(auth: &AuthContext) -> uuid::Uuid {
    // env_id is the sole tenant key (globally unique). org_id has been
    // removed from all hotel8 tables - RLS context is set via env_id alone.
    auth.env_id()
}

pub fn err_db(e: sqlx::Error) -> (axum::http::StatusCode, String) {
    (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}

/// Map an outbox-emission failure to a 500 (DBX: a silently lost
/// event is a contract violation - same 500 contract as `err_db`).
fn err_emit(e: sentin8::bus::outbox::OutboxError) -> (axum::http::StatusCode, String) {
    (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}

pub fn err_not_found(msg: &str) -> (axum::http::StatusCode, String) {
    (axum::http::StatusCode::NOT_FOUND, msg.to_string())
}

#[utoipa::path(
    get,
    path = "/v1/health",
    responses(
        (status = 200, description = "OK", body = hotel8::pos::contracts::HealthDetail),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Forbidden"),
    ),
    security(),
    tag = "pos",
)]

pub async fn health(
    axum::extract::State(state): axum::extract::State<AppState>,
) -> axum::Json<hotel8::pos::contracts::HealthDetail> {
    axum::Json(hotel8::pos::contracts::HealthDetail {
        status: "ok".to_string(),
        service: "pos".to_string(),
        version: sentin8::build_info::id(),
        build: sentin8::build_info::build_hash().to_string(),
        database: if state.pool.size() > 0 {
            "connected".to_string()
        } else {
            "disconnected".to_string()
        },
    })
}
