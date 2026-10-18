// SPDX-FileCopyrightText: Copyright Logically Magic Inc. <hello@logicallymagic.com>
// SPDX-FileCopyrightText: Copyright Hamid Jahanzad <Hamid@Jahanzad.com>
// SPDX-FileCopyrightText: Copyright Contributors to the Hotel8:POS
// SPDX-License-Identifier: Apache-2.0

//! Revenue center (outlet) management handlers.
use crate::auth;
use crate::event_emit::emit_tx;
use crate::handlers::AppState;
use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use sqlx::Row;
#[utoipa::path(
    post,
    path = "/v1/outlets",
    responses(
        (status = 200, description = "OK", body = serde_json::Value),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Forbidden"),
    ),
    security(),
    tag = "pos",
)]

pub async fn create_outlet(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
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
    let property_id: uuid::Uuid = req
        .get("property_id")
        .and_then(|v| v.as_str())
        .and_then(|s| uuid::Uuid::parse_str(s).ok())
        .ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                "property_id is required".to_string(),
            )
        })?;
    let mut conn = state.pool.begin().await.map_err(super::err_db)?;
    super::set_tenant_context(&mut conn, env_id).await;
    let outlet_code = req
        .get("outlet_code")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let outlet_name = req
        .get("outlet_name")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let outlet_type = req
        .get("outlet_type")
        .and_then(|v| v.as_str())
        .unwrap_or("RESTAURANT");
    let currency_code = req
        .get("currency_code")
        .and_then(|v| v.as_str())
        .unwrap_or("USD");
    let row = sqlx::query("INSERT INTO pos.revenue_centers (property_id, env_id, outlet_code, outlet_name, outlet_type, currency_code) VALUES ($1, $2, $3, $4, $5, $6) RETURNING id, outlet_code, outlet_name, outlet_type, currency_code")
        .bind(property_id)
        .bind(env_id)
        .bind(outlet_code)
        .bind(outlet_name)
        .bind(outlet_type)
        .bind(currency_code)
        .fetch_one(&mut *conn)
        .await
        .map_err(super::err_db)?;
    let id: uuid::Uuid = row.get("id");
    emit_tx(
        &mut *conn,
        env_id,
        "hotel8:pos:outlet:created",
        serde_json::json!({ "outlet_id": id, "outlet_code": outlet_code }),
        &crate::event_emit::Publisher::from_parts(&a.claims.sub, &a.claims.scope),
    )
    .await
    .map_err(super::err_emit)?;
    conn.commit().await.map_err(super::err_db)?;
    Ok(Json(serde_json::json!({
        "id": id,
        "property_id": property_id,
        "outlet_code": row.get::<String, _>("outlet_code"),
        "outlet_name": row.get::<String, _>("outlet_name"),
        "outlet_type": row.get::<String, _>("outlet_type"),
        "currency_code": row.get::<String, _>("currency_code"),
    })))
}
#[utoipa::path(
    get,
    path = "/v1/outlets",
    responses(
        (status = 200, description = "OK", body = serde_json::Value),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Forbidden"),
    ),
    security(),
    tag = "pos",
)]

pub async fn list_outlets(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    axum::extract::Query(params): axum::extract::Query<serde_json::Value>,
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
    let property_id = params
        .get("property_id")
        .and_then(|v| v.as_str())
        .and_then(|s| uuid::Uuid::parse_str(s).ok());
    let rows = if let Some(pid) = property_id {
        sqlx::query("SELECT id, property_id, outlet_code, outlet_name, outlet_type, currency_code, is_active FROM pos.revenue_centers WHERE property_id = $1 ORDER BY outlet_code")
            .bind(pid)
            .fetch_all(&mut *conn)
            .await
            .map_err(super::err_db)?
    } else {
        sqlx::query("SELECT id, property_id, outlet_code, outlet_name, outlet_type, currency_code, is_active FROM pos.revenue_centers ORDER BY outlet_code")
            .fetch_all(&mut *conn)
            .await
            .map_err(super::err_db)?
    };
    let items: Vec<serde_json::Value> = rows
        .iter()
        .map(|r| {
            serde_json::json!({
                "id": r.get::<uuid::Uuid, _>("id"),
                "property_id": r.get::<uuid::Uuid, _>("property_id"),
                "outlet_code": r.get::<String, _>("outlet_code"),
                "outlet_name": r.get::<String, _>("outlet_name"),
                "outlet_type": r.get::<String, _>("outlet_type"),
                "currency_code": r.get::<String, _>("currency_code"),
                "is_active": r.get::<bool, _>("is_active"),
            })
        })
        .collect();
    conn.commit().await.map_err(super::err_db)?;
    Ok(Json(serde_json::json!({ "items": items })))
}
