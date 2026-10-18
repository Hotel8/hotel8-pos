// SPDX-FileCopyrightText: Copyright Logically Magic Inc. <hello@logicallymagic.com>
// SPDX-FileCopyrightText: Copyright Hamid Jahanzad <Hamid@Jahanzad.com>
// SPDX-FileCopyrightText: Copyright Contributors to the Hotel8:POS
// SPDX-License-Identifier: Apache-2.0

//! H8POS reporting handlers — -.
use crate::auth;
use crate::event_emit::{emit_on_pool, emit_tx};
use crate::handlers::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use sqlx::Row;

// ── Daily Sales Summary ──

/// `GET /v1/outlets/{outlet_id}/reports/sales`
#[utoipa::path(
    get,
    path = "/v1/outlets/{outlet_id}/reports/sales",
    responses(
        (status = 200, description = "Daily sales summary report", body = serde_json::Value),
        (status = 403, description = "Missing pos:admin scope"),
    ),
    security(("pos:admin" = [])),
    tag = "pos",
)]
pub async fn daily_sales_summary(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path(outlet_id): Path<uuid::Uuid>,
    axum::extract::Query(params): axum::extract::Query<serde_json::Value>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let a = auth::verify_token(&state.config, &headers).await?;
    crate::policy::require(
        &state.policy,
        &a,
        &headers,
        sentin8::shield::EvalTarget::new("hotel8:pos:report:read"),
    )
    .await?;
    let env_id = super::resolve_org_env(&a);
    let mut conn = state.pool.begin().await.map_err(super::err_db)?;
    super::set_tenant_context(&mut conn, env_id).await;
    let date = params
        .get("date")
        .and_then(|v| v.as_str())
        .unwrap_or("today");
    let row = sqlx::query(
        "SELECT
            COUNT(*) as total_checks,
            COUNT(*) FILTER (WHERE status = 'PAID') as paid_checks,
            COUNT(*) FILTER (WHERE status = 'VOIDED') as voided_checks,
            COALESCE(SUM(total) FILTER (WHERE status NOT IN ('VOIDED')), 0)::text as gross_sales,
            COALESCE(SUM(subtotal) FILTER (WHERE status NOT IN ('VOIDED')), 0)::text as net_sales,
            COALESCE(SUM(tax_total) FILTER (WHERE status NOT IN ('VOIDED')), 0)::text as tax_collected,
            (SELECT COALESCE(SUM(tip_amount), 0)::text FROM pos.payments p JOIN pos.checks c ON p.check_id = c.id WHERE c.revenue_center_id = $1 AND c.opened_at::date = $2::date) as total_tips
         FROM pos.checks
         WHERE revenue_center_id = $1 AND opened_at::date = $2::date",
    )
    .bind(outlet_id).bind(date)
    .fetch_one(&mut *conn).await.map_err(super::err_db)?;
    conn.commit().await.map_err(super::err_db)?;
    Ok(Json(serde_json::json!({
        "outlet_id": outlet_id, "date": date,
        "total_checks": row.get::<i64, _>("total_checks"),
        "paid_checks": row.get::<i64, _>("paid_checks"),
        "voided_checks": row.get::<i64, _>("voided_checks"),
        "gross_sales": row.get::<String, _>("gross_sales"),
        "net_sales": row.get::<String, _>("net_sales"),
        "tax_collected": row.get::<String, _>("tax_collected"),
        "total_tips": row.get::<String, _>("total_tips"),
    })))
}

// ── Item Sales Report ──
#[utoipa::path(
    get,
    path = "/v1/outlets/{outlet_id}/reports/items",
    responses(
        (status = 200, description = "OK", body = serde_json::Value),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Forbidden"),
    ),
    security(),
    tag = "pos",
)]

pub async fn item_sales_report(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path(outlet_id): Path<uuid::Uuid>,
    axum::extract::Query(params): axum::extract::Query<serde_json::Value>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let a = auth::verify_token(&state.config, &headers).await?;
    crate::policy::require(
        &state.policy,
        &a,
        &headers,
        sentin8::shield::EvalTarget::new("hotel8:pos:report:read"),
    )
    .await?;
    let env_id = super::resolve_org_env(&a);
    let mut conn = state.pool.begin().await.map_err(super::err_db)?;
    super::set_tenant_context(&mut conn, env_id).await;
    let start = params
        .get("startDate")
        .and_then(|v| v.as_str())
        .unwrap_or("2020-01-01");
    let end = params
        .get("endDate")
        .and_then(|v| v.as_str())
        .unwrap_or("2030-12-31");
    let rows = sqlx::query(
        "SELECT o.menu_item_id, m.item_name, m.item_code,
                SUM(o.quantity) as total_quantity,
                SUM(o.line_total)::text as total_revenue,
                COUNT(*) as order_count
         FROM pos.orders o
         JOIN pos.checks c ON o.check_id = c.id
         JOIN pos.menu_items m ON o.menu_item_id = m.id
         WHERE c.revenue_center_id = $1 AND c.opened_at >= $2::date AND c.opened_at < $3::date
           AND o.status != 'CANCELLED'
         GROUP BY o.menu_item_id, m.item_name, m.item_code
         ORDER BY total_revenue DESC",
    )
    .bind(outlet_id)
    .bind(start)
    .bind(end)
    .fetch_all(&mut *conn)
    .await
    .map_err(super::err_db)?;
    let items: Vec<serde_json::Value> = rows
        .iter()
        .map(|r| {
            serde_json::json!({
                "menu_item_id": r.get::<uuid::Uuid, _>("menu_item_id"),
                "item_name": r.get::<String, _>("item_name"),
                "item_code": r.get::<String, _>("item_code"),
                "total_quantity": r.get::<i64, _>("total_quantity"),
                "total_revenue": r.get::<String, _>("total_revenue"),
                "order_count": r.get::<i64, _>("order_count"),
            })
        })
        .collect();
    conn.commit().await.map_err(super::err_db)?;
    Ok(Json(
        serde_json::json!({ "outlet_id": outlet_id, "startDate": start, "endDate": end, "items": items }),
    ))
}

// ── Labor Report ──
#[utoipa::path(
    get,
    path = "/v1/outlets/{outlet_id}/reports/labor",
    responses(
        (status = 200, description = "OK", body = serde_json::Value),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Forbidden"),
    ),
    security(),
    tag = "pos",
)]

pub async fn labor_report(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path(outlet_id): Path<uuid::Uuid>,
    axum::extract::Query(params): axum::extract::Query<serde_json::Value>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let a = auth::verify_token(&state.config, &headers).await?;
    crate::policy::require(
        &state.policy,
        &a,
        &headers,
        sentin8::shield::EvalTarget::new("hotel8:pos:report:read"),
    )
    .await?;
    let env_id = super::resolve_org_env(&a);
    let mut conn = state.pool.begin().await.map_err(super::err_db)?;
    super::set_tenant_context(&mut conn, env_id).await;
    let date = params
        .get("date")
        .and_then(|v| v.as_str())
        .unwrap_or("today");
    // Labor data would come from an HR/scheduling system — return placeholder
    conn.commit().await.map_err(super::err_db)?;
    Ok(Json(serde_json::json!({
        "outlet_id": outlet_id, "date": date,
        "labor": [
            { "role": "SERVER", "headcount": 5, "hours": 40.0, "labor_cost": "800.00" },
            { "role": "BARTENDER", "headcount": 2, "hours": 16.0, "labor_cost": "320.00" },
            { "role": "COOK", "headcount": 4, "hours": 32.0, "labor_cost": "640.00" },
        ],
        "total_labor_cost": "1760.00",
        "note": "Labor data is a placeholder — integrate with HR/scheduling system",
    })))
}

// ── End of Day Report ──
#[utoipa::path(
    get,
    path = "/v1/outlets/{outlet_id}/reports/end-of-day",
    responses(
        (status = 200, description = "OK", body = serde_json::Value),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Forbidden"),
    ),
    security(),
    tag = "pos",
)]

pub async fn end_of_day_report(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path(outlet_id): Path<uuid::Uuid>,
    axum::extract::Query(params): axum::extract::Query<serde_json::Value>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let a = auth::verify_token(&state.config, &headers).await?;
    crate::policy::require(
        &state.policy,
        &a,
        &headers,
        sentin8::shield::EvalTarget::new("hotel8:pos:report:read"),
    )
    .await?;
    let env_id = super::resolve_org_env(&a);
    let mut conn = state.pool.begin().await.map_err(super::err_db)?;
    super::set_tenant_context(&mut conn, env_id).await;
    let date = params
        .get("date")
        .and_then(|v| v.as_str())
        .unwrap_or("today");
    // Sales summary
    let sales = sqlx::query(
        "SELECT
            COUNT(*) as total_checks,
            COALESCE(SUM(total) FILTER (WHERE status NOT IN ('VOIDED')), 0)::text as gross_sales,
            COALESCE(SUM(tax_total) FILTER (WHERE status NOT IN ('VOIDED')), 0)::text as tax_collected
         FROM pos.checks WHERE revenue_center_id = $1 AND opened_at::date = $2::date",
    )
    .bind(outlet_id).bind(date)
    .fetch_one(&mut *conn).await.map_err(super::err_db)?;
    // Payment breakdown
    let payments = sqlx::query(
        "SELECT payment_type,
                COUNT(*) as count,
                SUM(amount)::text as total_amount,
                SUM(tip_amount)::text as total_tips
         FROM pos.payments p
         JOIN pos.checks c ON p.check_id = c.id
         WHERE c.revenue_center_id = $1 AND c.opened_at::date = $2::date
         GROUP BY payment_type ORDER BY total_amount DESC",
    )
    .bind(outlet_id)
    .bind(date)
    .fetch_all(&mut *conn)
    .await
    .map_err(super::err_db)?;
    let payment_breakdown: Vec<serde_json::Value> = payments
        .iter()
        .map(|r| {
            let pt: String = r.get("payment_type");
            serde_json::json!({
                "payment_type": pt,
                "count": r.get::<i64, _>("count"),
                "total_amount": r.get::<String, _>("total_amount"),
                "total_tips": r.get::<String, _>("total_tips"),
            })
        })
        .collect();
    emit_tx(
        &mut *conn,
        env_id,
        "hotel8:pos:report:eod_generated",
        serde_json::json!({ "outlet_id": outlet_id, "date": date }),
        &crate::event_emit::Publisher::from_parts(&a.claims.sub, &a.claims.scope),
    )
    .await
    .map_err(super::err_emit)?;
    conn.commit().await.map_err(super::err_db)?;
    Ok(Json(serde_json::json!({
        "outlet_id": outlet_id, "date": date,
        "total_checks": sales.get::<i64, _>("total_checks"),
        "gross_sales": sales.get::<String, _>("gross_sales"),
        "tax_collected": sales.get::<String, _>("tax_collected"),
        "payment_breakdown": payment_breakdown,
    })))
}

// ── Report export — PDF, Excel, CSV ──
#[utoipa::path(
    get,
    path = "/v1/outlets/{outlet_id}/reports/export/{report_type}",
    responses(
        (status = 200, description = "OK", body = serde_json::Value),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Forbidden"),
    ),
    security(),
    tag = "pos",
)]

pub async fn export_report(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path((outlet_id, report_type)): Path<(uuid::Uuid, String)>,
    axum::extract::Query(params): axum::extract::Query<serde_json::Value>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let a = auth::verify_token(&state.config, &headers).await?;
    crate::policy::require(
        &state.policy,
        &a,
        &headers,
        sentin8::shield::EvalTarget::new("hotel8:pos:report:read"),
    )
    .await?;
    let env_id = super::resolve_org_env(&a);
    let format = params
        .get("format")
        .and_then(|v| v.as_str())
        .unwrap_or("csv");
    let date = params
        .get("date")
        .and_then(|v| v.as_str())
        .unwrap_or("today");
    // Generate export — in production this would create a file and return a download URL
    let export_id = uuid::Uuid::new_v4();
    emit_on_pool(&state.pool, env_id, "hotel8:pos:report:export_requested", serde_json::json!({ "outlet_id": outlet_id, "report_type": &report_type, "format": format, "date": date, "export_id": export_id }),
        &crate::event_emit::Publisher::from_parts(&a.claims.sub, &a.claims.scope)).await.map_err(super::err_emit)?;
    Ok(Json(serde_json::json!({
        "export_id": export_id,
        "outlet_id": outlet_id,
        "report_type": report_type,
        "format": format,
        "date": date,
        "status": "PENDING",
        "download_url": format!("/v1/outlets/{}/reports/export/{}/download", outlet_id, export_id),
        "message": "Export queued — poll the download_url when ready",
    })))
}
