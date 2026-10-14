// SPDX-FileCopyrightText: Copyright Logically Magic Inc. <hello@logicallymagic.com>
// SPDX-FileCopyrightText: Copyright Hamid Jahanzad <Hamid@Jahanzad.com>
// SPDX-FileCopyrightText: Copyright Contributors to the Hotel8:POS
// SPDX-License-Identifier: Apache-2.0

//! Hotel8:POS Admin UI handlers — RFW definitions.
//! Served from `GET /v1/admin/ui/{page}`, rendered by Sentin8:Dash at runtime.

use crate::handlers::AppState;
use axum::{
    extract::{Path, State},
    http::HeaderMap,
    Json,
};
use sentin8::rfw::page::*;
use sentin8::rfw::response::AdminUiResponse;
use sentin8::rfw::widget::*;

pub async fn get_admin_ui(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(page): Path<String>,
) -> Result<Json<serde_json::Value>, (axum::http::StatusCode, String)> {
    let a = crate::auth::verify_token(&state.config, &headers).await?;
    crate::policy::require(
        &state.policy,
        &a,
        &headers,
        sentin8::shield::EvalTarget::new("hotel8:pos:admin"),
    )
    .await?;
    let def = match page.as_str() {
        "overview" => AdminPage::new("POS Overview")
            .widget(card("Summary", "Orders, payments, reports."))
            .build(),
        "orders" => AdminPage::new("Orders")
            .widget(table(
                vec![
                    col("id", "Order ID"),
                    cols("status", "Status"),
                    cols("total", "Total"),
                    cols("created_at", "Created"),
                ],
                "/orders",
            ))
            .build(),
        "payments" => AdminPage::new("Payments")
            .widget(table(
                vec![
                    col("id", "Payment ID"),
                    cols("method", "Method"),
                    cols("amount", "Amount"),
                ],
                "/payments",
            ))
            .build(),
        "reports" => AdminPage::new("Reports")
            .widget(card("Reports", "Sales, voids, Z-reports."))
            .build(),
        "settings" => AdminPage::new("POS Settings")
            .widget(card("Settings", "Hotel8:POS configuration."))
            .build(),
        // ── Policy Studio — shared page
        // constructors; the guard guarantees `Some` (§3.7 dispatch).
        p if sentin8::rfw::policy_page("hotel8:pos", p).is_some() => {
            sentin8::rfw::policy_page("hotel8:pos", p)
                .expect("policy_page checked in guard")
                .build()
        }
        _ => {
            return Err((
                axum::http::StatusCode::NOT_FOUND,
                format!("Unknown admin UI page: {page}"),
            ))
        }
    };
    Ok(Json(
        serde_json::to_value(AdminUiResponse::page(def)).unwrap(),
    ))
}

#[allow(dead_code)]
fn col(key: &str, label: &str) -> WidgetDataTableColumn {
    WidgetDataTableColumn {
        key: key.into(),
        label: label.into(),
        sortable: false,
        width: None,
        format: None,
    }
}
#[allow(dead_code)]
fn cols(key: &str, label: &str) -> WidgetDataTableColumn {
    WidgetDataTableColumn {
        key: key.into(),
        label: label.into(),
        sortable: true,
        width: None,
        format: None,
    }
}
#[allow(dead_code)]
fn table(columns: Vec<WidgetDataTableColumn>, endpoint: &str) -> RemoteWidget {
    RemoteWidget::DataTable(WidgetDataTable {
        columns,
        data_endpoint: endpoint.into(),
        sortable: true,
        filterable: true,
        page_size: 20,
        empty_message: Some("No data.".into()),
        row_link: None,
        row_actions: vec![],
    })
}
fn card(title: &str, body: &str) -> RemoteWidget {
    RemoteWidget::Card(WidgetCard {
        title: Some(title.into()),
        subtitle: None,
        children: vec![RemoteWidget::Text(WidgetText {
            text: body.into(),
            text_type: WidgetTextType::Body,
            align: TextAlign::Left,
            max_lines: None,
        })],
        actions: vec![],
    })
}
#[allow(dead_code)]
fn link(label: &str, route: &str) -> AdminUiAction {
    AdminUiAction {
        label: label.into(),
        action_type: AdminUiActionType::Link,
        route: Some(route.into()),
        endpoint: None,
        icon: Some("0xe145".into()),
    }
}
