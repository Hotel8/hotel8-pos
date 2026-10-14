// SPDX-FileCopyrightText: Copyright Logically Magic Inc. <hello@logicallymagic.com>
// SPDX-FileCopyrightText: Copyright Hamid Jahanzad <Hamid@Jahanzad.com>
// SPDX-FileCopyrightText: Copyright Contributors to the Hotel8:POS
// SPDX-License-Identifier: Apache-2.0

//! Auth token verification for H8POS.
//! Auth token verification for POS.
//!
//! Re-exports the shared types from the sentin8-auth-client SDK and provides
//! thin wrappers matching the original call sites (taking `&POSConfig` + `&HeaderMap`).
//!
//! Step 5 (Shield authorization) is the ONLY gate (DESIGN.md §5). The
//! `require_any_scope` helper is gone — handlers verify the token (steps 1–4)
//! with `verify_token`, then evaluate a `hotel8:pos:<resource>:<action>` Shield
//! action via `crate::policy::require`. `sys:admin`/org/env-admin scopes pass
//! through the mirror's admin permits — platform admins never bypass Shield.

use crate::config::POSConfig;
use axum::http::{HeaderMap, StatusCode};
pub use sentin8::auth::client::AuthContext;

pub async fn verify_token(
    config: &POSConfig,
    headers: &HeaderMap,
) -> Result<AuthContext, (StatusCode, String)> {
    sentin8::auth::client::verify_bearer_token(&config.sentin8_auth_client, headers).await
}
