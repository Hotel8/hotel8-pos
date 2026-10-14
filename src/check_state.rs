// SPDX-FileCopyrightText: Copyright Logically Magic Inc. <hello@logicallymagic.com>
// SPDX-FileCopyrightText: Copyright Hamid Jahanzad <Hamid@Jahanzad.com>
// SPDX-FileCopyrightText: Copyright Contributors to the Hotel8:POS
// SPDX-License-Identifier: Apache-2.0

//! POS check/order status machines — pure, DB-free transition rules.
//!
//! Extracts the status guards previously hand-written in
//! `handlers/orders.rs` (hold/recall/void string checks) and
//! `handlers/payments.rs` (the paid-status SQL CASE) into testable pure
//! functions. Normative matrices: `hotel8/design/sentin8-hotel8-rust-test-strategy.md`
//! §3.5–3.7, mirroring `sentin8-hotel8-pos-technical-design.md` §4.3–4.4.
//!
//! Every variant's [`CheckState::label`]/[`OrderState::label`] equals the
//! corresponding `CREATE TYPE hotel8.pos_* AS ENUM (...)` value in
//! `services/pos/sql/init.sql`.

use std::fmt;

/// `hotel8.pos_check_status` — check lifecycle (POS design §4.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckState {
    Open,
    Held,
    PartiallyPaid,
    Paid,
    Voided,
    Refunded,
}

impl CheckState {
    /// Exact DB enum label.
    pub fn label(&self) -> &'static str {
        match self {
            CheckState::Open => "OPEN",
            CheckState::Held => "HELD",
            CheckState::PartiallyPaid => "PARTIALLY_PAID",
            CheckState::Paid => "PAID",
            CheckState::Voided => "VOIDED",
            CheckState::Refunded => "REFUNDED",
        }
    }

    /// Parse a DB `status::text` value.
    pub fn from_label(s: &str) -> Self {
        match s {
            "OPEN" => CheckState::Open,
            "HELD" => CheckState::Held,
            "PARTIALLY_PAID" => CheckState::PartiallyPaid,
            "PAID" => CheckState::Paid,
            "VOIDED" => CheckState::Voided,
            "REFUNDED" => CheckState::Refunded,
            _ => CheckState::Open,
        }
    }

    /// Design-doc transition rules (test-strategy §3.5):
    /// - `OPEN ⇄ HELD` (hold / recall)
    /// - any non-terminal state → `PARTIALLY_PAID` / `PAID` / `VOIDED`
    /// - `PAID → REFUNDED` (refund flow; never void)
    /// - `VOIDED` / `REFUNDED` terminal
    ///
    /// Handlers enforce hold/recall in SQL (`WHERE status = 'OPEN'`), so the
    /// matrix itself is only exercised from tests — targeted allow.
    #[allow(dead_code)]
    pub fn can_transition_to(&self, next: Self) -> bool {
        use CheckState::*;
        matches!(
            (self, next),
            (Open, Held)
                | (Held, Open)
                | (Open, PartiallyPaid)
                | (Held, PartiallyPaid)
                | (PartiallyPaid, PartiallyPaid)
                | (Open, Paid)
                | (Held, Paid)
                | (PartiallyPaid, Paid)
                | (Open, Voided)
                | (Held, Voided)
                | (PartiallyPaid, Voided)
                | (Paid, Refunded)
        )
    }

    /// Outcome of applying a payment to a check (the SQL
    /// `CASE WHEN total <= paid_sum THEN 'PAID' ELSE 'PARTIALLY_PAID'` logic).
    ///
    /// Handlers currently apply this rule atomically in SQL
    /// (`payments.rs`), so this pure mirror is only exercised from tests
    /// and the integration suite — hence the targeted `dead_code` allow.
    #[allow(dead_code)]
    pub fn payment_outcome(total: rust_decimal::Decimal, paid_sum: rust_decimal::Decimal) -> Self {
        if total <= paid_sum {
            CheckState::Paid
        } else {
            CheckState::PartiallyPaid
        }
    }

    /// Void-authorization policy (`orders.rs::void_check`):
    /// - `VOIDED` checks cannot be voided again;
    /// - `PAID` checks cannot be voided — refund instead;
    /// - voids over $100.00 require manager approval.
    pub fn can_void(
        status: Self,
        total: rust_decimal::Decimal,
        manager_approved: bool,
    ) -> Result<(), VoidError> {
        if status == CheckState::Voided {
            return Err(VoidError::AlreadyVoided);
        }
        if status == CheckState::Paid {
            return Err(VoidError::PaidNeedsRefund);
        }
        if status == CheckState::Refunded {
            return Err(VoidError::AlreadyVoided);
        }
        // $100.00, cents-exact via Decimal scale 2.
        let threshold = rust_decimal::Decimal::new(10_000, 2);
        if total > threshold && !manager_approved {
            return Err(VoidError::ManagerApprovalRequired);
        }
        Ok(())
    }
}

impl fmt::Display for CheckState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

/// Why a void was refused (maps 1:1 to the handler's HTTP errors).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoidError {
    /// 400 "Check already voided".
    AlreadyVoided,
    /// 400 "Cannot void a paid check — use refund instead".
    PaidNeedsRefund,
    /// 403 "Manager approval required for voids over $100.00".
    ManagerApprovalRequired,
}

/// `hotel8.pos_order_status` — kitchen order lifecycle (POS design §4.4).
///
/// Kitchen-stage transitions (SENT→PREPARING→READY→SERVED) are not yet
/// exposed by a handler; documented + tested for the planned KDS endpoint.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OrderState {
    Sent,
    Preparing,
    Ready,
    Served,
    Cancelled,
}

#[allow(dead_code)]
impl OrderState {
    /// Exact DB enum label.
    pub fn label(&self) -> &'static str {
        match self {
            OrderState::Sent => "SENT",
            OrderState::Preparing => "PREPARING",
            OrderState::Ready => "READY",
            OrderState::Served => "SERVED",
            OrderState::Cancelled => "CANCELLED",
        }
    }

    /// Parse a DB `status::text` value.
    pub fn from_label(s: &str) -> Self {
        match s {
            "SENT" => OrderState::Sent,
            "PREPARING" => OrderState::Preparing,
            "READY" => OrderState::Ready,
            "SERVED" => OrderState::Served,
            "CANCELLED" => OrderState::Cancelled,
            _ => OrderState::Sent,
        }
    }

    /// SENT → PREPARING → READY → SERVED; any pre-serve state may CANCEL
    /// (kitchen void). SERVED/CANCELLED terminal.
    pub fn can_transition_to(&self, next: Self) -> bool {
        use OrderState::*;
        matches!(
            (self, next),
            (Sent, Preparing)
                | (Preparing, Ready)
                | (Ready, Served)
                | (Sent, Cancelled)
                | (Preparing, Cancelled)
                | (Ready, Cancelled)
        )
    }
}

impl fmt::Display for OrderState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_decimal::Decimal;

    fn d(cents: i64) -> Decimal {
        Decimal::new(cents, 2)
    }

    #[test]
    fn check_labels_match_db_enum() {
        assert_eq!(CheckState::Open.label(), "OPEN");
        assert_eq!(CheckState::Held.label(), "HELD");
        assert_eq!(CheckState::PartiallyPaid.label(), "PARTIALLY_PAID");
        assert_eq!(CheckState::Paid.label(), "PAID");
        assert_eq!(CheckState::Voided.label(), "VOIDED");
        assert_eq!(CheckState::Refunded.label(), "REFUNDED");
    }

    #[test]
    fn check_from_label_round_trips() {
        for s in [
            CheckState::Open,
            CheckState::Held,
            CheckState::PartiallyPaid,
            CheckState::Paid,
            CheckState::Voided,
            CheckState::Refunded,
        ] {
            assert_eq!(CheckState::from_label(s.label()), s);
        }
    }

    #[test]
    fn check_hold_recall_cycle() {
        use CheckState::*;
        assert!(Open.can_transition_to(Held));
        assert!(Held.can_transition_to(Open), "recall");
        // Double hold is a no-op state, not a transition.
        assert!(!Held.can_transition_to(Held));
        assert!(!Open.can_transition_to(Open));
    }

    #[test]
    fn check_void_windows() {
        use CheckState::*;
        // Voidable while not paid/settled.
        assert!(Open.can_transition_to(Voided));
        assert!(Held.can_transition_to(Voided));
        assert!(PartiallyPaid.can_transition_to(Voided));
        // Paid must be refunded, never voided.
        assert!(!Paid.can_transition_to(Voided));
        // Terminal states.
        assert!(!Voided.can_transition_to(Voided));
        assert!(!Refunded.can_transition_to(Voided));
        assert!(!Voided.can_transition_to(Open));
    }

    #[test]
    fn check_payment_progression() {
        use CheckState::*;
        assert!(Open.can_transition_to(PartiallyPaid));
        assert!(
            PartiallyPaid.can_transition_to(PartiallyPaid),
            "second partial payment keeps state"
        );
        assert!(PartiallyPaid.can_transition_to(Paid));
        assert!(Open.can_transition_to(Paid), "full payment in one go");
        assert!(Paid.can_transition_to(Refunded));
        assert!(!Refunded.can_transition_to(Refunded));
    }

    #[test]
    fn payment_outcome_matches_sql_case() {
        use CheckState::*;
        // total <= paid_sum → PAID (the SQL comparison).
        assert_eq!(CheckState::payment_outcome(d(5000), d(5000)), Paid);
        assert_eq!(
            CheckState::payment_outcome(d(5000), d(6000)),
            Paid,
            "overpay still closes"
        );
        // total > paid → partial.
        assert_eq!(CheckState::payment_outcome(d(5000), d(3000)), PartiallyPaid);
        assert_eq!(CheckState::payment_outcome(d(5000), d(0)), PartiallyPaid);
        // Cent-exact boundary.
        assert_eq!(
            CheckState::payment_outcome(d(10001), d(10000)),
            PartiallyPaid
        );
        assert_eq!(CheckState::payment_outcome(d(10000), d(10001)), Paid);
    }

    // ── Void-authorization policy (test-strategy §3.7) ──

    #[test]
    fn void_policy_paid_and_voided_refused() {
        assert_eq!(
            CheckState::can_void(CheckState::Voided, d(5000), true),
            Err(VoidError::AlreadyVoided)
        );
        assert_eq!(
            CheckState::can_void(CheckState::Paid, d(5000), true),
            Err(VoidError::PaidNeedsRefund)
        );
    }

    #[test]
    fn void_policy_under_100_needs_no_approval() {
        assert_eq!(
            CheckState::can_void(CheckState::Open, d(9_999), false),
            Ok(())
        );
        // Exactly $100.00 is NOT over the threshold ("voids over $100.00").
        assert_eq!(
            CheckState::can_void(CheckState::Open, d(10_000), false),
            Ok(())
        );
    }

    #[test]
    fn void_policy_over_100_requires_manager() {
        assert_eq!(
            CheckState::can_void(CheckState::Open, d(10_001), false),
            Err(VoidError::ManagerApprovalRequired)
        );
        assert_eq!(
            CheckState::can_void(CheckState::Open, d(10_001), true),
            Ok(())
        );
        // A single cent over is already "over".
        assert_eq!(
            CheckState::can_void(CheckState::Open, d(100_00) + Decimal::new(1, 2), false),
            Err(VoidError::ManagerApprovalRequired)
        );
    }

    #[test]
    fn void_policy_held_and_partial_checks_can_void() {
        assert_eq!(
            CheckState::can_void(CheckState::Held, d(250_00), true),
            Ok(())
        );
        assert_eq!(
            CheckState::can_void(CheckState::PartiallyPaid, d(50_00), false),
            Ok(())
        );
    }

    // ── Order machine (test-strategy §3.6) ──

    #[test]
    fn order_labels_match_db_enum() {
        assert_eq!(OrderState::Sent.label(), "SENT");
        assert_eq!(OrderState::Preparing.label(), "PREPARING");
        assert_eq!(OrderState::Ready.label(), "READY");
        assert_eq!(OrderState::Served.label(), "SERVED");
        assert_eq!(OrderState::Cancelled.label(), "CANCELLED");
    }

    #[test]
    fn order_full_lifecycle() {
        use OrderState::*;
        let mut cur = Sent;
        for next in [Preparing, Ready, Served] {
            assert!(cur.can_transition_to(next), "{cur:?} → {next:?}");
            cur = next;
        }
        // SERVED terminal.
        assert!(!Served.can_transition_to(Served));
        assert!(!Served.can_transition_to(Cancelled));
    }

    #[test]
    fn order_cancellation_only_before_served() {
        use OrderState::*;
        assert!(Sent.can_transition_to(Cancelled));
        assert!(Preparing.can_transition_to(Cancelled));
        assert!(Ready.can_transition_to(Cancelled));
        assert!(!Served.can_transition_to(Cancelled));
        // No skipping stages.
        assert!(!Sent.can_transition_to(Ready));
        assert!(!Sent.can_transition_to(Served));
        assert!(!Cancelled.can_transition_to(Preparing));
    }
}
