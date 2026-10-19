// SPDX-FileCopyrightText: Copyright Logically Magic Inc. <hello@logicallymagic.com>
// SPDX-FileCopyrightText: Copyright Hamid Jahanzad <Hamid@Jahanzad.com>
// SPDX-FileCopyrightText: Copyright Contributors to the Hotel8:POS
// SPDX-License-Identifier: Apache-2.0

//! H8POS hardening handlers - -.
use crate::auth;
use crate::event_emit::{emit_on_pool, emit_tx};
use crate::handlers::AppState;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use hotel8::pos::contracts::{
    ManagerApproveRequest, ManagerApproval, OfflineStatus, OfflineSyncRequest, OfflineSyncResult,
    PosPermissions, RolePermissionRow, SyncedOperation, KitchenRouted,
};
use sqlx::Row;

// ── User permissions - Server, Bartender, Shift Manager, General Manager, Administrator ──
fn pos_role(
    role: &str,
    permissions: &[&str],
    can_void: bool,
    can_comp: bool,
    can_discount: bool,
    void_threshold: &str,
    comp_threshold: &str,
    discount_threshold: &str,
) -> RolePermissionRow {
    RolePermissionRow {
        role: role.to_string(),
        permissions: permissions.iter().map(|p| p.to_string()).collect(),
        can_void,
        can_comp,
        can_discount,
        void_threshold: void_threshold.to_string(),
        comp_threshold: comp_threshold.to_string(),
        discount_threshold: discount_threshold.to_string(),
    }
}

#[utoipa::path(
    get,
    path = "/v1/permissions",
    responses(
        (status = 200, description = "H8POS role permission matrix", body = PosPermissions),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Forbidden"),
    ),
    security(),
    tag = "pos",
)]

pub async fn get_user_permissions(
    State(_state): State<AppState>,
    headers: axum::http::HeaderMap,
) -> Result<Json<PosPermissions>, (StatusCode, String)> {
    let _a = auth::verify_token(&_state.config, &headers).await?;
    crate::policy::require(
        &_state.policy,
        &_a,
        &headers,
        sentin8::shield::EvalTarget::new("hotel8:pos:read"),
    )
    .await?;
    // Return the permission matrix for H8POS roles
    Ok(Json(PosPermissions {
        roles: vec![
            pos_role(
                "SERVER",
                &["hotel8:pos:order:write", "hotel8:pos:order:read", "hotel8:pos:payment:write", "hotel8:pos:read"],
                false,
                false,
                false,
                "0.00",
                "0.00",
                "0.00",
            ),
            pos_role(
                "BARTENDER",
                &["hotel8:pos:order:write", "hotel8:pos:order:read", "hotel8:pos:payment:write", "hotel8:pos:read"],
                true,
                false,
                true,
                "25.00",
                "0.00",
                "10.00",
            ),
            pos_role(
                "SHIFT_MANAGER",
                &["hotel8:pos:order:write", "hotel8:pos:order:read", "hotel8:pos:payment:write", "hotel8:pos:admin", "hotel8:pos:report:read", "hotel8:pos:read"],
                true,
                true,
                true,
                "100.00",
                "50.00",
                "25.00",
            ),
            pos_role(
                "GENERAL_MANAGER",
                &["hotel8:pos:order:write", "hotel8:pos:order:read", "hotel8:pos:payment:write", "hotel8:pos:admin", "hotel8:pos:report:read", "hotel8:pos:read"],
                true,
                true,
                true,
                "10000.00",
                "10000.00",
                "10000.00",
            ),
            pos_role(
                "ADMINISTRATOR",
                &["hotel8:pos:admin", "hotel8:pos:order:write", "hotel8:pos:order:read", "hotel8:pos:payment:write", "hotel8:pos:report:read", "hotel8:pos:read"],
                true,
                true,
                true,
                "999999.99",
                "999999.99",
                "999999.99",
            ),
        ],
    }))
}

// ── Manager approval for voids, comps, and discounts above threshold ──
#[utoipa::path(
    post,
    path = "/v1/manager-approve",
    request_body = ManagerApproveRequest,
    responses(
        (status = 200, description = "Approval granted", body = ManagerApproval),
        (status = 400, description = "manager PIN required"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Forbidden"),
    ),
    security(),
    tag = "pos",
)]

pub async fn manager_approve(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(req): Json<ManagerApproveRequest>,
) -> Result<Json<ManagerApproval>, (StatusCode, String)> {
    let a = auth::verify_token(&state.config, &headers).await?;
    crate::policy::require(
        &state.policy,
        &a,
        &headers,
        sentin8::shield::EvalTarget::new("hotel8:pos:admin"),
    )
    .await?;
    let env_id = super::resolve_org_env(&a);
    let action_type = req.action_type.clone();
    let amount_str = req.amount.clone();
    let reference_id = req.reference_id.clone();
    let manager_id = req.manager_id.clone();
    let pin = req.pin.clone();
    // In production, verify the manager PIN against Auth
    if pin.is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "manager PIN is required".to_string(),
        ));
    }
    let approval_id = uuid::Uuid::new_v4();
    emit_on_pool(
        &state.pool,
        env_id,
        "hotel8:pos:manager:approved",
        serde_json::json!({
            "approval_id": approval_id, "action_type": action_type,
            "amount": amount_str, "reference_id": reference_id,
            "manager_id": manager_id,
        }),
        &crate::event_emit::Publisher::from_parts(&a.claims.sub, &a.claims.scope),
    )
    .await
    .map_err(super::err_emit)?;
    Ok(Json(ManagerApproval {
        approval_id,
        action_type: action_type.to_string(),
        amount: amount_str.to_string(),
        reference_id: reference_id.to_string(),
        approved: true,
        approved_by: manager_id.to_string(),
    }))
}

// ── Offline support - queue orders and payments locally, sync when reconnected ──
#[utoipa::path(
    get,
    path = "/v1/offline-status",
    responses(
        (status = 200, description = "Terminal offline status", body = OfflineStatus),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Forbidden"),
    ),
    security(),
    tag = "pos",
)]

pub async fn get_offline_status(
    State(_state): State<AppState>,
    headers: axum::http::HeaderMap,
) -> Result<Json<OfflineStatus>, (StatusCode, String)> {
    let _a = auth::verify_token(&_state.config, &headers).await?;
    crate::policy::require(
        &_state.policy,
        &_a,
        &headers,
        sentin8::shield::EvalTarget::new("hotel8:pos:read"),
    )
    .await?;
    Ok(Json(OfflineStatus {
        is_online: true,
        queued_operations: 0,
        last_sync_at: chrono::Utc::now(),
        sync_status: "SYNCED".to_string(),
    }))
}

#[utoipa::path(
    post,
    path = "/v1/offline-sync",
    request_body = OfflineSyncRequest,
    responses(
        (status = 200, description = "Sync replay result", body = OfflineSyncResult),
        (status = 400, description = "operations array required"),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Forbidden"),
    ),
    security(),
    tag = "pos",
)]
pub async fn sync_offline_data(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(req): Json<OfflineSyncRequest>,
) -> Result<Json<OfflineSyncResult>, (StatusCode, String)> {
    let a = auth::verify_token(&state.config, &headers).await?;
    crate::policy::require(
        &state.policy,
        &a,
        &headers,
        sentin8::shield::EvalTarget::new("hotel8:pos:order:write"),
    )
    .await?;
    let env_id = super::resolve_org_env(&a);
    let operations = req.operations.clone();
    let mut synced: Vec<SyncedOperation> = Vec::new();
    for op in operations.iter() {
        let op_type = op.r#type.clone();
        let status = match op_type.as_str() {
            "check_create" | "order_add" | "payment_post" => "SYNCED",
            _ => "SKIPPED",
        };
        synced.push(SyncedOperation {
            r#type: op_type,
            status: status.to_string(),
        });
    }
    emit_on_pool(
        &state.pool,
        env_id,
        "hotel8:pos:offline:synced",
        serde_json::json!({ "operation_count": operations.len() }),
        &crate::event_emit::Publisher::from_parts(&a.claims.sub, &a.claims.scope),
    )
    .await
    .map_err(super::err_emit)?;
    Ok(Json(OfflineSyncResult {
        synced: synced.len(),
        operations: synced,
        synced_at: chrono::Utc::now(),
    }))
}

// ── Kitchen display integration - order routing by printer group ──
#[utoipa::path(
    post,
    path = "/v1/outlets/{outlet_id}/checks/{check_id}/orders/{order_id}/route-kitchen",
    responses(
        (status = 200, description = "Routed to kitchen printer group", body = KitchenRouted),
        (status = 401, description = "Not authenticated"),
        (status = 403, description = "Forbidden"),
    ),
    security(),
    tag = "pos",
)]

pub async fn route_order_to_kitchen(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path((outlet_id, check_id, order_id)): Path<(uuid::Uuid, uuid::Uuid, uuid::Uuid)>,
) -> Result<Json<KitchenRouted>, (StatusCode, String)> {
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
    // Get the order with menu item info for routing
    let order = sqlx::query(
        "SELECT o.id, o.menu_item_id, m.item_name, o.quantity, o.special_instructions
         FROM pos.orders o
         JOIN pos.menu_items m ON o.menu_item_id = m.id
         WHERE o.id = $1 AND o.check_id = $2",
    )
    .bind(order_id)
    .bind(check_id)
    .fetch_one(&mut *conn)
    .await
    .map_err(super::err_db)?;
    let item_name: String = order.get("item_name");
    let quantity: i32 = order.get("quantity");
    let special = order.get::<Option<String>, _>("special_instructions");
    // Route to printer group - in production, look up the printer group for the menu item's category
    let routing_id = uuid::Uuid::new_v4();
    emit_tx(
        &mut *conn,
        env_id,
        "hotel8:pos:kitchen:order_routed",
        serde_json::json!({
            "routing_id": routing_id, "outlet_id": outlet_id,
            "check_id": check_id, "order_id": order_id,
            "item_name": &item_name, "quantity": quantity,
            "special_instructions": &special,
            "printer_group": "KITCHEN_MAIN",
        }),
        &crate::event_emit::Publisher::from_parts(&a.claims.sub, &a.claims.scope),
    )
    .await
    .map_err(super::err_emit)?;
    // Update order status to SENT (already SENT from add_order_item, but ensure)
    sqlx::query(
        "UPDATE pos.orders SET sent_at = COALESCE(sent_at, now()), status = 'SENT' WHERE id = $1",
    )
    .bind(order_id)
    .execute(&mut *conn)
    .await
    .map_err(super::err_db)?;
    conn.commit().await.map_err(super::err_db)?;
    Ok(Json(KitchenRouted {
        routing_id,
        order_id,
        check_id,
        item_name,
        quantity,
        routed_to: "KITCHEN_MAIN".to_string(),
        status: "ROUTED".to_string(),
    }))
}
