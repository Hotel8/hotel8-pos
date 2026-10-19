# Hotel8:POS - Business Requirements & Functionality

> Audience: product owners, business analysts, and anyone evaluating what
> Hotel8:POS **does for the business**. For architecture read `README.md`;
> for wire-level detail read `DESIGN.md`.

## 1. Business Purpose

- Run food-and-beverage service at the table, bar, or outlet: take orders,
  route them to stations, settle checks, and close the outlet's day.
- Connect restaurant revenue to the guest stay (room charges) and to the
  loyalty program (reward redemption) without manual reconciliation.

## 2. Business Scope (What the Service Provides)

| Capability | Description |
|---|---|
| Outlet & area management | Outlets, service areas, table inventory with statuses, combine/split/transfer |
| Menu management | Categories, items, modifier groups/modifiers, tax groups |
| Check lifecycle | Open/hold/recall/split/merge/transfer/void with manager approvals |
| Ordering | Per-seat course-aware ordering and kitchen routing |
| Payments | Cash/card/room-charge/mobile tendering, splitting, payment requests; refund flow after settlement |
| Printer routing | Printer groups mapping items to stations |
| Fiscal configuration | Tax groups, revenue centers |
| Reporting & EOD | Outlet-level reports, end-of-day report generation and export |
| Offline mode | Terminal-disconnected operation with reconciliation on sync |
| Loyalty coupling | Redemption of loyalty rewards at the table |
| Admin console | RFW pages incl. shared Shield policy admin |

**What the service does NOT own (business boundary)**

| Concern | Owning service |
|---|---|
| Room booking, rates, guest folios | hotel8:pms |
| Actual money movement | Bill8:Payments + Bill8:Ledger |
| Reward balances | hotel8:loyalty |
| Rate distribution | hotel8:distrib / hotel8:meta |
| Email receipts transport | Sentin8:Post |
| Identity | Sentin8:Auth |

## 3. Business Capabilities

| Area | Business rule |
|---|---|
| Checks | A check starts when guests are seated and ends settled or voided; a paid check cannot be voided - only refunded through its flow. |
| Holds | A held check keeps the table while deferring kitchen fire; recall resumes normal service. |
| Splits & merges | Checks split by seat/item; merging requires both checks in a mergeable state. |
| Room charging | A room charge appends the amount to the guest's folio through PMS - the terminal never touches folio tables. |
| Kitchen | Orders route to the configured printer/station group; the kitchen state progresses independently of the server view. |
| Modifiers | Modifier groups constrain what can be combined on an item line (choice/range enforcement). |
| Taxation | Tax groups attach to items; totals are computed per item's configuration. |
| Manager authority | Override/void/discount authority is an attribute of the staff member (policy), not a hidden flag. |
| Offline | Terminals continue selling and reconcile on reconnect; conflicts resolve by event idempotency. |
| Loyalty | A member's reward redeems at the table and reduces the check balance. |

## 4. Business Events (Outbound)

| Event | Fires when |
|---|---|
| `hotel8:pos:check:created / held / recalled / split / transferred / voided` | Check lifecycle movements |
| `hotel8:pos:order:added / voided / seat_assigned` | Order lines change |
| `hotel8:pos:kitchen:order_routed` | Kitchen/station routing |
| `hotel8:pos:payment:posted / split / room_charge` | Settlement activity |
| `hotel8:pos:payments:payment_requested` | Payment request raised to the payer |
| `hotel8:pos:table:created/updated/deleted/status_changed`, `tables:combined/separated`, `table:prep` | Table operations |
| `hotel8:pos:menu_item:created`, `modifier:*` | Menu configuration changes |
| `hotel8:pos:discount:applied`, `manager:approved` | Discount/override actions |
| `hotel8:pos:package:redeemed` | Loyalty redemption at the table |
| `hotel8:pos:report:eod_generated`, `report:export_requested` | Business-day close and exports |
| `hotel8:pos:offline:synced` | Offline terminal reconciliation |

**Inbound (consumed):** `hotel8:pms:reservation:checked_in` - prepares
tables/areas for arriving guests.

## 5. Business Rules & Invariants

1. A check's total always equals its items (with modifiers and taxes) minus
   discounts, plus tips where applied.
2. A void after payment is impossible - the refund flow is the only path.
3. Room charges appear on the guest folio exactly once per settled amount.
4. Held checks are excluded from the payable flow until recalled.
5. Every manager-approval action carries the approving identity.
6. Outlet end-of-day is reproducible: the report is generated from recorded
   events, never from ad-hoc aggregation.
7. Offline sales reconcile without loss - every offline mutation surfaces in
   the sync event stream once.

## 6. Target-User Roles & Use Cases

| Persona | Use |
|---|---|
| Server/waiter | Open tables, take orders, fire to kitchen, print checks |
| Bartender | Quick-serve checks, tab handling |
| Cashier | Payments, splits, voids with approvals |
| Kitchen/station | Receive routed orders, mark progress |
| Outlet manager | Discounts/overrides, EOD, reports, menu changes |
| Revenue/finance | Revenue center reports, exports |
| Guest (indirect) | Room charge to folio; reward redemption |

## 7. KPIs / Success Criteria

- Order-to-kitchen latency well under the service-time budget.
- Zero duplicate payment instructions (idempotency holds under retries).
- Zero unreconciled offline checks after sync.
- Table-turn and check-length metrics visible in the admin reports.
- 100% of voids/refunds carry an approval trail.

## 8. Dependencies & Reliance on Other Services

| Reliance | For |
|---|---|
| hotel8:pms | Room-charge folio posting; reservation context |
| Bill8:Payments / Bill8:Ledger | Non-room settlements |
| hotel8:loyalty | Reward redemption validation |
| Sentin8:Auth + Shield | Identity and authorization |
| Sentin8:Bus | `pos` stream delivery + PMS events intake |
| Sentin8:Sys | Audit ingestion |
| Sentin8:Storage | Report exports |

## 9. Acceptance Criteria

1. A full meal cycle (seat → order → fire → pay incl. split/room-charge)
   completes without server-side manual steps.
2. A paid check void is refused and routed to refund.
3. An arriving guest's table is prepared automatically from the PMS check-in.
4. A loyalty reward redeems at the table and reduces the folio/check.
5. An offline terminal's orders appear (exactly once each) after
   reconnection.
6. The end-of-day report reflects all recorded checks and payments.

## 10. Open Business Questions

- Table-side card pre-authorization (tabs).
- Kitchen display batching strategies.
- Tip pooling/distribution policy.

---

**Design attribution:** hotel8:pos and its underlying concepts were designed by Hamid Jahanzad <Hamid@Jahanzad.com>.
