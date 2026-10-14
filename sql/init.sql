-- SPDX-FileCopyrightText: Copyright Logically Magic Inc. <hello@logicallymagic.com>
-- SPDX-FileCopyrightText: Copyright Hamid Jahanzad <Hamid@Jahanzad.com>
-- SPDX-FileCopyrightText: Copyright Contributors to the Hotel8:POS
-- SPDX-License-Identifier: Apache-2.0

-- H8POS — Point of Sale tables for the hospitality F&B industry.
--
-- Creates tables: revenue_centers, service_areas, tables, menu_categories,
-- menu_items, modifier_groups, modifiers, checks, orders, payments,
-- tax_groups, printer_groups.
--
-- All tables use RLS via sentin8_set_tenant_context().

-- Custom enum types
CREATE SCHEMA IF NOT EXISTS pos;
DO $$ BEGIN
    CREATE TYPE pos.table_status AS ENUM ('AVAILABLE', 'OCCUPIED', 'RESERVED', 'DIRTY', 'OUT_OF_SERVICE');
EXCEPTION WHEN duplicate_object THEN NULL; END $$;

DO $$ BEGIN
    CREATE TYPE pos.check_status AS ENUM ('OPEN', 'HELD', 'PARTIALLY_PAID', 'PAID', 'VOIDED', 'REFUNDED');
EXCEPTION WHEN duplicate_object THEN NULL; END $$;

DO $$ BEGIN
    CREATE TYPE pos.order_status AS ENUM ('SENT', 'PREPARING', 'READY', 'SERVED', 'CANCELLED');
EXCEPTION WHEN duplicate_object THEN NULL; END $$;

DO $$ BEGIN
    CREATE TYPE pos.payment_type AS ENUM ('CASH', 'CREDIT_CARD', 'DEBIT_CARD', 'ROOM_CHARGE', 'GIFT_CARD', 'MOBILE_WALLET', 'COMP');
EXCEPTION WHEN duplicate_object THEN NULL; END $$;

-- ═══════════════════════════════════════════════════════════════════════════
-- Revenue Centers
-- ═══════════════════════════════════════════════════════════════════════════

CREATE TABLE IF NOT EXISTS pos.revenue_centers (
    id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    property_id   UUID NOT NULL,
    env_id        UUID NOT NULL,
    outlet_code   TEXT NOT NULL,
    outlet_name   TEXT NOT NULL,
    outlet_type   TEXT NOT NULL DEFAULT 'RESTAURANT',
    currency_code TEXT NOT NULL DEFAULT 'USD',
    is_active     BOOLEAN NOT NULL DEFAULT true,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_pos_rc_property ON pos.revenue_centers (property_id);
CREATE INDEX IF NOT EXISTS idx_pos_rc_env ON pos.revenue_centers (env_id);
CREATE UNIQUE INDEX IF NOT EXISTS idx_pos_rc_code ON pos.revenue_centers (property_id, outlet_code);

-- ═══════════════════════════════════════════════════════════════════════════
-- Service Areas
-- ═══════════════════════════════════════════════════════════════════════════

CREATE TABLE IF NOT EXISTS pos.service_areas (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    revenue_center_id UUID NOT NULL REFERENCES pos.revenue_centers(id),
    area_code    TEXT NOT NULL,
    area_name    TEXT NOT NULL,
    is_active    BOOLEAN NOT NULL DEFAULT true,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_pos_sa_rc ON pos.service_areas (revenue_center_id);

-- ═══════════════════════════════════════════════════════════════════════════
-- Tables
-- ═══════════════════════════════════════════════════════════════════════════

CREATE TABLE IF NOT EXISTS pos.tables (
    id                UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    service_area_id   UUID NOT NULL REFERENCES pos.service_areas(id),
    table_number      TEXT NOT NULL,
    table_name        TEXT,
    capacity          INTEGER NOT NULL DEFAULT 4,
    status            pos.table_status NOT NULL DEFAULT 'AVAILABLE',
    current_check_id  UUID,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_pos_tables_sa ON pos.tables (service_area_id);
CREATE UNIQUE INDEX IF NOT EXISTS idx_pos_tables_number ON pos.tables (service_area_id, table_number);

-- ═══════════════════════════════════════════════════════════════════════════
-- Menu Categories
-- ═══════════════════════════════════════════════════════════════════════════

CREATE TABLE IF NOT EXISTS pos.menu_categories (
    id                UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    revenue_center_id UUID NOT NULL REFERENCES pos.revenue_centers(id),
    category_code     TEXT NOT NULL,
    category_name     TEXT NOT NULL,
    sort_order        INTEGER NOT NULL DEFAULT 0,
    is_active         BOOLEAN NOT NULL DEFAULT true,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_pos_mc_rc ON pos.menu_categories (revenue_center_id);

-- ═══════════════════════════════════════════════════════════════════════════
-- Menu Items
-- ═══════════════════════════════════════════════════════════════════════════

CREATE TABLE IF NOT EXISTS pos.menu_items (
    id                UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    revenue_center_id UUID NOT NULL REFERENCES pos.revenue_centers(id),
    category_id       UUID REFERENCES pos.menu_categories(id),
    item_code         TEXT NOT NULL,
    item_name         TEXT NOT NULL,
    description       TEXT,
    price             NUMERIC(12,2) NOT NULL DEFAULT 0,
    tax_rate          NUMERIC(5,4) NOT NULL DEFAULT 0,
    is_active         BOOLEAN NOT NULL DEFAULT true,
    sort_order        INTEGER NOT NULL DEFAULT 0,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_pos_mi_rc ON pos.menu_items (revenue_center_id);
CREATE INDEX IF NOT EXISTS idx_pos_mi_category ON pos.menu_items (category_id);
CREATE UNIQUE INDEX IF NOT EXISTS idx_pos_mi_code ON pos.menu_items (revenue_center_id, item_code);

-- ═══════════════════════════════════════════════════════════════════════════
-- Modifier Groups + Modifiers
-- ═══════════════════════════════════════════════════════════════════════════

CREATE TABLE IF NOT EXISTS pos.modifier_groups (
    id                UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    menu_item_id      UUID NOT NULL REFERENCES pos.menu_items(id),
    group_name        TEXT NOT NULL,
    is_required       BOOLEAN NOT NULL DEFAULT false,
    min_selections   INTEGER NOT NULL DEFAULT 0,
    max_selections   INTEGER NOT NULL DEFAULT 1,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_pos_mg_item ON pos.modifier_groups (menu_item_id);

CREATE TABLE IF NOT EXISTS pos.modifiers (
    id                UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    modifier_group_id UUID NOT NULL REFERENCES pos.modifier_groups(id),
    modifier_name     TEXT NOT NULL,
    price_adjustment  NUMERIC(12,2) NOT NULL DEFAULT 0,
    is_active         BOOLEAN NOT NULL DEFAULT true,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_pos_mod_group ON pos.modifiers (modifier_group_id);

-- ═══════════════════════════════════════════════════════════════════════════
-- Checks (bills)
-- ═══════════════════════════════════════════════════════════════════════════

CREATE TABLE IF NOT EXISTS pos.checks (
    id                UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    revenue_center_id UUID NOT NULL REFERENCES pos.revenue_centers(id),
    table_id          UUID REFERENCES pos.tables(id),
    check_number      TEXT NOT NULL,
    status            pos.check_status NOT NULL DEFAULT 'OPEN',
    guest_name        TEXT,
    guest_count       INTEGER NOT NULL DEFAULT 1,
    subtotal          NUMERIC(12,2) NOT NULL DEFAULT 0,
    tax_total         NUMERIC(12,2) NOT NULL DEFAULT 0,
    total             NUMERIC(12,2) NOT NULL DEFAULT 0,
    room_number       TEXT,
    reservation_id    UUID,
    opened_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    closed_at         TIMESTAMPTZ,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_pos_checks_rc ON pos.checks (revenue_center_id);
CREATE INDEX IF NOT EXISTS idx_pos_checks_status ON pos.checks (status);
CREATE INDEX IF NOT EXISTS idx_pos_checks_table ON pos.checks (table_id) WHERE table_id IS NOT NULL;
CREATE UNIQUE INDEX IF NOT EXISTS idx_pos_checks_number ON pos.checks (revenue_center_id, check_number);

-- ═══════════════════════════════════════════════════════════════════════════
-- Orders (line items on a check)
-- ═══════════════════════════════════════════════════════════════════════════

CREATE TABLE IF NOT EXISTS pos.orders (
    id                UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    check_id          UUID NOT NULL REFERENCES pos.checks(id),
    menu_item_id      UUID NOT NULL REFERENCES pos.menu_items(id),
    quantity          INTEGER NOT NULL DEFAULT 1,
    unit_price        NUMERIC(12,2) NOT NULL DEFAULT 0,
    line_total        NUMERIC(12,2) NOT NULL DEFAULT 0,
    status            pos.order_status NOT NULL DEFAULT 'SENT',
    modifiers         JSONB NOT NULL DEFAULT '[]',
    special_instructions TEXT,
    sent_at           TIMESTAMPTZ,
    served_at         TIMESTAMPTZ,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_pos_orders_check ON pos.orders (check_id);
CREATE INDEX IF NOT EXISTS idx_pos_orders_status ON pos.orders (status);

-- ═══════════════════════════════════════════════════════════════════════════
-- Payments
-- ═══════════════════════════════════════════════════════════════════════════

CREATE TABLE IF NOT EXISTS pos.payments (
    id                UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    check_id          UUID NOT NULL REFERENCES pos.checks(id),
    payment_type      pos.payment_type NOT NULL,
    amount            NUMERIC(12,2) NOT NULL DEFAULT 0,
    reference         TEXT,
    tip_amount        NUMERIC(12,2) NOT NULL DEFAULT 0,
    room_number       TEXT,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_pos_payments_check ON pos.payments (check_id);

-- ═══════════════════════════════════════════════════════════════════════════
-- Tax Groups
-- ═══════════════════════════════════════════════════════════════════════════

CREATE TABLE IF NOT EXISTS pos.tax_groups (
    id                UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    revenue_center_id UUID NOT NULL REFERENCES pos.revenue_centers(id),
    tax_code          TEXT NOT NULL,
    tax_name          TEXT NOT NULL,
    tax_rate          NUMERIC(5,4) NOT NULL DEFAULT 0,
    is_active         BOOLEAN NOT NULL DEFAULT true,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_pos_tg_rc ON pos.tax_groups (revenue_center_id);

-- ═══════════════════════════════════════════════════════════════════════════
-- Printer Groups
-- ═══════════════════════════════════════════════════════════════════════════

CREATE TABLE IF NOT EXISTS pos.printer_groups (
    id                UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    revenue_center_id UUID NOT NULL REFERENCES pos.revenue_centers(id),
    printer_code      TEXT NOT NULL,
    printer_name      TEXT NOT NULL,
    station_code      TEXT,
    is_active         BOOLEAN NOT NULL DEFAULT true,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_pos_pg_rc ON pos.printer_groups (revenue_center_id);

-- ═══════════════════════════════════════════════════════════════════════════
-- RLS Policies
-- ═══════════════════════════════════════════════════════════════════════════

ALTER TABLE pos.revenue_centers ENABLE ROW LEVEL SECURITY;
ALTER TABLE pos.revenue_centers FORCE ROW LEVEL SECURITY;
ALTER TABLE pos.service_areas ENABLE ROW LEVEL SECURITY;
ALTER TABLE pos.service_areas FORCE ROW LEVEL SECURITY;
ALTER TABLE pos.tables ENABLE ROW LEVEL SECURITY;
ALTER TABLE pos.tables FORCE ROW LEVEL SECURITY;
ALTER TABLE pos.menu_categories ENABLE ROW LEVEL SECURITY;
ALTER TABLE pos.menu_categories FORCE ROW LEVEL SECURITY;
ALTER TABLE pos.menu_items ENABLE ROW LEVEL SECURITY;
ALTER TABLE pos.menu_items FORCE ROW LEVEL SECURITY;
ALTER TABLE pos.modifier_groups ENABLE ROW LEVEL SECURITY;
ALTER TABLE pos.modifier_groups FORCE ROW LEVEL SECURITY;
ALTER TABLE pos.modifiers ENABLE ROW LEVEL SECURITY;
ALTER TABLE pos.modifiers FORCE ROW LEVEL SECURITY;
ALTER TABLE pos.checks ENABLE ROW LEVEL SECURITY;
ALTER TABLE pos.checks FORCE ROW LEVEL SECURITY;
ALTER TABLE pos.orders ENABLE ROW LEVEL SECURITY;
ALTER TABLE pos.orders FORCE ROW LEVEL SECURITY;
ALTER TABLE pos.payments ENABLE ROW LEVEL SECURITY;
ALTER TABLE pos.payments FORCE ROW LEVEL SECURITY;
ALTER TABLE pos.tax_groups ENABLE ROW LEVEL SECURITY;
ALTER TABLE pos.tax_groups FORCE ROW LEVEL SECURITY;
ALTER TABLE pos.printer_groups ENABLE ROW LEVEL SECURITY;
ALTER TABLE pos.printer_groups FORCE ROW LEVEL SECURITY;

-- env_id is the sole tenant key (architecture-principles.md §4.3).
DROP POLICY IF EXISTS pos_revenue_centers_rls ON pos.revenue_centers;
CREATE POLICY pos_revenue_centers_rls ON pos.revenue_centers
    USING (sentin8_context_initialized() AND env_id = sentin8_current_env_id());

DROP POLICY IF EXISTS pos_service_areas_rls ON pos.service_areas;
CREATE POLICY pos_service_areas_rls ON pos.service_areas
    USING (revenue_center_id IN (SELECT id FROM pos.revenue_centers));

DROP POLICY IF EXISTS pos_tables_rls ON pos.tables;
CREATE POLICY pos_tables_rls ON pos.tables
    USING (service_area_id IN (SELECT id FROM pos.service_areas));

DROP POLICY IF EXISTS pos_menu_categories_rls ON pos.menu_categories;
CREATE POLICY pos_menu_categories_rls ON pos.menu_categories
    USING (revenue_center_id IN (SELECT id FROM pos.revenue_centers));

DROP POLICY IF EXISTS pos_menu_items_rls ON pos.menu_items;
CREATE POLICY pos_menu_items_rls ON pos.menu_items
    USING (revenue_center_id IN (SELECT id FROM pos.revenue_centers));

DROP POLICY IF EXISTS pos_modifier_groups_rls ON pos.modifier_groups;
CREATE POLICY pos_modifier_groups_rls ON pos.modifier_groups
    USING (menu_item_id IN (SELECT id FROM pos.menu_items));

DROP POLICY IF EXISTS pos_modifiers_rls ON pos.modifiers;
CREATE POLICY pos_modifiers_rls ON pos.modifiers
    USING (modifier_group_id IN (SELECT id FROM pos.modifier_groups));

DROP POLICY IF EXISTS pos_checks_rls ON pos.checks;
CREATE POLICY pos_checks_rls ON pos.checks
    USING (revenue_center_id IN (SELECT id FROM pos.revenue_centers));

DROP POLICY IF EXISTS pos_orders_rls ON pos.orders;
CREATE POLICY pos_orders_rls ON pos.orders
    USING (check_id IN (SELECT id FROM pos.checks));

DROP POLICY IF EXISTS pos_payments_rls ON pos.payments;
CREATE POLICY pos_payments_rls ON pos.payments
    USING (check_id IN (SELECT id FROM pos.checks));

DROP POLICY IF EXISTS pos_tax_groups_rls ON pos.tax_groups;
CREATE POLICY pos_tax_groups_rls ON pos.tax_groups
    USING (revenue_center_id IN (SELECT id FROM pos.revenue_centers));

DROP POLICY IF EXISTS pos_printer_groups_rls ON pos.printer_groups;
CREATE POLICY pos_printer_groups_rls ON pos.printer_groups
    USING (revenue_center_id IN (SELECT id FROM pos.revenue_centers));
-- ──────────────────────────────────────────────────────────────────────────
-- Local outbox (DBX, architecture-principles §6.7) — producer path:
-- rows are appended in the SAME transaction as the domain change; the
-- shell's post-commit relay pushes them to Bus POST /v1/produce
-- (M2M sentin8:bus:produce, idempotency_key = row id).
-- Generated from sentin8_outbox::outbox_table_sql("pos").
-- ──────────────────────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS pos.outbox (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    env_id          UUID NOT NULL,
    event_type      TEXT NOT NULL,
    stream_family   TEXT NOT NULL,
    payload         JSONB NOT NULL,
    publisher_id_token  TEXT,
    publisher_subject   TEXT,
    relayed         BOOLEAN NOT NULL DEFAULT false,
    relayed_at      TIMESTAMPTZ,
    relay_attempts  INT NOT NULL DEFAULT 0,
    next_retry_at   TIMESTAMPTZ,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_outbox_unrelayed
    ON pos.outbox (created_at) WHERE relayed = false;
ALTER TABLE pos.outbox ENABLE ROW LEVEL SECURITY;
ALTER TABLE pos.outbox FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS outbox_select ON pos.outbox;
CREATE POLICY outbox_select ON pos.outbox FOR SELECT
    USING (sentin8_context_initialized() AND env_id = sentin8_current_env_id());
DROP POLICY IF EXISTS outbox_insert ON pos.outbox;
CREATE POLICY outbox_insert ON pos.outbox FOR INSERT
    WITH CHECK (sentin8_context_initialized() AND env_id = sentin8_current_env_id());
DROP POLICY IF EXISTS outbox_update ON pos.outbox;
CREATE POLICY outbox_update ON pos.outbox FOR UPDATE
    USING (sentin8_context_initialized() AND env_id = sentin8_current_env_id());
CREATE OR REPLACE FUNCTION pos.pos_notify_outbox() RETURNS TRIGGER
    LANGUAGE plpgsql AS $fn$
BEGIN
    PERFORM pg_notify('pos_outbox_changed', '');
    RETURN NEW;
END;
$fn$;
DROP TRIGGER IF EXISTS outbox_notify_trigger ON pos.outbox;
CREATE TRIGGER outbox_notify_trigger
    AFTER INSERT ON pos.outbox
    FOR EACH ROW
    EXECUTE FUNCTION pos.pos_notify_outbox();

-- ──────────────────────────────────────────────────────────────────────────
-- Local audit journal (architecture-principles §6.8): same-transaction
-- journal rows; the shell's post-commit pusher ships them to Sys
-- POST /v1/audit (M2M sentin8:sys:audit:write). A Sys outage only delays
-- delivery — rows persist until pushed.
-- Generated from sentin8_audit::audit_journal_sql("pos").
-- ──────────────────────────────────────────────────────────────────────────
CREATE TABLE IF NOT EXISTS pos.audit_journal (
    id              UUID PRIMARY KEY,
    env_id          UUID NOT NULL,
    action          TEXT NOT NULL,
    target_resource_type TEXT,
    target_resource_id    TEXT,
    actor_user_id   TEXT,
    actor_subject_type TEXT,
    severity        TEXT NOT NULL DEFAULT 'info',
    correlation_id  TEXT,
    before_state    JSONB,
    after_state     JSONB,
    details         JSONB,
    pushed          BOOLEAN NOT NULL DEFAULT false,
    pushed_at       TIMESTAMPTZ,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS idx_pos_audit_journal_unpushed
    ON pos.audit_journal (created_at) WHERE pushed = false;
ALTER TABLE pos.audit_journal ENABLE ROW LEVEL SECURITY;
ALTER TABLE pos.audit_journal FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS audit_journal_dml ON pos.audit_journal;
CREATE POLICY audit_journal_dml ON pos.audit_journal
    USING (sentin8_context_initialized() AND env_id = sentin8_current_env_id())
    WITH CHECK (sentin8_context_initialized() AND env_id = sentin8_current_env_id());

-- ──────────────────────────────────────────────────────────────────────────
-- Per-service role grants + search_path (§4.3e / VDC §VDC.3):
-- hotel8_{leaf}_service connects via PostgreSQL 18 OAuth (scope→role map in
-- hotel8.sys.pg_oauth_scope_map). search_path puts the service's OWN schema
-- FIRST (outbox/audit_journal resolution for the shell's relay + journal
-- pusher; unqualified domain tables resolve here), then public (RLS helpers).
-- `sentin8-cli vertical enroll`'s never-clobber block preserves this role-level setting.
-- ──────────────────────────────────────────────────────────────────────────
DO $$
BEGIN
    IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'hotel8_pos_service') THEN
        ALTER ROLE hotel8_pos_service SET search_path = pos, public;
        GRANT USAGE ON SCHEMA pos TO hotel8_pos_service;
        GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA pos TO hotel8_pos_service;
        GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA pos TO hotel8_pos_service;
    END IF;
END $$;
