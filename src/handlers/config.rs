// SPDX-FileCopyrightText: Copyright Logically Magic Inc. <hello@logicallymagic.com>
// SPDX-FileCopyrightText: Copyright Hamid Jahanzad <Hamid@Jahanzad.com>
// SPDX-FileCopyrightText: Copyright Contributors to the Hotel8:POS
// SPDX-License-Identifier: Apache-2.0

//! H8POS configuration handlers — service areas, menu categories, modifier
//! groups, item availability, bulk price updates, tax groups, printer groups,
//! and payment method configuration.
//!
//! Implements , , , , , , ,
//! , .
use crate::auth;
use crate::event_emit::{emit_on_pool, emit_tx};
use crate::handlers::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use sqlx::Row;

fn err_bad_request(msg: &str) -> (StatusCode, String) {
    (StatusCode::BAD_REQUEST, msg.to_string())
}

// ═══════════════════════════════════════════════════════════════════════════
// Service Area CRUD
// ═══════════════════════════════════════════════════════════════════════════

pub async fn create_service_area(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path(outlet_id): Path<uuid::Uuid>,
    Json(req): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let a = auth::verify_token(&state.config, &headers).await?;
    crate::policy::require(
        &state.policy,
        &a,
        &headers,
        sentin8::shield::EvalTarget::new("hotel8:pos:admin"),
    )
    .await?;
    let env_id = super::resolve_org_env(&a);
    let area_code = req.get("area_code").and_then(|v| v.as_str()).unwrap_or("");
    let area_name = req.get("area_name").and_then(|v| v.as_str()).unwrap_or("");
    if area_code.is_empty() || area_name.is_empty() {
        return Err(err_bad_request("area_code and area_name are required"));
    }
    let mut conn = state.pool.begin().await.map_err(super::err_db)?;
    super::set_tenant_context(&mut conn, env_id).await;
    let row = sqlx::query(
        "INSERT INTO pos.service_areas (revenue_center_id, area_code, area_name) \
         VALUES ($1, $2, $3) \
         RETURNING id, area_code, area_name, is_active",
    )
    .bind(outlet_id)
    .bind(area_code)
    .bind(area_name)
    .fetch_one(&mut *conn)
    .await
    .map_err(super::err_db)?;
    let id: uuid::Uuid = row.get("id");
    emit_tx(
        &mut *conn,
        env_id,
        "hotel8:pos:service_area:created",
        serde_json::json!({ "outlet_id": outlet_id, "area_id": id, "area_code": area_code }),
        &crate::event_emit::Publisher::from_parts(&a.claims.sub, &a.claims.scope),
    )
    .await
    .map_err(super::err_emit)?;
    conn.commit().await.map_err(super::err_db)?;
    Ok(Json(serde_json::json!({
        "id": id,
        "outlet_id": outlet_id,
        "area_code": row.get::<String, _>("area_code"),
        "area_name": row.get::<String, _>("area_name"),
        "is_active": row.get::<bool, _>("is_active"),
    })))
}

pub async fn list_service_areas(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path(outlet_id): Path<uuid::Uuid>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
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
    let rows = sqlx::query(
        "SELECT id, area_code, area_name, is_active FROM pos.service_areas \
         WHERE revenue_center_id = $1 ORDER BY area_code",
    )
    .bind(outlet_id)
    .fetch_all(&mut *conn)
    .await
    .map_err(super::err_db)?;
    let items: Vec<serde_json::Value> = rows
        .iter()
        .map(|r| {
            serde_json::json!({
                "id": r.get::<uuid::Uuid, _>("id"),
                "outlet_id": outlet_id,
                "area_code": r.get::<String, _>("area_code"),
                "area_name": r.get::<String, _>("area_name"),
                "is_active": r.get::<bool, _>("is_active"),
            })
        })
        .collect();
    conn.commit().await.map_err(super::err_db)?;
    Ok(Json(serde_json::json!({ "items": items })))
}

pub async fn update_service_area(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path((outlet_id, area_id)): Path<(uuid::Uuid, uuid::Uuid)>,
    Json(req): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let a = auth::verify_token(&state.config, &headers).await?;
    crate::policy::require(
        &state.policy,
        &a,
        &headers,
        sentin8::shield::EvalTarget::new("hotel8:pos:admin"),
    )
    .await?;
    let env_id = super::resolve_org_env(&a);
    let area_name = req.get("area_name").and_then(|v| v.as_str());
    let is_active = req.get("is_active").and_then(|v| v.as_bool());
    let mut conn = state.pool.begin().await.map_err(super::err_db)?;
    super::set_tenant_context(&mut conn, env_id).await;
    let row = sqlx::query(
        "UPDATE pos.service_areas SET \
            area_name = COALESCE($1, area_name), \
            is_active = COALESCE($2, is_active), \
            updated_at = now() \
         WHERE id = $3 AND revenue_center_id = $4 \
         RETURNING id, area_code, area_name, is_active",
    )
    .bind(area_name)
    .bind(is_active)
    .bind(area_id)
    .bind(outlet_id)
    .fetch_optional(&mut *conn)
    .await
    .map_err(super::err_db)?
    .ok_or_else(|| super::err_not_found("service area not found"))?;
    emit_tx(
        &mut *conn,
        env_id,
        "hotel8:pos:service_area:updated",
        serde_json::json!({ "outlet_id": outlet_id, "area_id": area_id }),
        &crate::event_emit::Publisher::from_parts(&a.claims.sub, &a.claims.scope),
    )
    .await
    .map_err(super::err_emit)?;
    conn.commit().await.map_err(super::err_db)?;
    Ok(Json(serde_json::json!({
        "id": row.get::<uuid::Uuid, _>("id"),
        "outlet_id": outlet_id,
        "area_code": row.get::<String, _>("area_code"),
        "area_name": row.get::<String, _>("area_name"),
        "is_active": row.get::<bool, _>("is_active"),
    })))
}

pub async fn delete_service_area(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path((outlet_id, area_id)): Path<(uuid::Uuid, uuid::Uuid)>,
) -> Result<StatusCode, (StatusCode, String)> {
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
    // Block deletion if tables are linked to this service area.
    let linked: i64 = sqlx::query("SELECT COUNT(*) FROM pos.tables WHERE service_area_id = $1")
        .bind(area_id)
        .fetch_one(&mut *conn)
        .await
        .map_err(super::err_db)?
        .get::<i64, _>(0);
    if linked > 0 {
        return Err(err_bad_request(
            "cannot delete service area with linked tables",
        ));
    }
    let res = sqlx::query("DELETE FROM pos.service_areas WHERE id = $1 AND revenue_center_id = $2")
        .bind(area_id)
        .bind(outlet_id)
        .execute(&mut *conn)
        .await
        .map_err(super::err_db)?;
    if res.rows_affected() == 0 {
        return Err(super::err_not_found("service area not found"));
    }
    emit_tx(
        &mut *conn,
        env_id,
        "hotel8:pos:service_area:deleted",
        serde_json::json!({ "outlet_id": outlet_id, "area_id": area_id }),
        &crate::event_emit::Publisher::from_parts(&a.claims.sub, &a.claims.scope),
    )
    .await
    .map_err(super::err_emit)?;
    conn.commit().await.map_err(super::err_db)?;
    Ok(StatusCode::NO_CONTENT)
}

// ═══════════════════════════════════════════════════════════════════════════
// Menu Category CRUD
// ═══════════════════════════════════════════════════════════════════════════

pub async fn create_menu_category(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path(outlet_id): Path<uuid::Uuid>,
    Json(req): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let a = auth::verify_token(&state.config, &headers).await?;
    crate::policy::require(
        &state.policy,
        &a,
        &headers,
        sentin8::shield::EvalTarget::new("hotel8:pos:admin"),
    )
    .await?;
    let env_id = super::resolve_org_env(&a);
    let category_code = req
        .get("category_code")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let category_name = req
        .get("category_name")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let sort_order: i32 = req
        .get("sort_order")
        .and_then(|v| v.as_i64())
        .map(|n| n as i32)
        .unwrap_or(0);
    if category_code.is_empty() || category_name.is_empty() {
        return Err(err_bad_request(
            "category_code and category_name are required",
        ));
    }
    let mut conn = state.pool.begin().await.map_err(super::err_db)?;
    super::set_tenant_context(&mut conn, env_id).await;
    let row = sqlx::query(
        "INSERT INTO pos.menu_categories \
         (revenue_center_id, category_code, category_name, sort_order) \
         VALUES ($1, $2, $3, $4) \
         RETURNING id, category_code, category_name, sort_order, is_active",
    )
    .bind(outlet_id)
    .bind(category_code)
    .bind(category_name)
    .bind(sort_order)
    .fetch_one(&mut *conn)
    .await
    .map_err(super::err_db)?;
    let id: uuid::Uuid = row.get("id");
    emit_tx(&mut *conn, env_id, "hotel8:pos:menu:category:created", serde_json::json!({ "outlet_id": outlet_id, "category_id": id, "category_code": category_code }),
        &crate::event_emit::Publisher::from_parts(&a.claims.sub, &a.claims.scope)).await.map_err(super::err_emit)?;
    conn.commit().await.map_err(super::err_db)?;
    Ok(Json(serde_json::json!({
        "id": id,
        "outlet_id": outlet_id,
        "category_code": row.get::<String, _>("category_code"),
        "category_name": row.get::<String, _>("category_name"),
        "sort_order": row.get::<i32, _>("sort_order"),
        "is_active": row.get::<bool, _>("is_active"),
    })))
}

pub async fn list_menu_categories(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path(outlet_id): Path<uuid::Uuid>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
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
    let rows = sqlx::query(
        "SELECT id, category_code, category_name, sort_order, is_active \
         FROM pos.menu_categories \
         WHERE revenue_center_id = $1 \
         ORDER BY sort_order, category_code",
    )
    .bind(outlet_id)
    .fetch_all(&mut *conn)
    .await
    .map_err(super::err_db)?;
    let items: Vec<serde_json::Value> = rows
        .iter()
        .map(|r| {
            serde_json::json!({
                "id": r.get::<uuid::Uuid, _>("id"),
                "outlet_id": outlet_id,
                "category_code": r.get::<String, _>("category_code"),
                "category_name": r.get::<String, _>("category_name"),
                "sort_order": r.get::<i32, _>("sort_order"),
                "is_active": r.get::<bool, _>("is_active"),
            })
        })
        .collect();
    conn.commit().await.map_err(super::err_db)?;
    Ok(Json(serde_json::json!({ "items": items })))
}

pub async fn update_menu_category(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path((outlet_id, category_id)): Path<(uuid::Uuid, uuid::Uuid)>,
    Json(req): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let a = auth::verify_token(&state.config, &headers).await?;
    crate::policy::require(
        &state.policy,
        &a,
        &headers,
        sentin8::shield::EvalTarget::new("hotel8:pos:admin"),
    )
    .await?;
    let env_id = super::resolve_org_env(&a);
    let category_name = req.get("category_name").and_then(|v| v.as_str());
    let sort_order: Option<i32> = req
        .get("sort_order")
        .and_then(|v| v.as_i64())
        .map(|n| n as i32);
    let is_active = req.get("is_active").and_then(|v| v.as_bool());
    let mut conn = state.pool.begin().await.map_err(super::err_db)?;
    super::set_tenant_context(&mut conn, env_id).await;
    let row = sqlx::query(
        "UPDATE pos.menu_categories SET \
            category_name = COALESCE($1, category_name), \
            sort_order = COALESCE($2, sort_order), \
            is_active = COALESCE($3, is_active), \
            updated_at = now() \
         WHERE id = $4 AND revenue_center_id = $5 \
         RETURNING id, category_code, category_name, sort_order, is_active",
    )
    .bind(category_name)
    .bind(sort_order)
    .bind(is_active)
    .bind(category_id)
    .bind(outlet_id)
    .fetch_optional(&mut *conn)
    .await
    .map_err(super::err_db)?
    .ok_or_else(|| super::err_not_found("menu category not found"))?;
    emit_tx(
        &mut *conn,
        env_id,
        "hotel8:pos:menu:category:updated",
        serde_json::json!({ "outlet_id": outlet_id, "category_id": category_id }),
        &crate::event_emit::Publisher::from_parts(&a.claims.sub, &a.claims.scope),
    )
    .await
    .map_err(super::err_emit)?;
    conn.commit().await.map_err(super::err_db)?;
    Ok(Json(serde_json::json!({
        "id": row.get::<uuid::Uuid, _>("id"),
        "outlet_id": outlet_id,
        "category_code": row.get::<String, _>("category_code"),
        "category_name": row.get::<String, _>("category_name"),
        "sort_order": row.get::<i32, _>("sort_order"),
        "is_active": row.get::<bool, _>("is_active"),
    })))
}

pub async fn delete_menu_category(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path((outlet_id, category_id)): Path<(uuid::Uuid, uuid::Uuid)>,
) -> Result<StatusCode, (StatusCode, String)> {
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
    // Block deletion if menu items are linked to this category.
    let linked: i64 = sqlx::query("SELECT COUNT(*) FROM pos.menu_items WHERE category_id = $1")
        .bind(category_id)
        .fetch_one(&mut *conn)
        .await
        .map_err(super::err_db)?
        .get::<i64, _>(0);
    if linked > 0 {
        return Err(err_bad_request(
            "cannot delete menu category with linked menu items",
        ));
    }
    let res =
        sqlx::query("DELETE FROM pos.menu_categories WHERE id = $1 AND revenue_center_id = $2")
            .bind(category_id)
            .bind(outlet_id)
            .execute(&mut *conn)
            .await
            .map_err(super::err_db)?;
    if res.rows_affected() == 0 {
        return Err(super::err_not_found("menu category not found"));
    }
    emit_tx(
        &mut *conn,
        env_id,
        "hotel8:pos:menu:category:deleted",
        serde_json::json!({ "outlet_id": outlet_id, "category_id": category_id }),
        &crate::event_emit::Publisher::from_parts(&a.claims.sub, &a.claims.scope),
    )
    .await
    .map_err(super::err_emit)?;
    conn.commit().await.map_err(super::err_db)?;
    Ok(StatusCode::NO_CONTENT)
}

// ═══════════════════════════════════════════════════════════════════════════
// Modifier Group CRUD
// ═══════════════════════════════════════════════════════════════════════════

pub async fn create_modifier_group(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path((outlet_id, item_id)): Path<(uuid::Uuid, uuid::Uuid)>,
    Json(req): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let a = auth::verify_token(&state.config, &headers).await?;
    crate::policy::require(
        &state.policy,
        &a,
        &headers,
        sentin8::shield::EvalTarget::new("hotel8:pos:admin"),
    )
    .await?;
    let env_id = super::resolve_org_env(&a);
    let group_name = req.get("group_name").and_then(|v| v.as_str()).unwrap_or("");
    let is_required = req
        .get("is_required")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let min_selections: i32 = req
        .get("min_selections")
        .and_then(|v| v.as_i64())
        .map(|n| n as i32)
        .unwrap_or(0);
    let max_selections: i32 = req
        .get("max_selections")
        .and_then(|v| v.as_i64())
        .map(|n| n as i32)
        .unwrap_or(1);
    if group_name.is_empty() {
        return Err(err_bad_request("group_name is required"));
    }
    let mut conn = state.pool.begin().await.map_err(super::err_db)?;
    super::set_tenant_context(&mut conn, env_id).await;
    let row = sqlx::query(
        "INSERT INTO pos.modifier_groups \
         (menu_item_id, group_name, is_required, min_selections, max_selections) \
         VALUES ($1, $2, $3, $4, $5) \
         RETURNING id, group_name, is_required, min_selections, max_selections",
    )
    .bind(item_id)
    .bind(group_name)
    .bind(is_required)
    .bind(min_selections)
    .bind(max_selections)
    .fetch_one(&mut *conn)
    .await
    .map_err(super::err_db)?;
    let id: uuid::Uuid = row.get("id");
    emit_tx(
        &mut *conn,
        env_id,
        "hotel8:pos:modifier_group:created",
        serde_json::json!({ "outlet_id": outlet_id, "item_id": item_id, "group_id": id }),
        &crate::event_emit::Publisher::from_parts(&a.claims.sub, &a.claims.scope),
    )
    .await
    .map_err(super::err_emit)?;
    conn.commit().await.map_err(super::err_db)?;
    Ok(Json(serde_json::json!({
        "id": id,
        "outlet_id": outlet_id,
        "item_id": item_id,
        "group_name": row.get::<String, _>("group_name"),
        "is_required": row.get::<bool, _>("is_required"),
        "min_selections": row.get::<i32, _>("min_selections"),
        "max_selections": row.get::<i32, _>("max_selections"),
    })))
}

pub async fn list_modifier_groups(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path((outlet_id, item_id)): Path<(uuid::Uuid, uuid::Uuid)>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
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
    let rows = sqlx::query(
        "SELECT id, group_name, is_required, min_selections, max_selections \
         FROM pos.modifier_groups \
         WHERE menu_item_id = $1 ORDER BY group_name",
    )
    .bind(item_id)
    .fetch_all(&mut *conn)
    .await
    .map_err(super::err_db)?;
    let items: Vec<serde_json::Value> = rows
        .iter()
        .map(|r| {
            serde_json::json!({
                "id": r.get::<uuid::Uuid, _>("id"),
                "outlet_id": outlet_id,
                "item_id": item_id,
                "group_name": r.get::<String, _>("group_name"),
                "is_required": r.get::<bool, _>("is_required"),
                "min_selections": r.get::<i32, _>("min_selections"),
                "max_selections": r.get::<i32, _>("max_selections"),
            })
        })
        .collect();
    conn.commit().await.map_err(super::err_db)?;
    Ok(Json(serde_json::json!({ "items": items })))
}

pub async fn delete_modifier_group(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path((outlet_id, item_id, group_id)): Path<(uuid::Uuid, uuid::Uuid, uuid::Uuid)>,
) -> Result<StatusCode, (StatusCode, String)> {
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
    let res = sqlx::query("DELETE FROM pos.modifier_groups WHERE id = $1 AND menu_item_id = $2")
        .bind(group_id)
        .bind(item_id)
        .execute(&mut *conn)
        .await
        .map_err(super::err_db)?;
    if res.rows_affected() == 0 {
        return Err(super::err_not_found("modifier group not found"));
    }
    emit_tx(
        &mut *conn,
        env_id,
        "hotel8:pos:modifier_group:deleted",
        serde_json::json!({ "outlet_id": outlet_id, "item_id": item_id, "group_id": group_id }),
        &crate::event_emit::Publisher::from_parts(&a.claims.sub, &a.claims.scope),
    )
    .await
    .map_err(super::err_emit)?;
    conn.commit().await.map_err(super::err_db)?;
    Ok(StatusCode::NO_CONTENT)
}

// ═══════════════════════════════════════════════════════════════════════════
// Modifier CRUD
// ═══════════════════════════════════════════════════════════════════════════

pub async fn create_modifier(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path((outlet_id, item_id, group_id)): Path<(uuid::Uuid, uuid::Uuid, uuid::Uuid)>,
    Json(req): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let a = auth::verify_token(&state.config, &headers).await?;
    crate::policy::require(
        &state.policy,
        &a,
        &headers,
        sentin8::shield::EvalTarget::new("hotel8:pos:admin"),
    )
    .await?;
    let env_id = super::resolve_org_env(&a);
    let modifier_name = req
        .get("modifier_name")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let price_adjustment: String = req
        .get("price_adjustment")
        .and_then(|v| v.as_str())
        .unwrap_or("0")
        .to_string();
    if modifier_name.is_empty() {
        return Err(err_bad_request("modifier_name is required"));
    }
    let mut conn = state.pool.begin().await.map_err(super::err_db)?;
    super::set_tenant_context(&mut conn, env_id).await;
    let row = sqlx::query(
        "INSERT INTO pos.modifiers \
         (modifier_group_id, modifier_name, price_adjustment) \
         VALUES ($1, $2, $3) \
         RETURNING id, modifier_name, price_adjustment::text as price_adjustment, is_active",
    )
    .bind(group_id)
    .bind(modifier_name)
    .bind(&price_adjustment)
    .fetch_one(&mut *conn)
    .await
    .map_err(super::err_db)?;
    let id: uuid::Uuid = row.get("id");
    emit_tx(&mut *conn, env_id, "hotel8:pos:modifier:created", serde_json::json!({ "outlet_id": outlet_id, "item_id": item_id, "group_id": group_id, "modifier_id": id }),
        &crate::event_emit::Publisher::from_parts(&a.claims.sub, &a.claims.scope)).await.map_err(super::err_emit)?;
    conn.commit().await.map_err(super::err_db)?;
    Ok(Json(serde_json::json!({
        "id": id,
        "outlet_id": outlet_id,
        "item_id": item_id,
        "group_id": group_id,
        "modifier_name": row.get::<String, _>("modifier_name"),
        "price_adjustment": row.get::<String, _>("price_adjustment"),
        "is_active": row.get::<bool, _>("is_active"),
    })))
}

pub async fn list_modifiers(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path((outlet_id, item_id, group_id)): Path<(uuid::Uuid, uuid::Uuid, uuid::Uuid)>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
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
    let rows = sqlx::query(
        "SELECT id, modifier_name, price_adjustment::text as price_adjustment, is_active \
         FROM pos.modifiers \
         WHERE modifier_group_id = $1 ORDER BY modifier_name",
    )
    .bind(group_id)
    .fetch_all(&mut *conn)
    .await
    .map_err(super::err_db)?;
    let items: Vec<serde_json::Value> = rows
        .iter()
        .map(|r| {
            serde_json::json!({
                "id": r.get::<uuid::Uuid, _>("id"),
                "outlet_id": outlet_id,
                "item_id": item_id,
                "group_id": group_id,
                "modifier_name": r.get::<String, _>("modifier_name"),
                "price_adjustment": r.get::<String, _>("price_adjustment"),
                "is_active": r.get::<bool, _>("is_active"),
            })
        })
        .collect();
    conn.commit().await.map_err(super::err_db)?;
    Ok(Json(serde_json::json!({ "items": items })))
}

pub async fn delete_modifier(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path((outlet_id, item_id, group_id, modifier_id)): Path<(
        uuid::Uuid,
        uuid::Uuid,
        uuid::Uuid,
        uuid::Uuid,
    )>,
) -> Result<StatusCode, (StatusCode, String)> {
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
    let res = sqlx::query("DELETE FROM pos.modifiers WHERE id = $1 AND modifier_group_id = $2")
        .bind(modifier_id)
        .bind(group_id)
        .execute(&mut *conn)
        .await
        .map_err(super::err_db)?;
    if res.rows_affected() == 0 {
        return Err(super::err_not_found("modifier not found"));
    }
    emit_tx(&mut *conn, env_id, "hotel8:pos:modifier:deleted", serde_json::json!({ "outlet_id": outlet_id, "item_id": item_id, "group_id": group_id, "modifier_id": modifier_id }),
        &crate::event_emit::Publisher::from_parts(&a.claims.sub, &a.claims.scope)).await.map_err(super::err_emit)?;
    conn.commit().await.map_err(super::err_db)?;
    Ok(StatusCode::NO_CONTENT)
}

// ═══════════════════════════════════════════════════════════════════════════
// Item Availability (86'ing)
// ═══════════════════════════════════════════════════════════════════════════

pub async fn update_item_availability(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path((outlet_id, item_id)): Path<(uuid::Uuid, uuid::Uuid)>,
    Json(req): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let a = auth::verify_token(&state.config, &headers).await?;
    crate::policy::require(
        &state.policy,
        &a,
        &headers,
        sentin8::shield::EvalTarget::new("hotel8:pos:admin"),
    )
    .await?;
    let env_id = super::resolve_org_env(&a);
    let is_active = req
        .get("is_active")
        .and_then(|v| v.as_bool())
        .ok_or_else(|| err_bad_request("is_active is required"))?;
    let reason = req.get("reason").and_then(|v| v.as_str());
    let mut conn = state.pool.begin().await.map_err(super::err_db)?;
    super::set_tenant_context(&mut conn, env_id).await;
    let row = sqlx::query(
        "UPDATE pos.menu_items SET is_active = $1, updated_at = now() \
         WHERE id = $2 AND revenue_center_id = $3 \
         RETURNING id, item_code, item_name, is_active",
    )
    .bind(is_active)
    .bind(item_id)
    .bind(outlet_id)
    .fetch_optional(&mut *conn)
    .await
    .map_err(super::err_db)?
    .ok_or_else(|| super::err_not_found("menu item not found"))?;
    emit_tx(
        &mut *conn,
        env_id,
        "hotel8:pos:menu:item:availability_changed",
        serde_json::json!({
            "outlet_id": outlet_id,
            "item_id": item_id,
            "item_code": row.get::<String, _>("item_code"),
            "is_active": is_active,
            "reason": reason,
        }),
        &crate::event_emit::Publisher::from_parts(&a.claims.sub, &a.claims.scope),
    )
    .await
    .map_err(super::err_emit)?;
    conn.commit().await.map_err(super::err_db)?;
    Ok(Json(serde_json::json!({
        "id": row.get::<uuid::Uuid, _>("id"),
        "outlet_id": outlet_id,
        "item_code": row.get::<String, _>("item_code"),
        "item_name": row.get::<String, _>("item_name"),
        "is_active": row.get::<bool, _>("is_active"),
        "reason": reason,
    })))
}

// ═══════════════════════════════════════════════════════════════════════════
// Bulk Price Update
// ═══════════════════════════════════════════════════════════════════════════

pub async fn bulk_update_prices(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path(outlet_id): Path<uuid::Uuid>,
    Json(req): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let a = auth::verify_token(&state.config, &headers).await?;
    crate::policy::require(
        &state.policy,
        &a,
        &headers,
        sentin8::shield::EvalTarget::new("hotel8:pos:admin"),
    )
    .await?;
    let env_id = super::resolve_org_env(&a);
    let updates = req
        .get("updates")
        .and_then(|v| v.as_array())
        .ok_or_else(|| err_bad_request("updates array is required"))?;
    if updates.is_empty() {
        return Err(err_bad_request("updates array must not be empty"));
    }
    let mut conn = state.pool.begin().await.map_err(super::err_db)?;
    super::set_tenant_context(&mut conn, env_id).await;
    let mut updated: Vec<serde_json::Value> = Vec::with_capacity(updates.len());
    for u in updates {
        let item_id = u
            .get("item_id")
            .and_then(|v| v.as_str())
            .and_then(|s| uuid::Uuid::parse_str(s).ok())
            .ok_or_else(|| err_bad_request("each update requires item_id"))?;
        let price: String = u
            .get("price")
            .and_then(|v| v.as_str())
            .ok_or_else(|| err_bad_request("each update requires price as string"))?
            .to_string();
        let row = sqlx::query(
            "UPDATE pos.menu_items SET price = $1, updated_at = now() \
             WHERE id = $2 AND revenue_center_id = $3 \
             RETURNING id, item_code, price::text as price",
        )
        .bind(&price)
        .bind(item_id)
        .bind(outlet_id)
        .fetch_optional(&mut *conn)
        .await
        .map_err(super::err_db)?
        .ok_or_else(|| super::err_not_found("menu item not found"))?;
        updated.push(serde_json::json!({
            "item_id": row.get::<uuid::Uuid, _>("id"),
            "item_code": row.get::<String, _>("item_code"),
            "price": row.get::<String, _>("price"),
        }));
    }
    emit_tx(
        &mut *conn,
        env_id,
        "hotel8:pos:menu:items:prices_updated",
        serde_json::json!({ "outlet_id": outlet_id, "count": updated.len() }),
        &crate::event_emit::Publisher::from_parts(&a.claims.sub, &a.claims.scope),
    )
    .await
    .map_err(super::err_emit)?;
    conn.commit().await.map_err(super::err_db)?;
    Ok(Json(serde_json::json!({ "updated": updated })))
}

// ═══════════════════════════════════════════════════════════════════════════
// Tax Group CRUD
// ═══════════════════════════════════════════════════════════════════════════

pub async fn create_tax_group(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path(outlet_id): Path<uuid::Uuid>,
    Json(req): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let a = auth::verify_token(&state.config, &headers).await?;
    crate::policy::require(
        &state.policy,
        &a,
        &headers,
        sentin8::shield::EvalTarget::new("hotel8:pos:admin"),
    )
    .await?;
    let env_id = super::resolve_org_env(&a);
    let tax_code = req.get("tax_code").and_then(|v| v.as_str()).unwrap_or("");
    let tax_name = req.get("tax_name").and_then(|v| v.as_str()).unwrap_or("");
    let tax_rate: String = req
        .get("tax_rate")
        .and_then(|v| v.as_str())
        .unwrap_or("0")
        .to_string();
    if tax_code.is_empty() || tax_name.is_empty() {
        return Err(err_bad_request("tax_code and tax_name are required"));
    }
    let mut conn = state.pool.begin().await.map_err(super::err_db)?;
    super::set_tenant_context(&mut conn, env_id).await;
    let row = sqlx::query(
        "INSERT INTO pos.tax_groups \
         (revenue_center_id, tax_code, tax_name, tax_rate) \
         VALUES ($1, $2, $3, $4) \
         RETURNING id, tax_code, tax_name, tax_rate::text as tax_rate, is_active",
    )
    .bind(outlet_id)
    .bind(tax_code)
    .bind(tax_name)
    .bind(&tax_rate)
    .fetch_one(&mut *conn)
    .await
    .map_err(super::err_db)?;
    let id: uuid::Uuid = row.get("id");
    emit_tx(
        &mut *conn,
        env_id,
        "hotel8:pos:tax_group:created",
        serde_json::json!({ "outlet_id": outlet_id, "tax_id": id, "tax_code": tax_code }),
        &crate::event_emit::Publisher::from_parts(&a.claims.sub, &a.claims.scope),
    )
    .await
    .map_err(super::err_emit)?;
    conn.commit().await.map_err(super::err_db)?;
    Ok(Json(serde_json::json!({
        "id": id,
        "outlet_id": outlet_id,
        "tax_code": row.get::<String, _>("tax_code"),
        "tax_name": row.get::<String, _>("tax_name"),
        "tax_rate": row.get::<String, _>("tax_rate"),
        "is_active": row.get::<bool, _>("is_active"),
    })))
}

pub async fn list_tax_groups(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path(outlet_id): Path<uuid::Uuid>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
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
    let rows = sqlx::query(
        "SELECT id, tax_code, tax_name, tax_rate::text as tax_rate, is_active \
         FROM pos.tax_groups \
         WHERE revenue_center_id = $1 ORDER BY tax_code",
    )
    .bind(outlet_id)
    .fetch_all(&mut *conn)
    .await
    .map_err(super::err_db)?;
    let items: Vec<serde_json::Value> = rows
        .iter()
        .map(|r| {
            serde_json::json!({
                "id": r.get::<uuid::Uuid, _>("id"),
                "outlet_id": outlet_id,
                "tax_code": r.get::<String, _>("tax_code"),
                "tax_name": r.get::<String, _>("tax_name"),
                "tax_rate": r.get::<String, _>("tax_rate"),
                "is_active": r.get::<bool, _>("is_active"),
            })
        })
        .collect();
    conn.commit().await.map_err(super::err_db)?;
    Ok(Json(serde_json::json!({ "items": items })))
}

pub async fn update_tax_group(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path((outlet_id, tax_id)): Path<(uuid::Uuid, uuid::Uuid)>,
    Json(req): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let a = auth::verify_token(&state.config, &headers).await?;
    crate::policy::require(
        &state.policy,
        &a,
        &headers,
        sentin8::shield::EvalTarget::new("hotel8:pos:admin"),
    )
    .await?;
    let env_id = super::resolve_org_env(&a);
    let tax_name = req.get("tax_name").and_then(|v| v.as_str());
    let tax_rate: Option<String> = req
        .get("tax_rate")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let is_active = req.get("is_active").and_then(|v| v.as_bool());
    let mut conn = state.pool.begin().await.map_err(super::err_db)?;
    super::set_tenant_context(&mut conn, env_id).await;
    let row = sqlx::query(
        "UPDATE pos.tax_groups SET \
            tax_name = COALESCE($1, tax_name), \
            tax_rate = COALESCE($2::numeric, tax_rate), \
            is_active = COALESCE($3, is_active), \
            updated_at = now() \
         WHERE id = $4 AND revenue_center_id = $5 \
         RETURNING id, tax_code, tax_name, tax_rate::text as tax_rate, is_active",
    )
    .bind(tax_name)
    .bind(tax_rate.as_deref())
    .bind(is_active)
    .bind(tax_id)
    .bind(outlet_id)
    .fetch_optional(&mut *conn)
    .await
    .map_err(super::err_db)?
    .ok_or_else(|| super::err_not_found("tax group not found"))?;
    emit_tx(
        &mut *conn,
        env_id,
        "hotel8:pos:tax_group:updated",
        serde_json::json!({ "outlet_id": outlet_id, "tax_id": tax_id }),
        &crate::event_emit::Publisher::from_parts(&a.claims.sub, &a.claims.scope),
    )
    .await
    .map_err(super::err_emit)?;
    conn.commit().await.map_err(super::err_db)?;
    Ok(Json(serde_json::json!({
        "id": row.get::<uuid::Uuid, _>("id"),
        "outlet_id": outlet_id,
        "tax_code": row.get::<String, _>("tax_code"),
        "tax_name": row.get::<String, _>("tax_name"),
        "tax_rate": row.get::<String, _>("tax_rate"),
        "is_active": row.get::<bool, _>("is_active"),
    })))
}

pub async fn delete_tax_group(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path((outlet_id, tax_id)): Path<(uuid::Uuid, uuid::Uuid)>,
) -> Result<StatusCode, (StatusCode, String)> {
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
    let res = sqlx::query("DELETE FROM pos.tax_groups WHERE id = $1 AND revenue_center_id = $2")
        .bind(tax_id)
        .bind(outlet_id)
        .execute(&mut *conn)
        .await
        .map_err(super::err_db)?;
    if res.rows_affected() == 0 {
        return Err(super::err_not_found("tax group not found"));
    }
    emit_tx(
        &mut *conn,
        env_id,
        "hotel8:pos:tax_group:deleted",
        serde_json::json!({ "outlet_id": outlet_id, "tax_id": tax_id }),
        &crate::event_emit::Publisher::from_parts(&a.claims.sub, &a.claims.scope),
    )
    .await
    .map_err(super::err_emit)?;
    conn.commit().await.map_err(super::err_db)?;
    Ok(StatusCode::NO_CONTENT)
}

// ═══════════════════════════════════════════════════════════════════════════
// Printer Group CRUD
// ═══════════════════════════════════════════════════════════════════════════

pub async fn create_printer_group(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path(outlet_id): Path<uuid::Uuid>,
    Json(req): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let a = auth::verify_token(&state.config, &headers).await?;
    crate::policy::require(
        &state.policy,
        &a,
        &headers,
        sentin8::shield::EvalTarget::new("hotel8:pos:admin"),
    )
    .await?;
    let env_id = super::resolve_org_env(&a);
    let printer_code = req
        .get("printer_code")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let printer_name = req
        .get("printer_name")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let station_code = req.get("station_code").and_then(|v| v.as_str());
    if printer_code.is_empty() || printer_name.is_empty() {
        return Err(err_bad_request(
            "printer_code and printer_name are required",
        ));
    }
    let mut conn = state.pool.begin().await.map_err(super::err_db)?;
    super::set_tenant_context(&mut conn, env_id).await;
    let row = sqlx::query(
        "INSERT INTO pos.printer_groups \
         (revenue_center_id, printer_code, printer_name, station_code) \
         VALUES ($1, $2, $3, $4) \
         RETURNING id, printer_code, printer_name, station_code, is_active",
    )
    .bind(outlet_id)
    .bind(printer_code)
    .bind(printer_name)
    .bind(station_code)
    .fetch_one(&mut *conn)
    .await
    .map_err(super::err_db)?;
    let id: uuid::Uuid = row.get("id");
    emit_tx(&mut *conn, env_id, "hotel8:pos:printer_group:created", serde_json::json!({ "outlet_id": outlet_id, "printer_id": id, "printer_code": printer_code }),
        &crate::event_emit::Publisher::from_parts(&a.claims.sub, &a.claims.scope)).await.map_err(super::err_emit)?;
    conn.commit().await.map_err(super::err_db)?;
    Ok(Json(serde_json::json!({
        "id": id,
        "outlet_id": outlet_id,
        "printer_code": row.get::<String, _>("printer_code"),
        "printer_name": row.get::<String, _>("printer_name"),
        "station_code": row.get::<Option<String>, _>("station_code"),
        "is_active": row.get::<bool, _>("is_active"),
    })))
}

pub async fn list_printer_groups(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path(outlet_id): Path<uuid::Uuid>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
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
    let rows = sqlx::query(
        "SELECT id, printer_code, printer_name, station_code, is_active \
         FROM pos.printer_groups \
         WHERE revenue_center_id = $1 ORDER BY printer_code",
    )
    .bind(outlet_id)
    .fetch_all(&mut *conn)
    .await
    .map_err(super::err_db)?;
    let items: Vec<serde_json::Value> = rows
        .iter()
        .map(|r| {
            serde_json::json!({
                "id": r.get::<uuid::Uuid, _>("id"),
                "outlet_id": outlet_id,
                "printer_code": r.get::<String, _>("printer_code"),
                "printer_name": r.get::<String, _>("printer_name"),
                "station_code": r.get::<Option<String>, _>("station_code"),
                "is_active": r.get::<bool, _>("is_active"),
            })
        })
        .collect();
    conn.commit().await.map_err(super::err_db)?;
    Ok(Json(serde_json::json!({ "items": items })))
}

pub async fn update_printer_group(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path((outlet_id, printer_id)): Path<(uuid::Uuid, uuid::Uuid)>,
    Json(req): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let a = auth::verify_token(&state.config, &headers).await?;
    crate::policy::require(
        &state.policy,
        &a,
        &headers,
        sentin8::shield::EvalTarget::new("hotel8:pos:admin"),
    )
    .await?;
    let env_id = super::resolve_org_env(&a);
    let printer_name = req.get("printer_name").and_then(|v| v.as_str());
    let station_code = req.get("station_code").and_then(|v| v.as_str());
    let is_active = req.get("is_active").and_then(|v| v.as_bool());
    let mut conn = state.pool.begin().await.map_err(super::err_db)?;
    super::set_tenant_context(&mut conn, env_id).await;
    let row = sqlx::query(
        "UPDATE pos.printer_groups SET \
            printer_name = COALESCE($1, printer_name), \
            station_code = COALESCE($2, station_code), \
            is_active = COALESCE($3, is_active), \
            updated_at = now() \
         WHERE id = $4 AND revenue_center_id = $5 \
         RETURNING id, printer_code, printer_name, station_code, is_active",
    )
    .bind(printer_name)
    .bind(station_code)
    .bind(is_active)
    .bind(printer_id)
    .bind(outlet_id)
    .fetch_optional(&mut *conn)
    .await
    .map_err(super::err_db)?
    .ok_or_else(|| super::err_not_found("printer group not found"))?;
    emit_tx(
        &mut *conn,
        env_id,
        "hotel8:pos:printer_group:updated",
        serde_json::json!({ "outlet_id": outlet_id, "printer_id": printer_id }),
        &crate::event_emit::Publisher::from_parts(&a.claims.sub, &a.claims.scope),
    )
    .await
    .map_err(super::err_emit)?;
    conn.commit().await.map_err(super::err_db)?;
    Ok(Json(serde_json::json!({
        "id": row.get::<uuid::Uuid, _>("id"),
        "outlet_id": outlet_id,
        "printer_code": row.get::<String, _>("printer_code"),
        "printer_name": row.get::<String, _>("printer_name"),
        "station_code": row.get::<Option<String>, _>("station_code"),
        "is_active": row.get::<bool, _>("is_active"),
    })))
}

pub async fn delete_printer_group(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path((outlet_id, printer_id)): Path<(uuid::Uuid, uuid::Uuid)>,
) -> Result<StatusCode, (StatusCode, String)> {
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
    let res =
        sqlx::query("DELETE FROM pos.printer_groups WHERE id = $1 AND revenue_center_id = $2")
            .bind(printer_id)
            .bind(outlet_id)
            .execute(&mut *conn)
            .await
            .map_err(super::err_db)?;
    if res.rows_affected() == 0 {
        return Err(super::err_not_found("printer group not found"));
    }
    emit_tx(
        &mut *conn,
        env_id,
        "hotel8:pos:printer_group:deleted",
        serde_json::json!({ "outlet_id": outlet_id, "printer_id": printer_id }),
        &crate::event_emit::Publisher::from_parts(&a.claims.sub, &a.claims.scope),
    )
    .await
    .map_err(super::err_emit)?;
    conn.commit().await.map_err(super::err_db)?;
    Ok(StatusCode::NO_CONTENT)
}

// ═══════════════════════════════════════════════════════════════════════════
// Payment Method configuration
// ═══════════════════════════════════════════════════════════════════════════

pub async fn get_payment_methods(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path(outlet_id): Path<uuid::Uuid>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let _a = auth::verify_token(&state.config, &headers).await?;
    crate::policy::require(
        &state.policy,
        &_a,
        &headers,
        sentin8::shield::EvalTarget::new("hotel8:pos:admin"),
    )
    .await?;
    // No dedicated payment-methods table: return the static supported list,
    // all enabled by default. Future enhancement: persist overrides on the
    // revenue center as a JSONB column.
    let methods = serde_json::json!([
        { "type": "CASH", "is_enabled": true },
        { "type": "CREDIT_CARD", "is_enabled": true },
        { "type": "DEBIT_CARD", "is_enabled": true },
        { "type": "ROOM_CHARGE", "is_enabled": true },
        { "type": "GIFT_CARD", "is_enabled": true },
        { "type": "MOBILE_WALLET", "is_enabled": true },
        { "type": "COMP", "is_enabled": true },
    ]);
    Ok(Json(serde_json::json!({
        "outlet_id": outlet_id,
        "methods": methods,
    })))
}

pub async fn update_payment_methods(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path(outlet_id): Path<uuid::Uuid>,
    Json(req): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let a = auth::verify_token(&state.config, &headers).await?;
    crate::policy::require(
        &state.policy,
        &a,
        &headers,
        sentin8::shield::EvalTarget::new("hotel8:pos:admin"),
    )
    .await?;
    let env_id = super::resolve_org_env(&a);
    let methods = req
        .get("methods")
        .and_then(|v| v.as_array())
        .ok_or_else(|| err_bad_request("methods array is required"))?;
    // Validate each entry.
    for m in methods {
        let _t = m
            .get("type")
            .and_then(|v| v.as_str())
            .ok_or_else(|| err_bad_request("each method requires a type"))?;
        let _enabled = m
            .get("is_enabled")
            .and_then(|v| v.as_bool())
            .ok_or_else(|| err_bad_request("each method requires is_enabled (bool)"))?;
    }
    emit_on_pool(
        &state.pool,
        env_id,
        "hotel8:pos:payment_methods:updated",
        serde_json::json!({ "outlet_id": outlet_id, "methods": methods }),
        &crate::event_emit::Publisher::from_parts(&a.claims.sub, &a.claims.scope),
    )
    .await
    .map_err(super::err_emit)?;
    Ok(Json(serde_json::json!({
        "outlet_id": outlet_id,
        "methods": methods,
    })))
}
