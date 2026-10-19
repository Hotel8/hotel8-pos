// SPDX-FileCopyrightText: Copyright Logically Magic Inc. <hello@logicallymagic.com>
// SPDX-FileCopyrightText: Copyright Hamid Jahanzad <Hamid@Jahanzad.com>
// SPDX-FileCopyrightText: Copyright Contributors to the Hotel8:POS
// SPDX-License-Identifier: Apache-2.0

//! Integration tests for Hotel8:POS - pure-logic paths, no database.
//!
//! Mirrors the bill8-payments convention. Covered (see
//! `hotel8/design/sentin8-hotel8-rust-test-strategy.md` §3.5-3.7):
//! - Check lifecycle transitions (orders.rs hold/recall/void guards).
//! - Void-authorization policy (manager approval > $100.00).
//! - Payment-application outcome (PAID vs PARTIALLY_PAID).
//! - Order (kitchen ticket) lifecycle.
//! - Contract wire format (pos-contracts decimal round-trip).

#![allow(clippy::needless_pass_by_value)]

use hotel8_pos_server::check_state::{CheckState, OrderState, VoidError};
use rust_decimal::Decimal;

fn d(cents: i64) -> Decimal {
    Decimal::new(cents, 2)
}

// ── 1. Check lifecycle ─────────────────────────────────────────────────────

#[test]
fn hold_only_from_open() {
    // orders.rs::hold_check - SQL `AND status = 'OPEN'`.
    assert!(CheckState::Open.can_transition_to(CheckState::Held));
    for s in [
        CheckState::Held,
        CheckState::PartiallyPaid,
        CheckState::Paid,
        CheckState::Voided,
        CheckState::Refunded,
    ] {
        assert!(!s.can_transition_to(CheckState::Held), "hold from {s:?}");
    }
}

#[test]
fn recall_only_from_held() {
    // orders.rs::recall_check - SQL `AND status = 'HELD'`.
    assert!(CheckState::Held.can_transition_to(CheckState::Open));
    for s in [
        CheckState::Open,
        CheckState::PartiallyPaid,
        CheckState::Paid,
        CheckState::Voided,
        CheckState::Refunded,
    ] {
        assert!(!s.can_transition_to(CheckState::Open), "recall from {s:?}");
    }
}

#[test]
fn void_refusals_map_to_handler_errors() {
    assert_eq!(
        CheckState::can_void(CheckState::Voided, d(1_00), false),
        Err(VoidError::AlreadyVoided)
    );
    assert_eq!(
        CheckState::can_void(CheckState::Paid, d(1_00), true),
        Err(VoidError::PaidNeedsRefund)
    );
    assert_eq!(
        CheckState::can_void(CheckState::Open, d(200_00), false),
        Err(VoidError::ManagerApprovalRequired)
    );
}

// ── 2. Payment outcome (payments.rs SQL CASE mirror) ───────────────────────

#[test]
fn payment_outcome_paid_when_total_covered() {
    assert_eq!(
        CheckState::payment_outcome(d(40_70), d(40_70)),
        CheckState::Paid
    );
    assert_eq!(
        CheckState::payment_outcome(d(40_70), d(45_00)),
        CheckState::Paid
    );
}

#[test]
fn payment_outcome_partial_when_short() {
    assert_eq!(
        CheckState::payment_outcome(d(40_70), d(20_35)),
        CheckState::PartiallyPaid
    );
}

#[test]
fn payment_outcome_cent_boundary() {
    assert_eq!(
        CheckState::payment_outcome(d(100_00), d(99_99)),
        CheckState::PartiallyPaid
    );
    assert_eq!(
        CheckState::payment_outcome(d(100_00), d(100_00)),
        CheckState::Paid
    );
}

// ── 3. Order (kitchen) lifecycle ───────────────────────────────────────────

#[test]
fn order_served_is_terminal() {
    let mut cur = OrderState::Sent;
    for next in [OrderState::Preparing, OrderState::Ready, OrderState::Served] {
        assert!(cur.can_transition_to(next));
        cur = next;
    }
    assert!(!OrderState::Served.can_transition_to(OrderState::Cancelled));
    assert!(!OrderState::Cancelled.can_transition_to(OrderState::Preparing));
}

#[test]
fn order_void_all_on_check_void() {
    // orders.rs::void_check cancels all non-served orders; SERVED items are
    // already consumed and the SQL cancels them too - the machine documents
    // which pre-serve states permit cancellation.
    for s in [OrderState::Sent, OrderState::Preparing, OrderState::Ready] {
        assert!(
            s.can_transition_to(OrderState::Cancelled),
            "cancel from {s:?}"
        );
    }
}

// ── 4. Contract sanity (pos-contracts decimal handling) ────────────────────

#[test]
fn decimal_cents_survive_json() {
    let price = Decimal::new(40_70, 2);
    let v = serde_json::to_value(price).unwrap();
    let back: Decimal = serde_json::from_value(v).unwrap();
    assert_eq!(back, price);
}
