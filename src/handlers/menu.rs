// SPDX-FileCopyrightText: Copyright Logically Magic Inc. <hello@logicallymagic.com>
// SPDX-FileCopyrightText: Copyright Hamid Jahanzad <Hamid@Jahanzad.com>
// SPDX-FileCopyrightText: Copyright Contributors to the Hotel8:POS
// SPDX-License-Identifier: Apache-2.0

//! Menu management handlers — categories, items, modifiers.
use crate::auth;
use crate::event_emit::emit_tx;
use crate::handlers::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use sqlx::Row;

pub async fn create_menu_item(
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
    let mut conn = state.pool.begin().await.map_err(super::err_db)?;
    super::set_tenant_context(&mut conn, env_id).await;
    let item_code = req.get("item_code").and_then(|v| v.as_str()).unwrap_or("");
    let item_name = req.get("item_name").and_then(|v| v.as_str()).unwrap_or("");
    let category_id = req
        .get("category_id")
        .and_then(|v| v.as_str())
        .and_then(|s| uuid::Uuid::parse_str(s).ok());
    let price: String = req
        .get("price")
        .and_then(|v| v.as_str())
        .unwrap_or("0")
        .to_string();
    let tax_rate: String = req
        .get("tax_rate")
        .and_then(|v| v.as_str())
        .unwrap_or("0")
        .to_string();
    let description = req.get("description").and_then(|v| v.as_str());
    let row = sqlx::query("INSERT INTO pos.menu_items (revenue_center_id, category_id, item_code, item_name, description, price, tax_rate) VALUES ($1, $2, $3, $4, $5, $6, $7) RETURNING id, item_code, item_name, price::text as price, tax_rate::text as tax_rate")
        .bind(outlet_id)
        .bind(category_id)
        .bind(item_code)
        .bind(item_name)
        .bind(description)
        .bind(&price)
        .bind(&tax_rate)
        .fetch_one(&mut *conn)
        .await
        .map_err(super::err_db)?;
    let id: uuid::Uuid = row.get("id");
    emit_tx(
        &mut *conn,
        env_id,
        "hotel8:pos:menu_item:created",
        serde_json::json!({ "item_id": id, "item_code": item_code }),
        &crate::event_emit::Publisher::from_parts(&a.claims.sub, &a.claims.scope),
    )
    .await
    .map_err(super::err_emit)?;
    conn.commit().await.map_err(super::err_db)?;
    Ok(Json(serde_json::json!({
        "id": id,
        "outlet_id": outlet_id,
        "item_code": row.get::<String, _>("item_code"),
        "item_name": row.get::<String, _>("item_name"),
        "price": row.get::<String, _>("price"),
        "tax_rate": row.get::<String, _>("tax_rate"),
    })))
}

pub async fn list_menu_items(
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
    let rows = sqlx::query("SELECT id, item_code, item_name, description, price::text as price, tax_rate::text as tax_rate, is_active FROM pos.menu_items WHERE revenue_center_id = $1 ORDER BY item_code")
        .bind(outlet_id)
        .fetch_all(&mut *conn)
        .await
        .map_err(super::err_db)?;
    let items: Vec<serde_json::Value> = rows
        .iter()
        .map(|r| {
            serde_json::json!({
                "id": r.get::<uuid::Uuid, _>("id"),
                "item_code": r.get::<String, _>("item_code"),
                "item_name": r.get::<String, _>("item_name"),
                "description": r.get::<Option<String>, _>("description"),
                "price": r.get::<String, _>("price"),
                "tax_rate": r.get::<String, _>("tax_rate"),
                "is_active": r.get::<bool, _>("is_active"),
            })
        })
        .collect();
    conn.commit().await.map_err(super::err_db)?;
    Ok(Json(serde_json::json!({ "items": items })))
}
