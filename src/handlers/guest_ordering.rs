// SPDX-FileCopyrightText: Copyright Logically Magic Inc. <hello@logicallymagic.com>
// SPDX-FileCopyrightText: Copyright Hamid Jahanzad <Hamid@Jahanzad.com>
// SPDX-FileCopyrightText: Copyright Contributors to the Hotel8:POS
// SPDX-License-Identifier: Apache-2.0

//! Guest-ordering (room service) handlers — the eGuest app surface.
//!
//! Contract: `services/pos/DESIGN.md` §4 "Guest ordering" — guest-scoped
//! routes that reuse the staff check/order machines with the reservation bound
//! at creation. The room-service outlet is the property's `ROOM_SERVICE`
//! revenue center; menu reads come from its catalog.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use sqlx::Row;

use crate::auth;
use crate::event_emit::{emit_tx, Publisher};
use crate::handlers::AppState;

/// `GET /v1/guest/{property_id}/menu?outlet_id=` — menu catalog for the
/// room-service outlet (defaults to the property's first `ROOM_SERVICE`
/// outlet when `outlet_id` is omitted).
pub async fn guest_menu(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path(property_id): Path<uuid::Uuid>,
    axum::extract::Query(params): axum::extract::Query<serde_json::Value>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let a = auth::verify_token(&state.config, &headers).await?;
    crate::policy::require(
        &state.policy,
        &a,
        &headers,
        sentin8::shield::EvalTarget::new("hotel8:pos:guest:read"),
    )
    .await?;
    let mut conn = state.pool.begin().await.map_err(super::err_db)?;
    super::set_tenant_context(&mut conn, super::resolve_org_env(&a)).await;

    let outlet_id: uuid::Uuid = match params.get("outlet_id").and_then(|v| v.as_str()).and_then(|s| uuid::Uuid::parse_str(s).ok()) {
        Some(id) => id,
        None => sqlx::query(
            "SELECT id FROM pos.revenue_centers WHERE property_id = $1 AND outlet_type = 'ROOM_SERVICE' AND is_active = true ORDER BY outlet_code LIMIT 1",
        )
        .bind(property_id)
        .fetch_optional(&mut *conn)
        .await
        .map_err(super::err_db)?
        .ok_or_else(|| super::err_not_found("No room-service outlet for this property"))?
        .get("id"),
    };

    let items = sqlx::query(
        "SELECT mi.id::text, mi.item_name, mi.description, mi.price::text AS price,
                COALESCE(mc.category_name, '') AS category, mi.is_available
         FROM pos.menu_items mi LEFT JOIN pos.menu_categories mc ON mc.id = mi.category_id
         WHERE mi.revenue_center_id = $1 AND mi.is_available = true
         ORDER BY COALESCE(mc.category_name, ''), mi.item_name",
    )
    .bind(outlet_id)
    .fetch_all(&mut *conn)
    .await
    .map_err(super::err_db)?;
    let menu: Vec<serde_json::Value> = items
        .iter()
        .map(|r| {
            serde_json::json!({
                "id": r.get::<String, _>("id"),
                "name": r.get::<String, _>("item_name"),
                "description": r.get::<Option<String>, _>("description").unwrap_or_default(),
                "price": r.get::<String, _>("price"),
                "category": r.get::<String, _>("category"),
            })
        })
        .collect();
    conn.commit().await.map_err(super::err_db)?;
    Ok(Json(
        serde_json::json!({ "outlet_id": outlet_id, "items": menu }),
    ))
}

/// `POST /v1/guest/{property_id}/orders` — room-service order. Creates an
/// `OPEN` check bound to the in-house reservation + room, inserts the ordered
/// lines (same machine the terminals use), returns the check + totals.
pub async fn guest_create_order(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path(property_id): Path<uuid::Uuid>,
    Json(req): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let a = auth::verify_token(&state.config, &headers).await?;
    crate::policy::require(
        &state.policy,
        &a,
        &headers,
        sentin8::shield::EvalTarget::new("hotel8:pos:guest:write"),
    )
    .await?;
    let env_id = super::resolve_org_env(&a);
    let mut conn = state.pool.begin().await.map_err(super::err_db)?;
    super::set_tenant_context(&mut conn, env_id).await;

    let items = req
        .get("items")
        .and_then(|v| v.as_array())
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "items array required".to_string()))?;
    if items.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "at least one item required".to_string(),
        ));
    }

    let outlet_id: uuid::Uuid = req
        .get("outlet_id")
        .and_then(|v| v.as_str())
        .and_then(|s| uuid::Uuid::parse_str(s).ok())
        .ok_or_else(|| (StatusCode::BAD_REQUEST, "outlet_id required".to_string()))?;
    let reservation_id: uuid::Uuid = req
        .get("reservation_id")
        .and_then(|v| v.as_str())
        .and_then(|s| uuid::Uuid::parse_str(s).ok())
        .ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                "reservation_id required".to_string(),
            )
        })?;
    // The check anchors to the in-house stay in this property (guest-ordering
    // invariant): verify the reservation is CHECKED_IN here before binding —
    // never accept a caller-supplied room that is not in-house.
    let stay = sqlx::query(
        "SELECT r.assigned_room_number, g.first_name || ' ' || g.last_name AS guest_name
         FROM pms_reservations r LEFT JOIN pms_guest_profiles g ON g.id = r.guest_profile_id
         WHERE r.id = $1 AND r.property_id = $2 AND r.status = 'CHECKED_IN'",
    )
    .bind(reservation_id)
    .bind(property_id)
    .fetch_optional(&mut *conn)
    .await
    .map_err(super::err_db)?
    .ok_or_else(|| super::err_not_found("Reservation is not an in-house stay at this property"))?;
    let room_number = stay
        .get::<Option<String>, _>("assigned_room_number")
        .or_else(|| {
            req.get("room_number")
                .and_then(|v| v.as_str())
                .map(str::to_owned)
        })
        .unwrap_or_default();
    let guest_name: String = stay.get("guest_name");
    let order_no = format!("RS-{}", chrono::Utc::now().format("%Y%m%d%H%M%S%3f"));
    let row = sqlx::query(
        "INSERT INTO pos.checks (revenue_center_id, check_number, guest_name, guest_count, room_number, reservation_id)
         VALUES ($1, $2, $3, 1, $4, $5) RETURNING id, check_number, status",
    )
    .bind(outlet_id)
    .bind(&order_no)
    .bind(&guest_name)
    .bind(&room_number)
    .bind(reservation_id)
    .fetch_one(&mut *conn)
    .await
    .map_err(super::err_db)?;
    let check_id: uuid::Uuid = row.get("id");
    let check_number: String = row.get("check_number");
    let check_status: String = row.get("status");

    // Ordered lines — same INSERT the staff `add_order_item` handler runs.
    let mut lines: Vec<serde_json::Value> = Vec::new();
    for item in items {
        let menu_item_id: uuid::Uuid = item
            .get("menu_item_id")
            .and_then(|v| v.as_str())
            .and_then(|s| uuid::Uuid::parse_str(s).ok())
            .ok_or_else(|| {
                (
                    StatusCode::BAD_REQUEST,
                    "each item needs menu_item_id".to_string(),
                )
            })?;
        let quantity = item.get("quantity").and_then(|v| v.as_u64()).unwrap_or(1) as i32;
        if quantity < 1 {
            return Err((StatusCode::BAD_REQUEST, "quantity must be >= 1".to_string()));
        }
        let mi = sqlx::query("SELECT item_name, price::text AS price FROM pos.menu_items WHERE id = $1 AND revenue_center_id = $2")
            .bind(menu_item_id)
            .bind(outlet_id)
            .fetch_optional(&mut *conn)
            .await
            .map_err(super::err_db)?
            .ok_or_else(|| (StatusCode::BAD_REQUEST, format!("menu item {menu_item_id} not on this outlet's menu")))?;
        let price_str: String = mi.get("price");
        let unit_price: rust_decimal::Decimal = price_str.parse().unwrap_or_default();
        let line_total = (unit_price * rust_decimal::Decimal::from(quantity)).to_string();
        let line = sqlx::query(
            "INSERT INTO pos.orders (check_id, menu_item_id, quantity, unit_price, line_total, status, sent_at)
             VALUES ($1, $2, $3, $4, $5, 'SENT', now()) RETURNING id",
        )
        .bind(check_id)
        .bind(menu_item_id)
        .bind(quantity)
        .bind(&price_str)
        .bind(&line_total)
        .fetch_one(&mut *conn)
        .await
        .map_err(super::err_db)?;
        lines.push(serde_json::json!({
            "id": line.get::<uuid::Uuid, _>("id"),
            "menu_item_id": menu_item_id,
            "name": mi.get::<String, _>("item_name"),
            "quantity": quantity,
            "unit_price": price_str,
        }));
    }
    // Recompute the check totals from its lines.
    sqlx::query(
        "UPDATE pos.checks SET subtotal = (SELECT COALESCE(SUM(line_total), 0) FROM pos.orders WHERE check_id = $1),
            total = (SELECT COALESCE(SUM(line_total), 0) FROM pos.orders WHERE check_id = $1), updated_at = now()
         WHERE id = $1",
    )
    .bind(check_id)
    .execute(&mut *conn)
    .await
    .map_err(super::err_db)?;

    emit_tx(
        &mut *conn,
        env_id,
        "hotel8:pos:order:created",
        serde_json::json!({ "check_id": check_id, "check_number": &check_number, "reservation_id": reservation_id, "room_number": room_number, "channel": "GUEST" }),
        &Publisher::from_parts(&a.claims.sub, &a.claims.scope),
    )
    .await
    .map_err(super::err_emit)?;

    conn.commit().await.map_err(super::err_db)?;

    Ok(Json(serde_json::json!({
        "check_id": check_id,
        "check_number": check_number,
        "status": check_status,
        "items": lines,
    })))
}

/// `GET /v1/guest/{property_id}/orders/{check_id}` — order status read-back
/// for the ordering screen (prep states drive the UI stepper).
pub async fn guest_get_order(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path((property_id, check_id)): Path<(uuid::Uuid, uuid::Uuid)>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let a = auth::verify_token(&state.config, &headers).await?;
    crate::policy::require(
        &state.policy,
        &a,
        &headers,
        sentin8::shield::EvalTarget::new("hotel8:pos:guest:read"),
    )
    .await?;
    let mut conn = state.pool.begin().await.map_err(super::err_db)?;
    super::set_tenant_context(&mut conn, super::resolve_org_env(&a)).await;
    let check = sqlx::query(
        "SELECT id::text, check_number, status::text AS status, subtotal::text AS subtotal,
                tax_total::text AS tax_total, total::text AS total, room_number, reservation_id::text
         FROM pos.checks WHERE id = $1 AND revenue_center_id IN (SELECT id FROM pos.revenue_centers WHERE property_id = $2)",
    )
    .bind(check_id)
    .bind(property_id)
    .fetch_optional(&mut *conn)
    .await
    .map_err(super::err_db)?
    .ok_or_else(|| super::err_not_found("Order not found"))?;
    let lines = sqlx::query(
        "SELECT o.id::text, mi.item_name, o.quantity, o.unit_price::text AS unit_price, o.line_total::text AS line_total, o.status::text AS status
         FROM pos.orders o JOIN pos.menu_items mi ON mi.id = o.menu_item_id
         WHERE o.check_id = $1 ORDER BY o.created_at",
    )
    .bind(check_id)
    .fetch_all(&mut *conn)
    .await
    .map_err(super::err_db)?;
    conn.commit().await.map_err(super::err_db)?;
    Ok(Json(serde_json::json!({
        "check_id": check.get::<String, _>("id"),
        "check_number": check.get::<String, _>("check_number"),
        "status": check.get::<String, _>("status"),
        "subtotal": check.get::<String, _>("subtotal"),
        "tax_total": check.get::<String, _>("tax_total"),
        "total": check.get::<String, _>("total"),
        "room_number": check.get::<Option<String>, _>("room_number"),
        "reservation_id": check.get::<Option<String>, _>("reservation_id"),
        "items": lines
            .iter()
            .map(|l| {
                serde_json::json!({
                    "id": l.get::<String, _>("id"),
                    "name": l.get::<String, _>("item_name"),
                    "quantity": l.get::<i32, _>("quantity"),
                    "unit_price": l.get::<String, _>("unit_price"),
                    "line_total": l.get::<String, _>("line_total"),
                    "status": l.get::<String, _>("status"),
                })
            })
            .collect::<Vec<_>>(),
    })))
}
