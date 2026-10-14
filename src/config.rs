// SPDX-FileCopyrightText: Copyright Logically Magic Inc. <hello@logicallymagic.com>
// SPDX-FileCopyrightText: Copyright Hamid Jahanzad <Hamid@Jahanzad.com>
// SPDX-FileCopyrightText: Copyright Contributors to the Hotel8:POS
// SPDX-License-Identifier: Apache-2.0

//! H8POS configuration loaded from environment variables.
use std::env;
use sentin8::auth::client::{Client as AuthClient, ClientOptions};

#[derive(Debug, Clone)]
pub struct POSConfig {
    pub port: u16,
    pub database_url: String,
    pub sentin8_auth_issuer: String,
    pub sentin8_auth_jwks_url: String,
    pub sentin8_bus_url: String,
    pub stream_family: String,
    pub bill8_payments_url: String,
    pub pms_url: String,
    pub sentin8_auth_client: AuthClient,
}

impl POSConfig {
    pub fn from_env() -> Self {
        let cfg = sentin8::config::Sentin8Config::from_env();
        Self {
            port: env::var("HOTEL8_POS_PORT")
                .expect("HOTEL8_POS_PORT must be set by captain")
                .parse()
                .expect("HOTEL8_POS_PORT must be a number"),
            // DBX (§VDC.3) — the DSN is REQUIRED and must name the vertical
            // database (hotel8): the shell verifies the service identity's
            // expected database and refuses to boot otherwise. There is no
            // platform-database fallback.
            database_url: env::var("HOTEL8_POS_DATABASE_URL")
                .unwrap_or_else(|_| "postgres://hotel8_pos_service@localhost:5432/hotel8".into()),
            sentin8_auth_issuer: env::var("SENTIN8_AUTH_ISSUER")
                .unwrap_or_else(|_| cfg.sentin8_auth_issuer()),
            sentin8_auth_jwks_url: env::var("SENTIN8_AUTH_JWKS_URL")
                .unwrap_or_else(|_| cfg.sentin8_auth_jwks_url()),
            sentin8_bus_url: env::var("SENTIN8_BUS_URL")
                .unwrap_or_else(|_| cfg.sentin8_bus_url()),
            stream_family: env::var("HOTEL8_POS_STREAM_FAMILY")
                .unwrap_or_else(|_| "pos-events".into()),
            bill8_payments_url: env::var("BILL8_PAYMENTS_URL")
                .unwrap_or_else(|_| format!("https://{}", cfg.platform_domain("bill8:payments"))),
            pms_url: env::var("HOTEL8_PMS_URL")
                .unwrap_or_else(|_| format!("https://{}", cfg.platform_domain("hotel8:pms"))),
            sentin8_auth_client: AuthClient::new(ClientOptions {
                base_url: env::var("SENTIN8_AUTH_ISSUER")
                    .unwrap_or_else(|_| cfg.sentin8_auth_issuer()),
                ..Default::default()
            }),
        }
    }
}