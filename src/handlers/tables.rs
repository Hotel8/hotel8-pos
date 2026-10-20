// SPDX-FileCopyrightText: Copyright Logically Magic Inc. <hello@logicallymagic.com>
// SPDX-FileCopyrightText: Copyright Hamid Jahanzad <Hamid@Jahanzad.com>
// SPDX-FileCopyrightText: Copyright Contributors to the Hotel8:POS
// SPDX-License-Identifier: Apache-2.0

//! Table management handlers - CRUD, status, combine/separate, floor plan.
//! Implements -.
use crate::auth;
use crate::event_emit::emit_tx;
use crate::handlers::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use hotel8::pos::contracts::{
    CombineTablesRequest, CombineTablesResult, CombinedSourceTable, FloorPlan, FloorPlanArea,
    FloorPlanTable, PosTable, PosTableDeleted, PosTableList, PosTableListView,
    SeparateTablesRequest, SeparateTablesResult, TableCreateRequest, TableStatusChange,
    TableUpdateRequest,
};
use sqlx::Row;

fn err_bad_request(msg: &str) -> (StatusCode, String) {
    (StatusCode::BAD_REQUEST, msg.to_string())
}

/// The set of valid `pos_table_status` enum values.
const VALID_STATUSES: &[&str] = &[
    "AVAILABLE",
    "OCCUPIED",
    "RESERVED",
    "DIRTY",
    "OUT_OF_SERVICE",
];

fn is_valid_status(s: &str) -> bool {
    VALID_STATUSES.contains(&s)
}

// ═══════════════════════════════════════════════════════════════════════════
// Table CRUD
// ═══════════════════════════════════════════════════════════════════════════
#[utoipa::path(
    post,
    path = "/v1/outlets/{outlet_id}/tables",
    request_body = TableCreateRequest,
    responses(
        (status = 200, description = "Created table", body = PosTable),
        (status = 400, description = "service_area_id/table_number missing"),
        (status = 404, description = "Service area not found for this outlet"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Forbidden"),
    ),
    security(),
    tag = "pos",
)]

pub async fn create_table(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path(outlet_id): Path<uuid::Uuid>,
    Json(req): Json<TableCreateRequest>,
) -> Result<Json<PosTable>, (StatusCode, String)> {
    let a = auth::verify_token(&state.config, &headers).await?;
    crate::policy::require(
        &state.policy,
        &a,
        &headers,
        sentin8::shield::EvalTarget::new("hotel8:pos:admin"),
    )
    .await?;
    let env_id = super::resolve_org_env(&a);
    let mut conn = state.pool.begin().await.map_err(super::err_db)?;
    super::set_tenant_context(&mut conn, env_id).await;

    let service_area_id = req.service_area_id;
    let table_number = req.table_number.clone();
    let table_name = req.table_name.clone();
    let capacity: i32 = req.capacity;

    // Verify the service area belongs to this outlet.
    let _ =
        sqlx::query("SELECT id FROM pos.service_areas WHERE id = $1 AND revenue_center_id = $2")
            .bind(service_area_id)
            .bind(outlet_id)
            .fetch_one(&mut *conn)
            .await
            .map_err(|e| match e {
                sqlx::Error::RowNotFound => {
                    super::err_not_found("Service area not found for this outlet")
                }
                e => super::err_db(e),
            })?;

    let row = sqlx::query(
        "INSERT INTO pos.tables (service_area_id, table_number, table_name, capacity) \
         VALUES ($1, $2, $3, $4) \
         RETURNING id, service_area_id, table_number, table_name, capacity, status::text as status, current_check_id, created_at, updated_at",
    )
    .bind(service_area_id)
    .bind(&table_number)
    .bind(&table_name)
    .bind(capacity)
    .fetch_one(&mut *conn)
    .await
    .map_err(super::err_db)?;

    let id: uuid::Uuid = row.get("id");
    let status: String = row.get("status");
    emit_tx(
        &mut *conn,
        env_id,
        "hotel8:pos:table:created",
        serde_json::json!({ "table_id": id, "outlet_id": outlet_id, "table_number": table_number }),
        &crate::event_emit::Publisher::from_parts(&a.claims.sub, &a.claims.scope),
    )
    .await
    .map_err(super::err_emit)?;

    conn.commit().await.map_err(super::err_db)?;

    Ok(Json(PosTable {
        id,
        outlet_id,
        service_area_id: row.get::<uuid::Uuid, _>("service_area_id"),
        table_number: row.get::<String, _>("table_number"),
        table_name: row.get::<Option<String>, _>("table_name"),
        capacity: row.get::<i32, _>("capacity"),
        status,
        current_check_id: row.get::<Option<uuid::Uuid>, _>("current_check_id"),
        created_at: row.get::<chrono::DateTime<chrono::Utc>, _>("created_at"),
        updated_at: row.get::<chrono::DateTime<chrono::Utc>, _>("updated_at"),
    }))
}
#[utoipa::path(
    get,
    path = "/v1/outlets/{outlet_id}/tables",
    responses(
        (status = 200, description = "Table rows", body = PosTableList),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Forbidden"),
    ),
    security(),
    tag = "pos",
)]

pub async fn list_tables(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path(outlet_id): Path<uuid::Uuid>,
    axum::extract::Query(params): axum::extract::Query<serde_json::Value>,
) -> Result<Json<PosTableList>, (StatusCode, String)> {
    let a = auth::verify_token(&state.config, &headers).await?;
    crate::policy::require(
        &state.policy,
        &a,
        &headers,
        sentin8::shield::EvalTarget::new("hotel8:pos:admin"),
    )
    .await?;
    let env_id = super::resolve_org_env(&a);
    let mut conn = state.pool.begin().await.map_err(super::err_db)?;
    super::set_tenant_context(&mut conn, env_id).await;

    let service_area_id = params
        .get("service_area_id")
        .and_then(|v| v.as_str())
        .and_then(|s| uuid::Uuid::parse_str(s).ok());

    let rows = if let Some(sa_id) = service_area_id {
        sqlx::query(
            "SELECT t.id, t.service_area_id, t.table_number, t.table_name, t.capacity, \
                    t.status::text as status, t.current_check_id, t.created_at, t.updated_at \
             FROM pos.tables t \
             JOIN pos.service_areas sa ON t.service_area_id = sa.id \
             WHERE sa.revenue_center_id = $1 AND t.service_area_id = $2 \
             ORDER BY t.table_number",
        )
        .bind(outlet_id)
        .bind(sa_id)
        .fetch_all(&mut *conn)
        .await
        .map_err(super::err_db)?
    } else {
        sqlx::query(
            "SELECT t.id, t.service_area_id, t.table_number, t.table_name, t.capacity, \
                    t.status::text as status, t.current_check_id, t.created_at, t.updated_at \
             FROM pos.tables t \
             JOIN pos.service_areas sa ON t.service_area_id = sa.id \
             WHERE sa.revenue_center_id = $1 \
             ORDER BY t.table_number",
        )
        .bind(outlet_id)
        .fetch_all(&mut *conn)
        .await
        .map_err(super::err_db)?
    };

    let items: Vec<PosTableListView> = rows
        .iter()
        .map(|r| {
            let status: String = r.get("status");
            PosTableListView {
                id: r.get::<uuid::Uuid, _>("id"),
                outlet_id,
                service_area_id: r.get::<uuid::Uuid, _>("service_area_id"),
                table_number: r.get::<String, _>("table_number"),
                table_name: r.get::<Option<String>, _>("table_name"),
                capacity: r.get::<i32, _>("capacity"),
                status,
                current_check_id: r.get::<Option<uuid::Uuid>, _>("current_check_id"),
                created_at: r.get::<chrono::DateTime<chrono::Utc>, _>("created_at"),
                updated_at: r.get::<chrono::DateTime<chrono::Utc>, _>("updated_at"),
            }
        })
        .collect();

    conn.commit().await.map_err(super::err_db)?;

    Ok(Json(PosTableList { items }))
}
#[utoipa::path(
    get,
    path = "/v1/outlets/{outlet_id}/tables/{table_id}",
    responses(
        (status = 200, description = "Table row", body = PosTable),
        (status = 404, description = "Table not found"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Forbidden"),
    ),
    security(),
    tag = "pos",
)]

pub async fn get_table(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path((outlet_id, table_id)): Path<(uuid::Uuid, uuid::Uuid)>,
) -> Result<Json<PosTable>, (StatusCode, String)> {
    let a = auth::verify_token(&state.config, &headers).await?;
    crate::policy::require(
        &state.policy,
        &a,
        &headers,
        sentin8::shield::EvalTarget::new("hotel8:pos:admin"),
    )
    .await?;
    let env_id = super::resolve_org_env(&a);
    let mut conn = state.pool.begin().await.map_err(super::err_db)?;
    super::set_tenant_context(&mut conn, env_id).await;

    let row = sqlx::query(
        "SELECT t.id, t.service_area_id, t.table_number, t.table_name, t.capacity, \
                t.status::text as status, t.current_check_id, t.created_at, t.updated_at \
         FROM pos.tables t \
         JOIN pos.service_areas sa ON t.service_area_id = sa.id \
         WHERE sa.revenue_center_id = $1 AND t.id = $2",
    )
    .bind(outlet_id)
    .bind(table_id)
    .fetch_one(&mut *conn)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => super::err_not_found("Table not found"),
        e => super::err_db(e),
    })?;

    let status: String = row.get("status");
    conn.commit().await.map_err(super::err_db)?;
    Ok(Json(PosTable {
        id: row.get::<uuid::Uuid, _>("id"),
        outlet_id,
        service_area_id: row.get::<uuid::Uuid, _>("service_area_id"),
        table_number: row.get::<String, _>("table_number"),
        table_name: row.get::<Option<String>, _>("table_name"),
        capacity: row.get::<i32, _>("capacity"),
        status,
        current_check_id: row.get::<Option<uuid::Uuid>, _>("current_check_id"),
        created_at: row.get::<chrono::DateTime<chrono::Utc>, _>("created_at"),
        updated_at: row.get::<chrono::DateTime<chrono::Utc>, _>("updated_at"),
    }))
}
#[utoipa::path(
    put,
    path = "/v1/outlets/{outlet_id}/tables/{table_id}",
    request_body = TableUpdateRequest,
    responses(
        (status = 200, description = "Updated table", body = PosTable),
        (status = 404, description = "Table not found"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Forbidden"),
    ),
    security(),
    tag = "pos",
)]

pub async fn update_table(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path((outlet_id, table_id)): Path<(uuid::Uuid, uuid::Uuid)>,
    Json(req): Json<TableUpdateRequest>,
) -> Result<Json<PosTable>, (StatusCode, String)> {
    let a = auth::verify_token(&state.config, &headers).await?;
    crate::policy::require(
        &state.policy,
        &a,
        &headers,
        sentin8::shield::EvalTarget::new("hotel8:pos:admin"),
    )
    .await?;
    let env_id = super::resolve_org_env(&a);
    let mut conn = state.pool.begin().await.map_err(super::err_db)?;
    super::set_tenant_context(&mut conn, env_id).await;

    // Ensure the table exists and belongs to this outlet.
    let _ = sqlx::query(
        "SELECT t.id FROM pos.tables t \
         JOIN pos.service_areas sa ON t.service_area_id = sa.id \
         WHERE sa.revenue_center_id = $1 AND t.id = $2",
    )
    .bind(outlet_id)
    .bind(table_id)
    .fetch_one(&mut *conn)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => super::err_not_found("Table not found"),
        e => super::err_db(e),
    })?;

    let table_name = req.table_name.clone();
    let capacity: Option<i32> = req.capacity;
    let status = req.status.clone();

    // Validate status if provided.
    if let Some(st) = status.as_deref() {
        if !is_valid_status(st) {
            return Err(err_bad_request(
                "status must be one of AVAILABLE, OCCUPIED, RESERVED, DIRTY, OUT_OF_SERVICE",
            ));
        }
    }

    let row = sqlx::query(
        "UPDATE pos.tables \
         SET table_name = COALESCE($3, table_name), \
             capacity = COALESCE($4, capacity), \
             status = COALESCE($5::pos.table_status, status), \
             updated_at = now() \
         WHERE id = $2 \
         RETURNING id, service_area_id, table_number, table_name, capacity, \
                  status::text as status, current_check_id, created_at, updated_at",
    )
    .bind(outlet_id)
    .bind(table_id)
    .bind(table_name)
    .bind(capacity)
    .bind(status)
    .fetch_one(&mut *conn)
    .await
    .map_err(super::err_db)?;

    let id: uuid::Uuid = row.get("id");
    let new_status: String = row.get("status");
    emit_tx(
        &mut *conn,
        env_id,
        "hotel8:pos:table:updated",
        serde_json::json!({ "table_id": id, "outlet_id": outlet_id, "status": new_status }),
        &crate::event_emit::Publisher::from_parts(&a.claims.sub, &a.claims.scope),
    )
    .await
    .map_err(super::err_emit)?;

    conn.commit().await.map_err(super::err_db)?;

    Ok(Json(PosTable {
        id,
        outlet_id,
        service_area_id: row.get::<uuid::Uuid, _>("service_area_id"),
        table_number: row.get::<String, _>("table_number"),
        table_name: row.get::<Option<String>, _>("table_name"),
        capacity: row.get::<i32, _>("capacity"),
        status: new_status,
        current_check_id: row.get::<Option<uuid::Uuid>, _>("current_check_id"),
        created_at: row.get::<chrono::DateTime<chrono::Utc>, _>("created_at"),
        updated_at: row.get::<chrono::DateTime<chrono::Utc>, _>("updated_at"),
    }))
}
#[utoipa::path(
    delete,
    path = "/v1/outlets/{outlet_id}/tables/{table_id}",
    responses(
        (status = 200, description = "Deletion acknowledgement", body = PosTableDeleted),
        (status = 400, description = "Cannot delete table with an open check"),
        (status = 404, description = "Table not found"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Forbidden"),
    ),
    security(),
    tag = "pos",
)]

pub async fn delete_table(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path((outlet_id, table_id)): Path<(uuid::Uuid, uuid::Uuid)>,
) -> Result<Json<PosTableDeleted>, (StatusCode, String)> {
    let a = auth::verify_token(&state.config, &headers).await?;
    crate::policy::require(
        &state.policy,
        &a,
        &headers,
        sentin8::shield::EvalTarget::new("hotel8:pos:admin"),
    )
    .await?;
    let env_id = super::resolve_org_env(&a);
    let mut conn = state.pool.begin().await.map_err(super::err_db)?;
    super::set_tenant_context(&mut conn, env_id).await;

    // Ensure the table exists and belongs to this outlet.
    let _ = sqlx::query(
        "SELECT t.id FROM pos.tables t \
         JOIN pos.service_areas sa ON t.service_area_id = sa.id \
         WHERE sa.revenue_center_id = $1 AND t.id = $2",
    )
    .bind(outlet_id)
    .bind(table_id)
    .fetch_one(&mut *conn)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => super::err_not_found("Table not found"),
        e => super::err_db(e),
    })?;

    // Refuse deletion if there is an open check on the table.
    let open_check: Option<uuid::Uuid> = sqlx::query(
        "SELECT id FROM pos.checks \
         WHERE table_id = $1 AND revenue_center_id = $2 AND status NOT IN ('PAID', 'VOIDED', 'CLOSED') \
         LIMIT 1",
    )
    .bind(table_id)
    .bind(outlet_id)
    .fetch_optional(&mut *conn)
    .await
    .map_err(super::err_db)?
    .and_then(|r| r.get::<uuid::Uuid, _>("id").into());

    if open_check.is_some() {
        return Err(err_bad_request("Cannot delete table with an open check"));
    }

    let _ = sqlx::query("DELETE FROM pos.tables WHERE id = $1")
        .bind(table_id)
        .execute(&mut *conn)
        .await
        .map_err(super::err_db)?;

    emit_tx(
        &mut *conn,
        env_id,
        "hotel8:pos:table:deleted",
        serde_json::json!({ "table_id": table_id, "outlet_id": outlet_id }),
        &crate::event_emit::Publisher::from_parts(&a.claims.sub, &a.claims.scope),
    )
    .await
    .map_err(super::err_emit)?;

    conn.commit().await.map_err(super::err_db)?;

    Ok(Json(PosTableDeleted {
        deleted: true,
        table_id,
        outlet_id,
    }))
}

// ═══════════════════════════════════════════════════════════════════════════
// / Table status management + update
// ═══════════════════════════════════════════════════════════════════════════
#[utoipa::path(
    put,
    path = "/v1/outlets/{outlet_id}/tables/{table_id}/status",
    request_body = TableStatusChange,
    responses(
        (status = 200, description = "Updated table", body = PosTable),
        (status = 400, description = "status missing or invalid"),
        (status = 404, description = "Table not found"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Forbidden"),
    ),
    security(),
    tag = "pos",
)]

pub async fn update_table_status(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path((outlet_id, table_id)): Path<(uuid::Uuid, uuid::Uuid)>,
    Json(req): Json<TableStatusChange>,
) -> Result<Json<PosTable>, (StatusCode, String)> {
    let a = auth::verify_token(&state.config, &headers).await?;
    crate::policy::require(
        &state.policy,
        &a,
        &headers,
        sentin8::shield::EvalTarget::new("hotel8:pos:admin"),
    )
    .await?;
    let env_id = super::resolve_org_env(&a);
    let mut conn = state.pool.begin().await.map_err(super::err_db)?;
    super::set_tenant_context(&mut conn, env_id).await;

    let status = req.status.clone();
    if !is_valid_status(&status) {
        return Err(err_bad_request(
            "status must be one of AVAILABLE, OCCUPIED, RESERVED, DIRTY, OUT_OF_SERVICE",
        ));
    }

    // Ensure the table exists and belongs to this outlet.
    let _ = sqlx::query(
        "SELECT t.id FROM pos.tables t \
         JOIN pos.service_areas sa ON t.service_area_id = sa.id \
         WHERE sa.revenue_center_id = $1 AND t.id = $2",
    )
    .bind(outlet_id)
    .bind(table_id)
    .fetch_one(&mut *conn)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => super::err_not_found("Table not found"),
        e => super::err_db(e),
    })?;

    let current_check_id = if status == "OCCUPIED" {
        // When setting to OCCUPIED, optionally accept a current_check_id.
        req.current_check_id
    } else if status == "AVAILABLE" {
        // When setting to AVAILABLE, clear the current check.
        None
    } else {
        // For other statuses, leave the current check unchanged.
        // We use a scout below to indicate "no change".
        return update_table_status_no_check_change(
            &state,
            &mut conn,
            env_id,
            outlet_id,
            table_id,
            status.as_str(),
            &crate::event_emit::Publisher::from_parts(&a.claims.sub, &a.claims.scope),
        )
        .await;
    };

    let row = sqlx::query(
        "UPDATE pos.tables \
         SET status = $3::pos.table_status, \
             current_check_id = CASE WHEN $3 = 'AVAILABLE' THEN NULL ELSE $4 END, \
             updated_at = now() \
         WHERE id = $2 \
         RETURNING id, service_area_id, table_number, table_name, capacity, \
                  status::text as status, current_check_id, created_at, updated_at",
    )
    .bind(outlet_id)
    .bind(table_id)
    .bind(status)
    .bind(current_check_id)
    .fetch_one(&mut *conn)
    .await
    .map_err(super::err_db)?;

    let new_status: String = row.get("status");
    emit_tx(
        &mut *conn,
        env_id,
        "hotel8:pos:table:status_changed",
        serde_json::json!({
            "table_id": table_id,
            "outlet_id": outlet_id,
            "status": new_status,
            "current_check_id": row.get::<Option<uuid::Uuid>, _>("current_check_id"),
        }),
        &crate::event_emit::Publisher::from_parts(&a.claims.sub, &a.claims.scope),
    )
    .await
    .map_err(super::err_emit)?;

    conn.commit().await.map_err(super::err_db)?;

    Ok(Json(PosTable {
        id: row.get::<uuid::Uuid, _>("id"),
        outlet_id,
        service_area_id: row.get::<uuid::Uuid, _>("service_area_id"),
        table_number: row.get::<String, _>("table_number"),
        table_name: row.get::<Option<String>, _>("table_name"),
        capacity: row.get::<i32, _>("capacity"),
        status: new_status,
        current_check_id: row.get::<Option<uuid::Uuid>, _>("current_check_id"),
        created_at: row.get::<chrono::DateTime<chrono::Utc>, _>("created_at"),
        updated_at: row.get::<chrono::DateTime<chrono::Utc>, _>("updated_at"),
    }))
}

/// Helper for status changes that do not alter `current_check_id` (RESERVED, DIRTY, OUT_OF_SERVICE).
async fn update_table_status_no_check_change(
    _state: &AppState,
    conn: &mut sqlx::PgConnection,
    env_id: uuid::Uuid,
    outlet_id: uuid::Uuid,
    table_id: uuid::Uuid,
    status: &str,
    publisher: &crate::event_emit::Publisher,
) -> Result<Json<PosTable>, (StatusCode, String)> {
    let row = sqlx::query(
        "UPDATE pos.tables \
         SET status = $3::pos.table_status, \
             updated_at = now() \
         WHERE id = $2 \
         RETURNING id, service_area_id, table_number, table_name, capacity, \
                  status::text as status, current_check_id, created_at, updated_at",
    )
    .bind(outlet_id)
    .bind(table_id)
    .bind(status)
    .fetch_one(&mut *conn)
    .await
    .map_err(super::err_db)?;

    let new_status: String = row.get("status");
    emit_tx(
        &mut *conn,
        env_id,
        "hotel8:pos:table:status_changed",
        serde_json::json!({
            "table_id": table_id,
            "outlet_id": outlet_id,
            "status": new_status,
            "current_check_id": row.get::<Option<uuid::Uuid>, _>("current_check_id"),
        }),
        publisher,
    )
    .await
    .map_err(super::err_emit)?;

    Ok(Json(PosTable {
        id: row.get::<uuid::Uuid, _>("id"),
        outlet_id,
        service_area_id: row.get::<uuid::Uuid, _>("service_area_id"),
        table_number: row.get::<String, _>("table_number"),
        table_name: row.get::<Option<String>, _>("table_name"),
        capacity: row.get::<i32, _>("capacity"),
        status: new_status,
        current_check_id: row.get::<Option<uuid::Uuid>, _>("current_check_id"),
        created_at: row.get::<chrono::DateTime<chrono::Utc>, _>("created_at"),
        updated_at: row.get::<chrono::DateTime<chrono::Utc>, _>("updated_at"),
    }))
}

// ═══════════════════════════════════════════════════════════════════════════
// Table Combine
// ═══════════════════════════════════════════════════════════════════════════
#[utoipa::path(
    post,
    path = "/v1/outlets/{outlet_id}/tables/combine",
    request_body = CombineTablesRequest,
    responses(
        (status = 200, description = "Combined table with source rows", body = CombineTablesResult),
        (status = 400, description = "table_ids/combined_name missing or table_ids < 2"),
        (status = 404, description = "One or more tables not found"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Forbidden"),
    ),
    security(),
    tag = "pos",
)]

pub async fn combine_tables(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path(outlet_id): Path<uuid::Uuid>,
    Json(req): Json<CombineTablesRequest>,
) -> Result<Json<CombineTablesResult>, (StatusCode, String)> {
    let a = auth::verify_token(&state.config, &headers).await?;
    crate::policy::require(
        &state.policy,
        &a,
        &headers,
        sentin8::shield::EvalTarget::new("hotel8:pos:admin"),
    )
    .await?;
    let env_id = super::resolve_org_env(&a);
    let mut conn = state.pool.begin().await.map_err(super::err_db)?;
    super::set_tenant_context(&mut conn, env_id).await;

    let table_ids: Vec<uuid::Uuid> = req.table_ids.clone();

    if table_ids.len() < 2 {
        return Err(err_bad_request(
            "At least two table_ids are required to combine",
        ));
    }

    let combined_name = req.combined_name.clone();

    // Fetch all the source tables (validate they belong to this outlet).
    let rows = sqlx::query(
        "SELECT t.id, t.service_area_id, t.table_number, t.table_name, t.capacity, \
                t.status::text as status \
         FROM pos.tables t \
         JOIN pos.service_areas sa ON t.service_area_id = sa.id \
         WHERE sa.revenue_center_id = $1 AND t.id = ANY($2)",
    )
    .bind(outlet_id)
    .bind(&table_ids)
    .fetch_all(&mut *conn)
    .await
    .map_err(super::err_db)?;

    if rows.len() != table_ids.len() {
        return Err(super::err_not_found(
            "One or more tables not found for this outlet",
        ));
    }

    // Use the first table's service area for the combined table.
    let service_area_id: uuid::Uuid = rows[0].get("service_area_id");
    let total_capacity: i32 = rows.iter().map(|r| r.get::<i32, _>("capacity")).sum();
    let combined_table_number = format!("COMBINED-{}", chrono::Utc::now().format("%Y%m%d%H%M%S"));

    let source_tables: Vec<CombinedSourceTable> = rows
        .iter()
        .map(|r| {
            let st: String = r.get("status");
            CombinedSourceTable {
                id: r.get::<uuid::Uuid, _>("id"),
                table_number: r.get::<String, _>("table_number"),
                table_name: r.get::<Option<String>, _>("table_name"),
                capacity: r.get::<i32, _>("capacity"),
                status: st,
            }
        })
        .collect();

    // Create a new table entry representing the combined tables.
    let row = sqlx::query(
        "INSERT INTO pos.tables (service_area_id, table_number, table_name, capacity, status) \
         VALUES ($1, $2, $3, $4, 'OCCUPIED'::pos.table_status) \
         RETURNING id, service_area_id, table_number, table_name, capacity, \
                  status::text as status, current_check_id, created_at, updated_at",
    )
    .bind(service_area_id)
    .bind(&combined_table_number)
    .bind(&combined_name)
    .bind(total_capacity)
    .fetch_one(&mut *conn)
    .await
    .map_err(super::err_db)?;

    let combined_id: uuid::Uuid = row.get("id");
    let status: String = row.get("status");

    emit_tx(
        &mut *conn,
        env_id,
        "hotel8:pos:tables:combined",
        serde_json::json!({
            "outlet_id": outlet_id,
            "combined_table_id": combined_id,
            "combined_name": combined_name,
            "source_table_ids": table_ids,
            "service_area_id": service_area_id,
        }),
        &crate::event_emit::Publisher::from_parts(&a.claims.sub, &a.claims.scope),
    )
    .await
    .map_err(super::err_emit)?;

    conn.commit().await.map_err(super::err_db)?;

    Ok(Json(CombineTablesResult {
        combined_table_id: combined_id,
        outlet_id,
        service_area_id,
        table_number: row.get::<String, _>("table_number"),
        table_name: row.get::<Option<String>, _>("table_name"),
        capacity: row.get::<i32, _>("capacity"),
        status,
        current_check_id: row.get::<Option<uuid::Uuid>, _>("current_check_id"),
        source_tables,
        created_at: row.get::<chrono::DateTime<chrono::Utc>, _>("created_at"),
        updated_at: row.get::<chrono::DateTime<chrono::Utc>, _>("updated_at"),
    }))
}

// ═══════════════════════════════════════════════════════════════════════════
// Table Separate
// ═══════════════════════════════════════════════════════════════════════════
#[utoipa::path(
    post,
    path = "/v1/outlets/{outlet_id}/tables/separate",
    request_body = SeparateTablesRequest,
    responses(
        (status = 200, description = "Separation acknowledgement", body = SeparateTablesResult),
        (status = 400, description = "table_id missing"),
        (status = 404, description = "Table not found"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Forbidden"),
    ),
    security(),
    tag = "pos",
)]

pub async fn separate_tables(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path(outlet_id): Path<uuid::Uuid>,
    Json(req): Json<SeparateTablesRequest>,
) -> Result<Json<SeparateTablesResult>, (StatusCode, String)> {
    let a = auth::verify_token(&state.config, &headers).await?;
    crate::policy::require(
        &state.policy,
        &a,
        &headers,
        sentin8::shield::EvalTarget::new("hotel8:pos:admin"),
    )
    .await?;
    let env_id = super::resolve_org_env(&a);
    let mut conn = state.pool.begin().await.map_err(super::err_db)?;
    super::set_tenant_context(&mut conn, env_id).await;

    let table_id: uuid::Uuid = req.table_id;

    // Fetch the combined table (validate it belongs to this outlet).
    let row = sqlx::query(
        "SELECT t.id, t.service_area_id, t.table_number, t.table_name, t.capacity, \
                t.status::text as status \
         FROM pos.tables t \
         JOIN pos.service_areas sa ON t.service_area_id = sa.id \
         WHERE sa.revenue_center_id = $1 AND t.id = $2",
    )
    .bind(outlet_id)
    .bind(table_id)
    .fetch_one(&mut *conn)
    .await
    .map_err(|e| match e {
        sqlx::Error::RowNotFound => super::err_not_found("Table not found"),
        e => super::err_db(e),
    })?;

    let table_number: String = row.get("table_number");

    // Delete the combined table entry and reset the source tables to AVAILABLE.
    let _ = sqlx::query("DELETE FROM pos.tables WHERE id = $1")
        .bind(table_id)
        .execute(&mut *conn)
        .await
        .map_err(super::err_db)?;

    emit_tx(
        &mut *conn,
        env_id,
        "hotel8:pos:tables:separated",
        serde_json::json!({
            "outlet_id": outlet_id,
            "combined_table_id": table_id,
            "table_number": table_number,
        }),
        &crate::event_emit::Publisher::from_parts(&a.claims.sub, &a.claims.scope),
    )
    .await
    .map_err(super::err_emit)?;

    conn.commit().await.map_err(super::err_db)?;

    Ok(Json(SeparateTablesResult {
        separated: true,
        outlet_id,
        combined_table_id: table_id,
        table_number,
    }))
}

// ═══════════════════════════════════════════════════════════════════════════
// Floor Plan
// ═══════════════════════════════════════════════════════════════════════════
#[utoipa::path(
    get,
    path = "/v1/outlets/{outlet_id}/tables/floor-plan",
    responses(
        (status = 200, description = "Floor plan (x/y are null - no position columns)", body = FloorPlan),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Forbidden"),
    ),
    security(),
    tag = "pos",
)]

pub async fn get_floor_plan(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path(outlet_id): Path<uuid::Uuid>,
    axum::extract::Query(params): axum::extract::Query<serde_json::Value>,
) -> Result<Json<FloorPlan>, (StatusCode, String)> {
    let a = auth::verify_token(&state.config, &headers).await?;
    crate::policy::require(
        &state.policy,
        &a,
        &headers,
        sentin8::shield::EvalTarget::new("hotel8:pos:admin"),
    )
    .await?;
    let env_id = super::resolve_org_env(&a);
    let mut conn = state.pool.begin().await.map_err(super::err_db)?;
    super::set_tenant_context(&mut conn, env_id).await;

    let service_area_id = params
        .get("service_area_id")
        .and_then(|v| v.as_str())
        .and_then(|s| uuid::Uuid::parse_str(s).ok());

    // Fetch service areas for this outlet.
    let sa_rows = if let Some(sa_id) = service_area_id {
        sqlx::query(
            "SELECT id, area_code, area_name, is_active \
             FROM pos.service_areas \
             WHERE revenue_center_id = $1 AND id = $2 \
             ORDER BY area_code",
        )
        .bind(outlet_id)
        .bind(sa_id)
        .fetch_all(&mut *conn)
        .await
        .map_err(super::err_db)?
    } else {
        sqlx::query(
            "SELECT id, area_code, area_name, is_active \
             FROM pos.service_areas \
             WHERE revenue_center_id = $1 \
             ORDER BY area_code",
        )
        .bind(outlet_id)
        .fetch_all(&mut *conn)
        .await
        .map_err(super::err_db)?
    };

    // Fetch tables for this outlet (optionally filtered by service area).
    let table_rows = if let Some(sa_id) = service_area_id {
        sqlx::query(
            "SELECT t.id, t.service_area_id, t.table_number, t.table_name, t.capacity, \
                    t.status::text as status, t.current_check_id \
             FROM pos.tables t \
             JOIN pos.service_areas sa ON t.service_area_id = sa.id \
             WHERE sa.revenue_center_id = $1 AND t.service_area_id = $2 \
             ORDER BY t.table_number",
        )
        .bind(outlet_id)
        .bind(sa_id)
        .fetch_all(&mut *conn)
        .await
        .map_err(super::err_db)?
    } else {
        sqlx::query(
            "SELECT t.id, t.service_area_id, t.table_number, t.table_name, t.capacity, \
                    t.status::text as status, t.current_check_id \
             FROM pos.tables t \
             JOIN pos.service_areas sa ON t.service_area_id = sa.id \
             WHERE sa.revenue_center_id = $1 \
             ORDER BY t.table_number",
        )
        .bind(outlet_id)
        .fetch_all(&mut *conn)
        .await
        .map_err(super::err_db)?
    };

    // Group tables by service area. `pos_tables` has no position columns, so
    // we emit `x` and `y` as null.
    let mut areas: Vec<FloorPlanArea> = Vec::new();
    for sa in &sa_rows {
        let sa_id: uuid::Uuid = sa.get("id");
        let tables: Vec<FloorPlanTable> = table_rows
            .iter()
            .filter(|t| t.get::<uuid::Uuid, _>("service_area_id") == sa_id)
            .map(|t| {
                let st: String = t.get("status");
                FloorPlanTable {
                    id: t.get::<uuid::Uuid, _>("id"),
                    table_number: t.get::<String, _>("table_number"),
                    table_name: t.get::<Option<String>, _>("table_name"),
                    capacity: t.get::<i32, _>("capacity"),
                    status: st,
                    current_check_id: t.get::<Option<uuid::Uuid>, _>("current_check_id"),
                    x: None,
                    y: None,
                }
            })
            .collect();

        areas.push(FloorPlanArea {
            id: sa_id,
            area_code: sa.get::<String, _>("area_code"),
            area_name: sa.get::<String, _>("area_name"),
            is_active: sa.get::<bool, _>("is_active"),
            tables,
        });
    }

    conn.commit().await.map_err(super::err_db)?;

    Ok(Json(FloorPlan {
        outlet_id,
        service_areas: areas,
    }))
}
