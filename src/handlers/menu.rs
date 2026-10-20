// SPDX-FileCopyrightText: Copyright Logically Magic Inc. <hello@logicallymagic.com>
// SPDX-FileCopyrightText: Copyright Hamid Jahanzad <Hamid@Jahanzad.com>
// SPDX-FileCopyrightText: Copyright Contributors to the Hotel8:POS
// SPDX-License-Identifier: Apache-2.0

//! Menu management handlers - categories, items, modifiers.
use crate::auth;
use crate::event_emit::emit_tx;
use crate::handlers::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use hotel8::pos::contracts::{MenuItemCreateRequest, MenuItemCreated, MenuItemList, MenuItemRow};
use sqlx::Row;
#[utoipa::path(
    post,
    path = "/v1/outlets/{outlet_id}/menu-items",
    request_body = MenuItemCreateRequest,
    responses(
        (status = 200, description = "Created menu item (price/tax_rate as decimal strings)", body = MenuItemCreated),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Forbidden"),
    ),
    security(),
    tag = "pos",
)]

pub async fn create_menu_item(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path(outlet_id): Path<uuid::Uuid>,
    Json(req): Json<MenuItemCreateRequest>,
) -> Result<Json<MenuItemCreated>, (StatusCode, String)> {
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
    let item_code = req.item_code.clone();
    let item_name = req.item_name.clone();
    let category_id = req.category_id;
    let price: String = req.price.clone();
    let tax_rate: String = req.tax_rate.clone();
    let description = req.description.clone();
    let row = sqlx::query("INSERT INTO pos.menu_items (revenue_center_id, category_id, item_code, item_name, description, price, tax_rate) VALUES ($1, $2, $3, $4, $5, $6, $7) RETURNING id, item_code, item_name, price::text as price, tax_rate::text as tax_rate")
        .bind(outlet_id)
        .bind(category_id)
        .bind(&item_code)
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
    Ok(Json(MenuItemCreated {
        id,
        outlet_id,
        item_code: row.get::<String, _>("item_code"),
        item_name: row.get::<String, _>("item_name"),
        price: row.get::<String, _>("price"),
        tax_rate: row.get::<String, _>("tax_rate"),
    }))
}
#[utoipa::path(
    get,
    path = "/v1/outlets/{outlet_id}/menu-items",
    responses(
        (status = 200, description = "Menu-item rows (price/tax_rate as decimal strings)", body = MenuItemList),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Forbidden"),
    ),
    security(),
    tag = "pos",
)]

pub async fn list_menu_items(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path(outlet_id): Path<uuid::Uuid>,
) -> Result<Json<MenuItemList>, (StatusCode, String)> {
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
    let items: Vec<MenuItemRow> = rows
        .iter()
        .map(|r| MenuItemRow {
            id: r.get::<uuid::Uuid, _>("id"),
            item_code: r.get::<String, _>("item_code"),
            item_name: r.get::<String, _>("item_name"),
            description: r.get::<Option<String>, _>("description"),
            price: r.get::<String, _>("price"),
            tax_rate: r.get::<String, _>("tax_rate"),
            is_active: r.get::<bool, _>("is_active"),
        })
        .collect();
    conn.commit().await.map_err(super::err_db)?;
    Ok(Json(MenuItemList { items }))
}
