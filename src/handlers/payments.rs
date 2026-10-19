// SPDX-FileCopyrightText: Copyright Logically Magic Inc. <hello@logicallymagic.com>
// SPDX-FileCopyrightText: Copyright Hamid Jahanzad <Hamid@Jahanzad.com>
// SPDX-FileCopyrightText: Copyright Contributors to the Hotel8:POS
// SPDX-License-Identifier: Apache-2.0

//! Payment handlers - post payments to checks.
//!
//! The `PAID` vs `PARTIALLY_PAID` outcome after posting is the pure
//! `check_state::CheckState::payment_outcome` rule, mirrored in the SQL
//! `CASE WHEN total <= paid_sum ...` below (tested DB-free in check_state.rs).
use crate::auth;
use crate::event_emit::emit_tx;
use crate::handlers::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use hotel8::pos::contracts::{
    ApplyDiscountRequest, CardPaymentProcessed, CardPaymentRequest, DiscountApplied,
    GuestRoomLookup, PackageRedeemed, PostPaymentRequest, PostedPayment, RedeemPackageRequest,
    RoomChargeRequest, RoomChargeResult, SplitPaymentLeg, SplitPaymentRequest, SplitPaymentResult,
};
use sqlx::Row;

/// `POST /v1/outlets/{outlet_id}/checks/{check_id}/payments`
#[utoipa::path(
    post,
    path = "/v1/outlets/{outlet_id}/checks/{check_id}/payments",
    request_body = PostPaymentRequest,
    responses(
        (status = 200, description = "Payment posted", body = PostedPayment),
        (status = 403, description = "Missing pos:admin scope"),
    ),
    security(("pos:admin" = [])),
    tag = "pos",
)]
pub async fn post_payment(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path((outlet_id, check_id)): Path<(uuid::Uuid, uuid::Uuid)>,
    Json(req): Json<PostPaymentRequest>,
) -> Result<Json<PostedPayment>, (StatusCode, String)> {
    let a = auth::verify_token(&state.config, &headers).await?;
    crate::policy::require(
        &state.policy,
        &a,
        &headers,
        sentin8::shield::EvalTarget::new("hotel8:pos:payment:write"),
    )
    .await?;
    let env_id = super::resolve_org_env(&a);
    let mut conn = state.pool.begin().await.map_err(super::err_db)?;
    super::set_tenant_context(&mut conn, env_id).await;

    let payment_type = req.payment_type.clone();
    let amount_str = req.amount.clone();
    let tip_str = req.tip_amount.clone();
    let reference = req.reference.clone();
    let room_number = req.room_number.clone();

    let row = sqlx::query("INSERT INTO pos.payments (check_id, payment_type, amount, tip_amount, reference, room_number) VALUES ($1, $2, $3, $4, $5, $6) RETURNING id, payment_type, amount::text as amount, tip_amount::text as tip_amount")
        .bind(check_id)
        .bind(&payment_type)
        .bind(&amount_str)
        .bind(&tip_str)
        .bind(reference.clone())
        .bind(room_number)
        .fetch_one(&mut *conn)
        .await
        .map_err(super::err_db)?;
    let payment_id: uuid::Uuid = row.get("id");
    let amt: String = row.get("amount");

    // Check if fully paid
    let _ = sqlx::query("UPDATE pos.checks SET status = CASE WHEN total <= (SELECT COALESCE(SUM(amount), 0) FROM pos.payments WHERE check_id = $1) THEN 'PAID'::pos.check_status ELSE 'PARTIALLY_PAID'::pos.check_status END, closed_at = CASE WHEN total <= (SELECT COALESCE(SUM(amount), 0) FROM pos.payments WHERE check_id = $1) THEN now() ELSE closed_at END, updated_at = now() WHERE id = $1")
        .bind(check_id)
        .execute(&mut *conn)
        .await;

    emit_tx(&mut *conn, env_id, "hotel8:pos:payment:posted", serde_json::json!({ "check_id": check_id, "payment_id": payment_id, "amount": &amt, "payment_type": payment_type }),
        &crate::event_emit::Publisher::from_parts(&a.claims.sub, &a.claims.scope)).await.map_err(super::err_emit)?;
    conn.commit().await.map_err(super::err_db)?;
    Ok(Json(PostedPayment {
        id: payment_id,
        check_id,
        outlet_id,
        payment_type: row.get::<String, _>("payment_type"),
        amount: amt,
        tip_amount: row.get::<String, _>("tip_amount"),
        reference,
    }))
}

// ── Payments integration - card tokenization, authorization, capture ──
#[utoipa::path(
    post,
    path = "/v1/outlets/{outlet_id}/checks/{check_id}/payments-process",
    request_body = CardPaymentRequest,
    responses(
        (status = 200, description = "Card payment captured (bill8 tokens only)", body = CardPaymentProcessed),
        (status = 400, description = "card_token missing"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Forbidden"),
    ),
    security(),
    tag = "pos",
)]

pub async fn bill8_payments_process_payment(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path((outlet_id, check_id)): Path<(uuid::Uuid, uuid::Uuid)>,
    Json(req): Json<CardPaymentRequest>,
) -> Result<Json<CardPaymentProcessed>, (StatusCode, String)> {
    let a = auth::verify_token(&state.config, &headers).await?;
    crate::policy::require(
        &state.policy,
        &a,
        &headers,
        sentin8::shield::EvalTarget::new("hotel8:pos:payment:write"),
    )
    .await?;
    let env_id = super::resolve_org_env(&a);
    let mut conn = state.pool.begin().await.map_err(super::err_db)?;
    super::set_tenant_context(&mut conn, env_id).await;
    let token = req.card_token.clone();
    let amount_str = req.amount.clone();
    let tip_str = req.tip_amount.clone();
    // Get check total for validation
    let check = sqlx::query(
        "SELECT total::text as total FROM pos.checks WHERE id = $1 AND revenue_center_id = $2",
    )
    .bind(check_id)
    .bind(outlet_id)
    .fetch_one(&mut *conn)
    .await
    .map_err(super::err_db)?;
    let total_str: String = check.get("total");
    // Emit event for Payments to process (in production, call Payments API)
    emit_tx(&mut *conn, env_id, "hotel8:pos:payments:payment_requested", serde_json::json!({ "check_id": check_id, "card_token": token, "amount": amount_str, "tip_amount": tip_str, "check_total": &total_str }),
        &crate::event_emit::Publisher::from_parts(&a.claims.sub, &a.claims.scope)).await.map_err(super::err_emit)?;
    // Record the payment as CREDIT_CARD
    let row = sqlx::query("INSERT INTO pos.payments (check_id, payment_type, amount, tip_amount, reference) VALUES ($1, 'CREDIT_CARD', $2, $3, $4) RETURNING id, amount::text as amount")
        .bind(check_id).bind(&amount_str).bind(&tip_str).bind(&token)
        .fetch_one(&mut *conn).await.map_err(super::err_db)?;
    let payment_id: uuid::Uuid = row.get("id");
    let amt: String = row.get("amount");
    // Update check status
    let _ = sqlx::query("UPDATE pos.checks SET status = CASE WHEN total <= (SELECT COALESCE(SUM(amount), 0) FROM pos.payments WHERE check_id = $1) THEN 'PAID'::pos.check_status ELSE 'PARTIALLY_PAID'::pos.check_status END, closed_at = CASE WHEN total <= (SELECT COALESCE(SUM(amount), 0) FROM pos.payments WHERE check_id = $1) THEN now() ELSE closed_at END, updated_at = now() WHERE id = $1")
        .bind(check_id).execute(&mut *conn).await;
    emit_tx(&mut *conn, env_id, "hotel8:pos:payment:posted", serde_json::json!({ "check_id": check_id, "payment_id": payment_id, "amount": &amt, "payment_type": "CREDIT_CARD", "via": "payments" }),
        &crate::event_emit::Publisher::from_parts(&a.claims.sub, &a.claims.scope)).await.map_err(super::err_emit)?;
    conn.commit().await.map_err(super::err_db)?;
    Ok(Json(CardPaymentProcessed {
        payment_id,
        check_id,
        amount: amt,
        payment_type: "CREDIT_CARD".to_string(),
        card_token: token,
        status: "CAPTURED".to_string(),
    }))
}

// ── Split Payment - multiple payment methods per check ──
#[utoipa::path(
    post,
    path = "/v1/outlets/{outlet_id}/checks/{check_id}/split-payment",
    request_body = SplitPaymentRequest,
    responses(
        (status = 200, description = "Split payments posted", body = SplitPaymentResult),
        (status = 400, description = "payments array missing"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Forbidden"),
    ),
    security(),
    tag = "pos",
)]

pub async fn split_payment(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path((_outlet_id, check_id)): Path<(uuid::Uuid, uuid::Uuid)>,
    Json(req): Json<SplitPaymentRequest>,
) -> Result<Json<SplitPaymentResult>, (StatusCode, String)> {
    let a = auth::verify_token(&state.config, &headers).await?;
    crate::policy::require(
        &state.policy,
        &a,
        &headers,
        sentin8::shield::EvalTarget::new("hotel8:pos:payment:write"),
    )
    .await?;
    let env_id = super::resolve_org_env(&a);
    let mut conn = state.pool.begin().await.map_err(super::err_db)?;
    super::set_tenant_context(&mut conn, env_id).await;
    let payments = req.payments.clone();
    let mut results: Vec<SplitPaymentLeg> = Vec::new();
    for p in payments.iter() {
        let ptype = p.payment_type.clone();
        let amount_str = p.amount.clone();
        let tip_str = p.tip_amount.clone();
        let reference = p.reference.clone();
        let row = sqlx::query("INSERT INTO pos.payments (check_id, payment_type, amount, tip_amount, reference) VALUES ($1, $2, $3, $4, $5) RETURNING id, amount::text as amount")
            .bind(check_id).bind(&ptype).bind(&amount_str).bind(&tip_str).bind(&reference)
            .fetch_one(&mut *conn).await.map_err(super::err_db)?;
        let pid: uuid::Uuid = row.get("id");
        let amt: String = row.get("amount");
        results
            .push(SplitPaymentLeg { payment_id: pid, payment_type: ptype, amount: amt });
    }
    // Update check status
    let _ = sqlx::query("UPDATE pos.checks SET status = CASE WHEN total <= (SELECT COALESCE(SUM(amount), 0) FROM pos.payments WHERE check_id = $1) THEN 'PAID'::pos.check_status ELSE 'PARTIALLY_PAID'::pos.check_status END, closed_at = CASE WHEN total <= (SELECT COALESCE(SUM(amount), 0) FROM pos.payments WHERE check_id = $1) THEN now() ELSE closed_at END, updated_at = now() WHERE id = $1")
        .bind(check_id).execute(&mut *conn).await;
    emit_tx(
        &mut *conn,
        env_id,
        "hotel8:pos:payment:split",
        serde_json::json!({ "check_id": check_id, "payments": &results }),
        &crate::event_emit::Publisher::from_parts(&a.claims.sub, &a.claims.scope),
    )
    .await
    .map_err(super::err_emit)?;
    conn.commit().await.map_err(super::err_db)?;
    Ok(Json(SplitPaymentResult {
        check_id,
        payments: results,
    }))
}

// ── Discount application - percentage, dollar, promo code, employee meal, manager comp ──
#[utoipa::path(
    post,
    path = "/v1/outlets/{outlet_id}/checks/{check_id}/discount",
    request_body = ApplyDiscountRequest,
    responses(
        (status = 200, description = "Discount applied", body = DiscountApplied),
        (status = 400, description = "discount_type missing"),
        (status = 403, description = "Manager approval required for comp"),
    ),
    security(),
    tag = "pos",
)]

pub async fn apply_discount(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path((_outlet_id, check_id)): Path<(uuid::Uuid, uuid::Uuid)>,
    Json(req): Json<ApplyDiscountRequest>,
) -> Result<Json<DiscountApplied>, (StatusCode, String)> {
    let a = auth::verify_token(&state.config, &headers).await?;
    crate::policy::require(
        &state.policy,
        &a,
        &headers,
        sentin8::shield::EvalTarget::new("hotel8:pos:payment:write"),
    )
    .await?;
    let env_id = super::resolve_org_env(&a);
    let mut conn = state.pool.begin().await.map_err(super::err_db)?;
    super::set_tenant_context(&mut conn, env_id).await;
    let discount_type = req.discount_type.clone();
    let value_str = req.value.clone();
    let manager_approved = req.manager_approved;
    // Get current check total
    let check = sqlx::query("SELECT total::text as total, subtotal::text as subtotal FROM pos.checks WHERE id = $1 AND revenue_center_id = $2")
        .bind(check_id).bind(_outlet_id)
        .fetch_one(&mut *conn).await.map_err(super::err_db)?;
    let total_str: String = check.get("total");
    let subtotal_str: String = check.get("subtotal");
    let total: rust_decimal::Decimal = total_str.parse().unwrap_or_default();
    let subtotal: rust_decimal::Decimal = subtotal_str.parse().unwrap_or_default();
    let discount_amount = match discount_type.as_str() {
        "PERCENTAGE" => {
            let pct: rust_decimal::Decimal = value_str.parse().unwrap_or_default();
            subtotal * pct / rust_decimal::Decimal::new(100, 0)
        }
        "DOLLAR" => value_str.as_str().parse().unwrap_or_default(),
        "EMPLOYEE_MEAL" => subtotal, // 100% comp
        "MANAGER_COMP" => {
            if !manager_approved {
                return Err((
                    StatusCode::FORBIDDEN,
                    "Manager approval required for comp".to_string(),
                ));
            }
            value_str.as_str().parse().unwrap_or_default()
        }
        _ => rust_decimal::Decimal::ZERO,
    };
    let new_total = (total - discount_amount).max(rust_decimal::Decimal::ZERO);
    // Store discount as a COMP payment with reference
    let row = sqlx::query("INSERT INTO pos.payments (check_id, payment_type, amount, reference) VALUES ($1, 'COMP', $2, $3) RETURNING id, amount::text as amount")
        .bind(check_id).bind(discount_amount.to_string()).bind(format!("DISCOUNT:{}:{}", discount_type, value_str))
        .fetch_one(&mut *conn).await.map_err(super::err_db)?;
    let payment_id: uuid::Uuid = row.get("id");
    // Update check total
    sqlx::query(
        "UPDATE pos.checks SET total = GREATEST(total - $2, 0), updated_at = now() WHERE id = $1",
    )
    .bind(check_id)
    .bind(discount_amount.to_string())
    .execute(&mut *conn)
    .await
    .map_err(super::err_db)?;
    emit_tx(&mut *conn, env_id, "hotel8:pos:discount:applied", serde_json::json!({ "check_id": check_id, "discount_type": discount_type, "discount_amount": discount_amount.to_string(), "new_total": new_total.to_string(), "manager_approved": manager_approved }),
        &crate::event_emit::Publisher::from_parts(&a.claims.sub, &a.claims.scope)).await.map_err(super::err_emit)?;
    conn.commit().await.map_err(super::err_db)?;
    Ok(Json(DiscountApplied {
        payment_id,
        check_id,
        discount_type: discount_type.clone(),
        discount_amount: discount_amount.to_string(),
        new_total: new_total.to_string(),
    }))
}

// ── Room Charge - POST /v1/payments/roomCharge → PMS folio posting ──
#[utoipa::path(
    post,
    path = "/v1/payments/room-charge",
    request_body = RoomChargeRequest,
    responses(
        (status = 200, description = "Posted to folio (PMS event emitted)", body = RoomChargeResult),
        (status = 400, description = "check_id/room_number missing"),
        (status = 404, description = "Check not open"),
    ),
    security(),
    tag = "pos",
)]

pub async fn room_charge(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(req): Json<RoomChargeRequest>,
) -> Result<Json<RoomChargeResult>, (StatusCode, String)> {
    let a = auth::verify_token(&state.config, &headers).await?;
    crate::policy::require(
        &state.policy,
        &a,
        &headers,
        sentin8::shield::EvalTarget::new("hotel8:pos:payment:write"),
    )
    .await?;
    let env_id = super::resolve_org_env(&a);
    let mut conn = state.pool.begin().await.map_err(super::err_db)?;
    super::set_tenant_context(&mut conn, env_id).await;
    let check_id = req.check_id;
    let room_number = req.room_number.clone();
    let reservation_id = req.reservation_id;
    // Get check total
    let check = sqlx::query("SELECT total::text as total, revenue_center_id FROM pos.checks WHERE id = $1 AND status NOT IN ('PAID', 'VOIDED')")
        .bind(check_id)
        .fetch_one(&mut *conn).await.map_err(super::err_db)?;
    let total_str: String = check.get("total");
    let outlet_id: uuid::Uuid = check.get("revenue_center_id");
    // Post as ROOM_CHARGE payment
    let row = sqlx::query("INSERT INTO pos.payments (check_id, payment_type, amount, room_number) VALUES ($1, 'ROOM_CHARGE', $2, $3) RETURNING id, amount::text as amount")
        .bind(check_id).bind(&total_str).bind(&room_number)
        .fetch_one(&mut *conn).await.map_err(super::err_db)?;
    let payment_id: uuid::Uuid = row.get("id");
    let amt: String = row.get("amount");
    // Mark check as paid
    sqlx::query("UPDATE pos.checks SET status = 'PAID', closed_at = now(), updated_at = now() WHERE id = $1")
        .bind(check_id).execute(&mut *conn).await.map_err(super::err_db)?;
    // Emit event for PMS to post to folio
    emit_tx(&mut *conn, env_id, "hotel8:pos:payment:room_charge", serde_json::json!({ "check_id": check_id, "payment_id": payment_id, "room_number": room_number, "reservation_id": reservation_id, "amount": &amt, "outlet_id": outlet_id }),
        &crate::event_emit::Publisher::from_parts(&a.claims.sub, &a.claims.scope)).await.map_err(super::err_emit)?;
    conn.commit().await.map_err(super::err_db)?;
    Ok(Json(RoomChargeResult {
        payment_id,
        check_id,
        room_number,
        amount: amt,
        payment_type: "ROOM_CHARGE".to_string(),
        status: "POSTED_TO_FOLIO".to_string(),
    }))
}

// ── PMS guest profile lookup for room charges ──
#[utoipa::path(
    get,
    path = "/v1/guests/lookup-by-room",
    responses(
        (status = 200, description = "In-house guest for the room (PMS projection fields)", body = GuestRoomLookup),
        (status = 400, description = "room_number missing"),
        (status = 404, description = "No in-house guest found for this room"),
    ),
    security(),
    tag = "pos-guest",
)]

pub async fn lookup_guest_by_room(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    axum::extract::Query(params): axum::extract::Query<serde_json::Value>,
) -> Result<Json<GuestRoomLookup>, (StatusCode, String)> {
    let a = auth::verify_token(&state.config, &headers).await?;
    crate::policy::require(
        &state.policy,
        &a,
        &headers,
        sentin8::shield::EvalTarget::new("hotel8:pos:payment:write"),
    )
    .await?;
    let env_id = super::resolve_org_env(&a);
    let mut conn = state.pool.begin().await.map_err(super::err_db)?;
    super::set_tenant_context(&mut conn, env_id).await;
    let room_number = params
        .get("room_number")
        .and_then(|v| v.as_str())
        .ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                "room_number is required".to_string(),
            )
        })?;
    // B1: in-house reservation lookup via the PMS HTTP API. The API's
    // reservation projection carries assigned_room_number - scan the
    // property's recent CHECKED_IN reservations for a room match.
    // (Room→property scoping: pos uses the env's property set; the property
    // id arrives from the outlet config in a follow-up story - for now the
    // PMS search runs without a property filter param beyond the endpoint's
    // required property_id, so resolve it from the outlet's property.)
    let property_id = super::property_id_for_env_pool(&state.pool, env_id)
        .await
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                "No POS outlet configured for this environment".to_string(),
            )
        })?;
    let search = state
        .pms_client
        .search_reservations(property_id, &[("status", "CHECKED_IN"), ("limit", "200")])
        .await
        .map_err(|e| (StatusCode::BAD_GATEWAY, format!("PMS search failed: {e}")))?;
    let found = search
        .get("items")
        .and_then(|v| v.as_array())
        .and_then(|arr| {
            arr.iter()
                .find(|r| {
                    r.get("assigned_room_number").and_then(|v| v.as_str()) == Some(room_number)
                })
                .cloned()
        })
        .ok_or_else(|| super::err_not_found("No in-house guest found for this room"))?;
    conn.commit().await.map_err(super::err_db)?;
    Ok(Json(GuestRoomLookup {
        reservation_id: found.get("id").cloned().unwrap_or(serde_json::Value::Null),
        guest_name: found.get("guest_name").cloned().unwrap_or(serde_json::Value::Null),
        room_number: found.get("assigned_room_number").cloned().unwrap_or(serde_json::Value::Null),
        guest_profile_id: found.get("guest_profile_id").cloned().unwrap_or(serde_json::Value::Null),
    }))
}

// ── PMS package redemption (e.g., "Breakfast Included" → $0 charge) ──
#[utoipa::path(
    post,
    path = "/v1/outlets/{outlet_id}/checks/{check_id}/redeem-package",
    request_body = RedeemPackageRequest,
    responses(
        (status = 200, description = "Package redeemed (full $0 COMP payment)", body = PackageRedeemed),
        (status = 400, description = "package_code/reservation_id missing"),
        (status = 404, description = "Reservation not found or guest not checked in"),
    ),
    security(),
    tag = "pos",
)]

pub async fn redeem_package(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path((_outlet_id, check_id)): Path<(uuid::Uuid, uuid::Uuid)>,
    Json(req): Json<RedeemPackageRequest>,
) -> Result<Json<PackageRedeemed>, (StatusCode, String)> {
    let a = auth::verify_token(&state.config, &headers).await?;
    crate::policy::require(
        &state.policy,
        &a,
        &headers,
        sentin8::shield::EvalTarget::new("hotel8:pos:payment:write"),
    )
    .await?;
    let env_id = super::resolve_org_env(&a);
    let mut conn = state.pool.begin().await.map_err(super::err_db)?;
    super::set_tenant_context(&mut conn, env_id).await;
    let package_code = req.package_code.clone();
    let reservation_id = req.reservation_id;
    // B1: reservation package verification via the PMS HTTP API.
    let property_id = super::property_id_for_env_pool(&state.pool, env_id)
        .await
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                "No POS outlet configured for this environment".to_string(),
            )
        })?;
    let res = state
        .pms_client
        .get_reservation(property_id, reservation_id)
        .await;
    let res_ok = match res {
        Ok(r) => r.get("status").and_then(|v| v.as_str()) == Some("CHECKED_IN"),
        Err(hotel8::pms::client::H8pmsError::Http(ref m)) if m.contains("404") => false,
        Err(e) => return Err((StatusCode::BAD_GATEWAY, format!("PMS read failed: {e}"))),
    };
    if !res_ok {
        return Err(super::err_not_found(
            "Reservation not found or guest not checked in",
        ));
    }
    // Post a $0 COMP payment with the package code as reference
    let row = sqlx::query("INSERT INTO pos.payments (check_id, payment_type, amount, reference) VALUES ($1, 'COMP', 0, $2) RETURNING id")
        .bind(check_id).bind(format!("PACKAGE:{}", package_code))
        .fetch_one(&mut *conn).await.map_err(super::err_db)?;
    let payment_id: uuid::Uuid = row.get("id");
    // Mark the check as PAID (package covers the full amount)
    sqlx::query("UPDATE pos.checks SET status = 'PAID', closed_at = now(), updated_at = now() WHERE id = $1")
        .bind(check_id).execute(&mut *conn).await.map_err(super::err_db)?;
    emit_tx(&mut *conn, env_id, "hotel8:pos:package:redeemed", serde_json::json!({ "check_id": check_id, "payment_id": payment_id, "package_code": package_code, "reservation_id": reservation_id }),
        &crate::event_emit::Publisher::from_parts(&a.claims.sub, &a.claims.scope)).await.map_err(super::err_emit)?;
    conn.commit().await.map_err(super::err_db)?;
    Ok(Json(PackageRedeemed {
        payment_id,
        check_id,
        package_code,
        redeemed: true,
        amount_charged: "0.00".to_string(),
    }))
}
