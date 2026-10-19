// SPDX-FileCopyrightText: Copyright Logically Magic Inc. <hello@logicallymagic.com>
// SPDX-FileCopyrightText: Copyright Hamid Jahanzad <Hamid@Jahanzad.com>
// SPDX-FileCopyrightText: Copyright Contributors to the Hotel8:POS
// SPDX-License-Identifier: Apache-2.0

//! Order handlers - create checks, add items, send to kitchen.
use crate::auth;
use crate::check_state::{CheckState, VoidError};
use crate::event_emit::emit_tx;
use crate::handlers::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use hotel8::pos::contracts::{
    AddedOrderLine, AddOrderItemRequest, AssignSeatRequest, CheckCreated, CheckCreateRequest,
    CheckDetail, CheckList, CheckStateChange, CheckSummary, CheckVoidResult, OrderItemVoidResult,
    OrderLine, SeatAssigned, SplitCheckRequest, SplitCheckResult, SplitPart, TransferCheckRequest,
    TransferCheckResult, VoidCheckRequest, VoidOrderItemRequest,
};
use sqlx::Row;

/// `POST /v1/outlets/{outlet_id}/checks`
#[utoipa::path(
    post,
    path = "/v1/outlets/{outlet_id}/checks",
    request_body = CheckCreateRequest,
    responses(
        (status = 200, description = "Check created", body = CheckCreated),
        (status = 403, description = "Missing pos:admin scope"),
    ),
    security(("pos:admin" = [])),
    tag = "pos",
)]
pub async fn create_check(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path(outlet_id): Path<uuid::Uuid>,
    Json(req): Json<CheckCreateRequest>,
) -> Result<Json<CheckCreated>, (StatusCode, String)> {
    let a = auth::verify_token(&state.config, &headers).await?;
    crate::policy::require(
        &state.policy,
        &a,
        &headers,
        sentin8::shield::EvalTarget::new("hotel8:pos:order:write"),
    )
    .await?;
    let env_id = super::resolve_org_env(&a);
    let mut conn = state.pool.begin().await.map_err(super::err_db)?;
    super::set_tenant_context(&mut conn, env_id).await;
    let check_number = format!("CHK-{}", chrono::Utc::now().format("%Y%m%d%H%M%S"));
    let table_id = req.table_id;
    let guest_name = req.guest_name.clone();
    let guest_count = req.guest_count;
    let room_number = req.room_number.clone();
    let row = sqlx::query("INSERT INTO pos.checks (revenue_center_id, table_id, check_number, guest_name, guest_count, room_number) VALUES ($1, $2, $3, $4, $5, $6) RETURNING id, check_number, status")
        .bind(outlet_id)
        .bind(table_id)
        .bind(&check_number)
        .bind(&guest_name)
        .bind(guest_count)
        .bind(&room_number)
        .fetch_one(&mut *conn)
        .await
        .map_err(super::err_db)?;
    let check_id: uuid::Uuid = row.get("id");
    let status: String = row.get("status");
    emit_tx(
        &mut *conn,
        env_id,
        "hotel8:pos:check:created",
        serde_json::json!({ "check_id": check_id, "check_number": &check_number }),
        &crate::event_emit::Publisher::from_parts(&a.claims.sub, &a.claims.scope),
    )
    .await
    .map_err(super::err_emit)?;
    conn.commit().await.map_err(super::err_db)?;
    Ok(Json(CheckCreated {
        id: check_id,
        outlet_id,
        check_number,
        status,
    }))
}
#[utoipa::path(
    post,
    path = "/v1/outlets/{outlet_id}/checks/{check_id}/orders",
    request_body = AddOrderItemRequest,
    responses(
        (status = 200, description = "Added order line", body = AddedOrderLine),
        (status = 400, description = "menu_item_id missing/invalid"),
        (status = 404, description = "Menu item not found"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Forbidden"),
    ),
    security(),
    tag = "pos",
)]

pub async fn add_order_item(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path((outlet_id, check_id)): Path<(uuid::Uuid, uuid::Uuid)>,
    Json(req): Json<AddOrderItemRequest>,
) -> Result<Json<AddedOrderLine>, (StatusCode, String)> {
    let a = auth::verify_token(&state.config, &headers).await?;
    crate::policy::require(
        &state.policy,
        &a,
        &headers,
        sentin8::shield::EvalTarget::new("hotel8:pos:order:write"),
    )
    .await?;
    let env_id = super::resolve_org_env(&a);
    let mut conn = state.pool.begin().await.map_err(super::err_db)?;
    super::set_tenant_context(&mut conn, env_id).await;

    let menu_item_id = req.menu_item_id;
    let quantity = req.quantity;

    // Get menu item price
    let item = sqlx::query("SELECT item_name, price::text as price FROM pos.menu_items WHERE id = $1 AND revenue_center_id = $2")
        .bind(menu_item_id)
        .bind(outlet_id)
        .fetch_one(&mut *conn)
        .await
        .map_err(|e| match e {
            sqlx::Error::RowNotFound => super::err_not_found("Menu item not found"),
            e => super::err_db(e),
        })?;
    let item_name: String = item.get("item_name");
    let price_str: String = item.get("price");
    let unit_price: rust_decimal::Decimal = price_str.parse().unwrap_or_default();
    let line_total = unit_price * rust_decimal::Decimal::from(quantity);

    let row = sqlx::query("INSERT INTO pos.orders (check_id, menu_item_id, quantity, unit_price, line_total, modifiers, special_instructions, sent_at) VALUES ($1, $2, $3, $4, $5, $6, $7, now()) RETURNING id, quantity, line_total::text as line_total")
        .bind(check_id)
        .bind(menu_item_id)
        .bind(quantity)
        .bind(unit_price.to_string())
        .bind(line_total.to_string())
        .bind(req.modifiers.clone())
        .bind(req.special_instructions.as_deref())
        .fetch_one(&mut *conn)
        .await
        .map_err(super::err_db)?;
    let order_id: uuid::Uuid = row.get("id");
    let lt: String = row.get("line_total");

    // Update check totals
    let _ = sqlx::query("UPDATE pos.checks SET subtotal = subtotal + $2, total = total + $2, updated_at = now() WHERE id = $1")
        .bind(check_id)
        .bind(line_total.to_string())
        .execute(&mut *conn)
        .await;

    emit_tx(
        &mut *conn,
        env_id,
        "hotel8:pos:order:added",
        serde_json::json!({ "check_id": check_id, "order_id": order_id, "item_name": &item_name }),
        &crate::event_emit::Publisher::from_parts(&a.claims.sub, &a.claims.scope),
    )
    .await
    .map_err(super::err_emit)?;
    conn.commit().await.map_err(super::err_db)?;
    Ok(Json(AddedOrderLine {
        id: order_id,
        check_id,
        menu_item_id,
        item_name,
        quantity,
        line_total: lt,
        status: "SENT".to_string(),
    }))
}

/// `GET /v1/outlets/{outlet_id}/checks/{check_id}`
#[utoipa::path(
    get,
    path = "/v1/outlets/{outlet_id}/checks/{check_id}",
    params(("check_id" = uuid::Uuid, Path, description = "Check ID")),
    responses(
        (status = 200, description = "Check details", body = CheckDetail),
        (status = 404, description = "Check not found"),
        (status = 403, description = "Missing pos:admin scope"),
    ),
    security(("pos:admin" = [])),
    tag = "pos",
)]
pub async fn get_check(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path((outlet_id, check_id)): Path<(uuid::Uuid, uuid::Uuid)>,
) -> Result<Json<CheckDetail>, (StatusCode, String)> {
    let a = auth::verify_token(&state.config, &headers).await?;
    crate::policy::require(
        &state.policy,
        &a,
        &headers,
        sentin8::shield::EvalTarget::new("hotel8:pos:order:read"),
    )
    .await?;
    let env_id = super::resolve_org_env(&a);
    let mut conn = state.pool.begin().await.map_err(super::err_db)?;
    super::set_tenant_context(&mut conn, env_id).await;
    let check = sqlx::query("SELECT id, check_number, status, guest_name, guest_count, subtotal::text as subtotal, tax_total::text as tax_total, total::text as total, room_number, opened_at FROM pos.checks WHERE id = $1 AND revenue_center_id = $2")
        .bind(check_id)
        .bind(outlet_id)
        .fetch_one(&mut *conn)
        .await
        .map_err(|e| match e {
            sqlx::Error::RowNotFound => super::err_not_found("Check not found"),
            e => super::err_db(e),
        })?;
    let status: String = check.get("status");
    let orders = sqlx::query("SELECT id, menu_item_id, quantity, unit_price::text as unit_price, line_total::text as line_total, status FROM pos.orders WHERE check_id = $1 ORDER BY created_at")
        .bind(check_id)
        .fetch_all(&mut *conn)
        .await
        .map_err(super::err_db)?;
    let order_items: Vec<OrderLine> = orders
        .iter()
        .map(|r| {
            let o_status: String = r.get("status");
            OrderLine {
                id: r.get::<uuid::Uuid, _>("id"),
                menu_item_id: r.get::<uuid::Uuid, _>("menu_item_id"),
                quantity: r.get::<i32, _>("quantity"),
                unit_price: r.get::<String, _>("unit_price"),
                line_total: r.get::<String, _>("line_total"),
                status: o_status,
            }
        })
        .collect();
    conn.commit().await.map_err(super::err_db)?;
    Ok(Json(CheckDetail {
        id: check.get::<uuid::Uuid, _>("id"),
        outlet_id,
        check_number: check.get::<String, _>("check_number"),
        status,
        guest_name: check.get::<Option<String>, _>("guest_name"),
        guest_count: check.get::<i32, _>("guest_count"),
        subtotal: check.get::<String, _>("subtotal"),
        tax_total: check.get::<String, _>("tax_total"),
        total: check.get::<String, _>("total"),
        room_number: check.get::<Option<String>, _>("room_number"),
        opened_at: check.get::<chrono::DateTime<chrono::Utc>, _>("opened_at"),
        orders: order_items,
    }))
}
// ── Check search - by table, server, status, date, check number ──

/// `GET /v1/outlets/{outlet_id}/checks`
#[utoipa::path(
    get,
    path = "/v1/outlets/{outlet_id}/checks",
    responses(
        (status = 200, description = "List of orders", body = CheckList),
        (status = 403, description = "Missing pos:admin scope"),
    ),
    security(("pos:admin" = [])),
    tag = "pos",
)]
pub async fn search_checks(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path(outlet_id): Path<uuid::Uuid>,
    axum::extract::Query(params): axum::extract::Query<serde_json::Value>,
) -> Result<Json<CheckList>, (StatusCode, String)> {
    let a = auth::verify_token(&state.config, &headers).await?;
    crate::policy::require(
        &state.policy,
        &a,
        &headers,
        sentin8::shield::EvalTarget::new("hotel8:pos:order:read"),
    )
    .await?;
    let env_id = super::resolve_org_env(&a);
    let mut conn = state.pool.begin().await.map_err(super::err_db)?;
    super::set_tenant_context(&mut conn, env_id).await;
    let limit: i64 = params.get("limit").and_then(|v| v.as_i64()).unwrap_or(100);
    // Build dynamic query based on filters
    let mut conditions = vec!["revenue_center_id = $1".to_string()];
    let mut param_idx = 2u32;
    let mut binds: Vec<(u32, serde_json::Value)> = Vec::new();
    if let Some(table_id) = params.get("table_id").and_then(|v| v.as_str()) {
        conditions.push(format!("table_id = ${}", param_idx));
        binds.push((param_idx, serde_json::Value::String(table_id.to_string())));
        param_idx += 1;
    }
    if let Some(status) = params.get("status").and_then(|v| v.as_str()) {
        conditions.push(format!("status = ${}::pos.check_status", param_idx));
        binds.push((param_idx, serde_json::Value::String(status.to_string())));
        param_idx += 1;
    }
    if let Some(check_number) = params.get("check_number").and_then(|v| v.as_str()) {
        conditions.push(format!("check_number ILIKE ${}", param_idx));
        binds.push((
            param_idx,
            serde_json::Value::String(format!("%{}%", check_number)),
        ));
        param_idx += 1;
    }
    if let Some(date) = params.get("date").and_then(|v| v.as_str()) {
        conditions.push(format!("opened_at::date = ${}::date", param_idx));
        binds.push((param_idx, serde_json::Value::String(date.to_string())));
        param_idx += 1;
    }
    let limit_param = param_idx;
    let where_clause = conditions.join(" AND ");
    let sql = format!(
        "SELECT id, check_number, status, guest_name, guest_count, subtotal::text as subtotal, total::text as total, table_id, opened_at FROM pos.checks WHERE {} ORDER BY opened_at DESC LIMIT ${}",
        where_clause, limit_param
    );
    let mut query = sqlx::query(&sql).bind(outlet_id);
    for (idx, val) in &binds {
        match val {
            serde_json::Value::String(s) => {
                query = query.bind(s);
            }
            _ => {
                query = query.bind(val.to_string());
            }
        }
        let _ = idx;
    }
    query = query.bind(limit);
    let rows = query.fetch_all(&mut *conn).await.map_err(super::err_db)?;
    let items: Vec<CheckSummary> = rows.iter().map(|r| {
        let st: String = r.get("status");
        CheckSummary {
            id: r.get::<uuid::Uuid, _>("id"),
            check_number: r.get::<String, _>("check_number"),
            status: st,
            guest_name: r.get::<Option<String>, _>("guest_name"),
            guest_count: r.get::<i32, _>("guest_count"),
            subtotal: r.get::<String, _>("subtotal"),
            total: r.get::<String, _>("total"),
            table_id: r.get::<Option<uuid::Uuid>, _>("table_id"),
            opened_at: r.get::<chrono::DateTime<chrono::Utc>, _>("opened_at"),
        }
    }).collect();
    conn.commit().await.map_err(super::err_db)?;
    Ok(Json(CheckList { items }))
}

// ── Check Transfer - to different table ──
#[utoipa::path(
    post,
    path = "/v1/outlets/{outlet_id}/checks/{check_id}/transfer",
    request_body = TransferCheckRequest,
    responses(
        (status = 200, description = "Transferred", body = TransferCheckResult),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Forbidden"),
    ),
    security(),
    tag = "pos",
)]

pub async fn transfer_check(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path((outlet_id, check_id)): Path<(uuid::Uuid, uuid::Uuid)>,
    Json(req): Json<TransferCheckRequest>,
) -> Result<Json<TransferCheckResult>, (StatusCode, String)> {
    let a = auth::verify_token(&state.config, &headers).await?;
    crate::policy::require(
        &state.policy,
        &a,
        &headers,
        sentin8::shield::EvalTarget::new("hotel8:pos:order:write"),
    )
    .await?;
    let env_id = super::resolve_org_env(&a);
    let mut conn = state.pool.begin().await.map_err(super::err_db)?;
    super::set_tenant_context(&mut conn, env_id).await;
    let new_table_id = req.new_table_id;
    // Verify the new table exists and belongs to this outlet
    let table = sqlx::query("SELECT t.id FROM pos.tables t JOIN pos.service_areas sa ON t.service_area_id = sa.id WHERE t.id = $1 AND sa.revenue_center_id = $2")
        .bind(new_table_id).bind(outlet_id)
        .fetch_optional(&mut *conn).await.map_err(super::err_db)?
        .ok_or_else(|| super::err_not_found("Target table not found"))?;
    let _ = table;
    // Update the check's table_id
    sqlx::query("UPDATE pos.checks SET table_id = $2, updated_at = now() WHERE id = $1 AND revenue_center_id = $3")
        .bind(check_id).bind(new_table_id).bind(outlet_id)
        .execute(&mut *conn).await.map_err(super::err_db)?;
    emit_tx(
        &mut *conn,
        env_id,
        "hotel8:pos:check:transferred",
        serde_json::json!({ "check_id": check_id, "new_table_id": new_table_id }),
        &crate::event_emit::Publisher::from_parts(&a.claims.sub, &a.claims.scope),
    )
    .await
    .map_err(super::err_emit)?;
    conn.commit().await.map_err(super::err_db)?;
    Ok(Json(TransferCheckResult {
        check_id,
        new_table_id,
        transferred: true,
    }))
}

// ── Check Split - by seat, by item, equally ──
#[utoipa::path(
    post,
    path = "/v1/outlets/{outlet_id}/checks/{check_id}/split",
    request_body = SplitCheckRequest,
    responses(
        (status = 200, description = "Equal split result", body = SplitCheckResult),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Forbidden"),
    ),
    security(),
    tag = "pos",
)]

pub async fn split_check(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path((outlet_id, check_id)): Path<(uuid::Uuid, uuid::Uuid)>,
    Json(req): Json<SplitCheckRequest>,
) -> Result<Json<SplitCheckResult>, (StatusCode, String)> {
    let a = auth::verify_token(&state.config, &headers).await?;
    crate::policy::require(
        &state.policy,
        &a,
        &headers,
        sentin8::shield::EvalTarget::new("hotel8:pos:order:write"),
    )
    .await?;
    let env_id = super::resolve_org_env(&a);
    let mut conn = state.pool.begin().await.map_err(super::err_db)?;
    super::set_tenant_context(&mut conn, env_id).await;
    let split_type = req.split_type.clone();
    // Get the original check
    let check = sqlx::query("SELECT id, check_number, subtotal, tax_total, total, guest_count FROM pos.checks WHERE id = $1 AND revenue_center_id = $2")
        .bind(check_id).bind(outlet_id)
        .fetch_one(&mut *conn).await.map_err(super::err_db)?;
    let total_str: String = check.get("total");
    let total: rust_decimal::Decimal = total_str.parse().unwrap_or_default();
    let num_splits: i32 = req.num_splits;
    if num_splits < 2 {
        return Err((
            StatusCode::BAD_REQUEST,
            "num_splits must be >= 2".to_string(),
        ));
    }
    // Calculate equal split
    let split_amount = total / rust_decimal::Decimal::from(num_splits);
    let mut splits: Vec<SplitPart> = Vec::new();
    for i in 1..=num_splits {
        let new_check_number = format!("{}-S{}", check.get::<String, _>("check_number"), i);
        let row = sqlx::query("INSERT INTO pos.checks (revenue_center_id, check_number, status, guest_count, subtotal, tax_total, total) VALUES ($1, $2, 'OPEN', 1, $3, 0, $4) RETURNING id, check_number")
            .bind(outlet_id).bind(&new_check_number).bind(split_amount.to_string()).bind(split_amount.to_string())
            .fetch_one(&mut *conn).await.map_err(super::err_db)?;
        splits.push(SplitPart {
            check_id: row.get::<uuid::Uuid, _>("id"),
            check_number: row.get::<String, _>("check_number"),
            amount: split_amount.to_string(),
        });
    }
    emit_tx(&mut *conn, env_id, "hotel8:pos:check:split", serde_json::json!({ "original_check_id": check_id, "split_type": split_type, "num_splits": num_splits, "splits": &splits }),
        &crate::event_emit::Publisher::from_parts(&a.claims.sub, &a.claims.scope)).await.map_err(super::err_emit)?;
    conn.commit().await.map_err(super::err_db)?;
    Ok(Json(
        SplitCheckResult {
            original_check_id: check_id,
            split_type,
            splits,
        },
    ))
}

// ── Check Hold/Recall ──
#[utoipa::path(
    post,
    path = "/v1/outlets/{outlet_id}/checks/{check_id}/hold",
    responses(
        (status = 200, description = "Check held", body = CheckStateChange),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Forbidden"),
    ),
    security(),
    tag = "pos",
)]

pub async fn hold_check(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path((outlet_id, check_id)): Path<(uuid::Uuid, uuid::Uuid)>,
) -> Result<Json<CheckStateChange>, (StatusCode, String)> {
    let a = auth::verify_token(&state.config, &headers).await?;
    crate::policy::require(
        &state.policy,
        &a,
        &headers,
        sentin8::shield::EvalTarget::new("hotel8:pos:order:write"),
    )
    .await?;
    let env_id = super::resolve_org_env(&a);
    let mut conn = state.pool.begin().await.map_err(super::err_db)?;
    super::set_tenant_context(&mut conn, env_id).await;
    sqlx::query("UPDATE pos.checks SET status = 'HELD', updated_at = now() WHERE id = $1 AND revenue_center_id = $2 AND status = 'OPEN'")
        .bind(check_id).bind(outlet_id)
        .execute(&mut *conn).await.map_err(super::err_db)?;
    emit_tx(
        &mut *conn,
        env_id,
        "hotel8:pos:check:held",
        serde_json::json!({ "check_id": check_id }),
        &crate::event_emit::Publisher::from_parts(&a.claims.sub, &a.claims.scope),
    )
    .await
    .map_err(super::err_emit)?;
    conn.commit().await.map_err(super::err_db)?;
    Ok(Json(CheckStateChange {
        check_id,
        status: "HELD".to_string(),
    }))
}
#[utoipa::path(
    post,
    path = "/v1/outlets/{outlet_id}/checks/{check_id}/recall",
    responses(
        (status = 200, description = "Check recalled", body = CheckStateChange),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Forbidden"),
    ),
    security(),
    tag = "pos",
)]

pub async fn recall_check(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path((outlet_id, check_id)): Path<(uuid::Uuid, uuid::Uuid)>,
) -> Result<Json<CheckStateChange>, (StatusCode, String)> {
    let a = auth::verify_token(&state.config, &headers).await?;
    crate::policy::require(
        &state.policy,
        &a,
        &headers,
        sentin8::shield::EvalTarget::new("hotel8:pos:order:write"),
    )
    .await?;
    let env_id = super::resolve_org_env(&a);
    let mut conn = state.pool.begin().await.map_err(super::err_db)?;
    super::set_tenant_context(&mut conn, env_id).await;
    sqlx::query("UPDATE pos.checks SET status = 'OPEN', updated_at = now() WHERE id = $1 AND revenue_center_id = $2 AND status = 'HELD'")
        .bind(check_id).bind(outlet_id)
        .execute(&mut *conn).await.map_err(super::err_db)?;
    emit_tx(
        &mut *conn,
        env_id,
        "hotel8:pos:check:recalled",
        serde_json::json!({ "check_id": check_id }),
        &crate::event_emit::Publisher::from_parts(&a.claims.sub, &a.claims.scope),
    )
    .await
    .map_err(super::err_emit)?;
    conn.commit().await.map_err(super::err_db)?;
    Ok(Json(CheckStateChange {
        check_id,
        status: "OPEN".to_string(),
    }))
}

// ── Seat Assignment - assign items to specific seats ──
#[utoipa::path(
    post,
    path = "/v1/outlets/{outlet_id}/checks/{check_id}/orders/{order_id}/seat",
    request_body = AssignSeatRequest,
    responses(
        (status = 200, description = "Seat assigned", body = SeatAssigned),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Forbidden"),
    ),
    security(),
    tag = "pos",
)]

pub async fn assign_seat(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path((_outlet_id, check_id, order_id)): Path<(uuid::Uuid, uuid::Uuid, uuid::Uuid)>,
    Json(req): Json<AssignSeatRequest>,
) -> Result<Json<SeatAssigned>, (StatusCode, String)> {
    let a = auth::verify_token(&state.config, &headers).await?;
    crate::policy::require(
        &state.policy,
        &a,
        &headers,
        sentin8::shield::EvalTarget::new("hotel8:pos:order:write"),
    )
    .await?;
    let env_id = super::resolve_org_env(&a);
    let mut conn = state.pool.begin().await.map_err(super::err_db)?;
    super::set_tenant_context(&mut conn, env_id).await;
    let seat_number = req.seat_number;
    // Store seat assignment in the order's modifiers JSONB as a special key
    sqlx::query("UPDATE pos.orders SET modifiers = modifiers || jsonb_build_object('seat_number', $3::int), updated_at = now() WHERE id = $1 AND check_id = $2")
        .bind(order_id).bind(check_id).bind(seat_number)
        .execute(&mut *conn).await.map_err(super::err_db)?;
    emit_tx(&mut *conn, env_id, "hotel8:pos:order:seat_assigned", serde_json::json!({ "check_id": check_id, "order_id": order_id, "seat_number": seat_number }),
        &crate::event_emit::Publisher::from_parts(&a.claims.sub, &a.claims.scope)).await.map_err(super::err_emit)?;
    conn.commit().await.map_err(super::err_db)?;
    Ok(Json(SeatAssigned {
        order_id,
        seat_number,
        assigned: true,
    }))
}

// ── Void Item ──
#[utoipa::path(
    post,
    path = "/v1/outlets/{outlet_id}/orders/void",
    request_body = VoidOrderItemRequest,
    responses(
        (status = 200, description = "Order item voided", body = OrderItemVoidResult),
        (status = 400, description = "Already voided"),
        (status = 403, description = "Manager approval required for voids over $50.00"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Forbidden"),
    ),
    security(),
    tag = "pos",
)]

pub async fn void_order_item(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path((_outlet_id, check_id)): Path<(uuid::Uuid, uuid::Uuid)>,
    Json(req): Json<VoidOrderItemRequest>,
) -> Result<Json<OrderItemVoidResult>, (StatusCode, String)> {
    let a = auth::verify_token(&state.config, &headers).await?;
    crate::policy::require(
        &state.policy,
        &a,
        &headers,
        sentin8::shield::EvalTarget::new("hotel8:pos:order:write"),
    )
    .await?;
    let env_id = super::resolve_org_env(&a);
    let mut conn = state.pool.begin().await.map_err(super::err_db)?;
    super::set_tenant_context(&mut conn, env_id).await;
    let order_id = req.order_id;
    let reason = req.reason.clone();
    let manager_approved = req.manager_approved;
    // Get the order to find line_total
    let order = sqlx::query(
        "SELECT id, line_total, status FROM pos.orders WHERE id = $1 AND check_id = $2",
    )
    .bind(order_id)
    .bind(check_id)
    .fetch_one(&mut *conn)
    .await
    .map_err(super::err_db)?;
    let status: String = order.get("status");
    if status == "CANCELLED" {
        return Err((StatusCode::BAD_REQUEST, "Order already voided".to_string()));
    }
    let lt_str: String = order.get("line_total");
    let line_total: rust_decimal::Decimal = lt_str.parse().unwrap_or_default();
    // Manager approval required for voids over $50
    if line_total > rust_decimal::Decimal::new(5000, 2) && !manager_approved {
        return Err((
            StatusCode::FORBIDDEN,
            "Manager approval required for voids over $50.00".to_string(),
        ));
    }
    // Void the order item
    sqlx::query("UPDATE pos.orders SET status = 'CANCELLED', updated_at = now() WHERE id = $1")
        .bind(order_id)
        .execute(&mut *conn)
        .await
        .map_err(super::err_db)?;
    // Update check totals (subtract the voided amount)
    sqlx::query("UPDATE pos.checks SET subtotal = GREATEST(subtotal - $2, 0), total = GREATEST(total - $2, 0), updated_at = now() WHERE id = $1")
        .bind(check_id).bind(&lt_str)
        .execute(&mut *conn).await.map_err(super::err_db)?;
    emit_tx(&mut *conn, env_id, "hotel8:pos:order:voided", serde_json::json!({ "check_id": check_id, "order_id": order_id, "reason": reason, "amount": &lt_str, "manager_approved": manager_approved }),
        &crate::event_emit::Publisher::from_parts(&a.claims.sub, &a.claims.scope)).await.map_err(super::err_emit)?;
    conn.commit().await.map_err(super::err_db)?;
    Ok(Json(OrderItemVoidResult {
        order_id,
        voided: true,
        reason: reason.to_string(),
        amount_refunded: lt_str,
    }))
}

// ── Void Check ──
#[utoipa::path(
    post,
    path = "/v1/outlets/{outlet_id}/checks/{check_id}/void",
    request_body = VoidCheckRequest,
    responses(
        (status = 200, description = "Check voided", body = CheckVoidResult),
        (status = 400, description = "Already voided / paid - refund instead"),
        (status = 403, description = "Manager approval required for voids over $100.00"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Forbidden"),
    ),
    security(),
    tag = "pos",
)]

pub async fn void_check(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path((_outlet_id, check_id)): Path<(uuid::Uuid, uuid::Uuid)>,
    Json(req): Json<VoidCheckRequest>,
) -> Result<Json<CheckVoidResult>, (StatusCode, String)> {
    let a = auth::verify_token(&state.config, &headers).await?;
    crate::policy::require(
        &state.policy,
        &a,
        &headers,
        sentin8::shield::EvalTarget::new("hotel8:pos:order:write"),
    )
    .await?;
    let env_id = super::resolve_org_env(&a);
    let mut conn = state.pool.begin().await.map_err(super::err_db)?;
    super::set_tenant_context(&mut conn, env_id).await;
    let reason = req.reason.clone();
    let manager_approved = req.manager_approved;
    // Get the check
    let check = sqlx::query("SELECT id, status, total::text as total FROM pos.checks WHERE id = $1 AND revenue_center_id = $2")
        .bind(check_id).bind(_outlet_id)
        .fetch_one(&mut *conn).await.map_err(super::err_db)?;
    let status: String = check.get("status");
    let total_str: String = check.get("total");
    let total: rust_decimal::Decimal = total_str.parse().unwrap_or_default();
    // Void-authorization policy (see check_state::can_void): state guards +
    // manager approval for voids over $100.00.
    if let Err(e) = CheckState::can_void(CheckState::from_label(&status), total, manager_approved) {
        return Err(match e {
            VoidError::AlreadyVoided => {
                (StatusCode::BAD_REQUEST, "Check already voided".to_string())
            }
            VoidError::PaidNeedsRefund => (
                StatusCode::BAD_REQUEST,
                "Cannot void a paid check - use refund instead".to_string(),
            ),
            VoidError::ManagerApprovalRequired => (
                StatusCode::FORBIDDEN,
                "Manager approval required for voids over $100.00".to_string(),
            ),
        });
    }
    // Void all orders on this check
    sqlx::query(
        "UPDATE pos.orders SET status = 'CANCELLED', updated_at = now() WHERE check_id = $1",
    )
    .bind(check_id)
    .execute(&mut *conn)
    .await
    .map_err(super::err_db)?;
    // Void the check
    sqlx::query("UPDATE pos.checks SET status = 'VOIDED', subtotal = 0, tax_total = 0, total = 0, closed_at = now(), updated_at = now() WHERE id = $1")
        .bind(check_id).execute(&mut *conn).await.map_err(super::err_db)?;
    emit_tx(&mut *conn, env_id, "hotel8:pos:check:voided", serde_json::json!({ "check_id": check_id, "reason": reason, "manager_approved": manager_approved }),
        &crate::event_emit::Publisher::from_parts(&a.claims.sub, &a.claims.scope)).await.map_err(super::err_emit)?;
    conn.commit().await.map_err(super::err_db)?;
    Ok(Json(CheckVoidResult {
        check_id,
        voided: true,
        reason: reason.to_string(),
    }))
}
