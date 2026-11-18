-- Hosted seller page ("link in bio" storefront) and three small vertical templates built on payment intents,
-- invoices and instalments: school fees and society dues (one engine, `kind` says which), and donations.
-- Additive only. Every table is tenant-scoped with row-level security, except the slug directory, which holds
-- nothing but the public address of a page and the tenant it belongs to (read before any tenant is known).

-- ---------- storefront ----------
CREATE TABLE storefronts (
    id TEXT PRIMARY KEY, tenant_id TEXT NOT NULL UNIQUE REFERENCES tenants(id),
    slug TEXT NOT NULL,
    title TEXT NOT NULL, description TEXT NOT NULL DEFAULT '',
    logo_image_id TEXT,
    -- pages ask search engines to stay away unless the seller turns this off
    noindex BOOLEAN NOT NULL DEFAULT true,
    published BOOLEAN NOT NULL DEFAULT false,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(), updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE TABLE storefront_directory (
    slug TEXT PRIMARY KEY CHECK (slug ~ '^[a-z0-9]([a-z0-9-]{1,38})[a-z0-9]$'),
    tenant_id TEXT NOT NULL UNIQUE REFERENCES tenants(id)
);
CREATE TABLE storefront_images (
    id TEXT PRIMARY KEY, tenant_id TEXT NOT NULL REFERENCES tenants(id),
    content_type TEXT NOT NULL, size_bytes INT NOT NULL CHECK (size_bytes > 0), data BYTEA NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
ALTER TABLE storefronts ADD FOREIGN KEY (logo_image_id) REFERENCES storefront_images(id);
CREATE TABLE storefront_items (
    id TEXT PRIMARY KEY, tenant_id TEXT NOT NULL REFERENCES tenants(id),
    storefront_id TEXT NOT NULL REFERENCES storefronts(id),
    name TEXT NOT NULL, description TEXT NOT NULL DEFAULT '',
    price_minor BIGINT NOT NULL CHECK (price_minor > 0),
    image_id TEXT REFERENCES storefront_images(id),
    -- NULL: not counted. A number is what is left to sell; it never goes below zero.
    stock INT CHECK (stock IS NULL OR stock >= 0),
    active BOOLEAN NOT NULL DEFAULT true, position INT NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(), updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX storefront_items_page ON storefront_items (storefront_id, position, created_at);
CREATE TABLE storefront_orders (
    id TEXT PRIMARY KEY, tenant_id TEXT NOT NULL REFERENCES tenants(id),
    storefront_id TEXT NOT NULL REFERENCES storefronts(id),
    intent_id TEXT NOT NULL UNIQUE REFERENCES payment_intents(id),
    code TEXT NOT NULL,
    buyer_name TEXT NOT NULL, buyer_phone TEXT NOT NULL, buyer_email TEXT, delivery_note TEXT,
    total_minor BIGINT NOT NULL CHECK (total_minor > 0), currency TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('pending', 'paid', 'fulfilled', 'cancelled', 'expired')),
    -- the stock of this order is taken out of the items right now (put back exactly once)
    stock_held BOOLEAN NOT NULL DEFAULT true,
    -- money arrived for an order whose stock had already been given back and could not be taken again
    needs_attention BOOLEAN NOT NULL DEFAULT false,
    hold_until TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    paid_at TIMESTAMPTZ, fulfilled_at TIMESTAMPTZ, cancelled_at TIMESTAMPTZ
);
CREATE INDEX storefront_orders_list ON storefront_orders (tenant_id, created_at DESC);
CREATE INDEX storefront_orders_holds ON storefront_orders (hold_until) WHERE status = 'pending';
CREATE TABLE storefront_order_items (
    id TEXT PRIMARY KEY, tenant_id TEXT NOT NULL REFERENCES tenants(id),
    order_id TEXT NOT NULL REFERENCES storefront_orders(id),
    item_id TEXT NOT NULL REFERENCES storefront_items(id),
    name TEXT NOT NULL, unit_minor BIGINT NOT NULL CHECK (unit_minor > 0),
    quantity INT NOT NULL CHECK (quantity BETWEEN 1 AND 100)
);
CREATE INDEX storefront_order_items_order ON storefront_order_items (order_id);

-- ---------- school fees and society dues (one engine) ----------
CREATE TABLE dues_groups (
    id TEXT PRIMARY KEY, tenant_id TEXT NOT NULL REFERENCES tenants(id),
    kind TEXT NOT NULL CHECK (kind IN ('school', 'society')),
    name TEXT NOT NULL, created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (tenant_id, kind, name)
);
CREATE TABLE dues_heads (
    id TEXT PRIMARY KEY, tenant_id TEXT NOT NULL REFERENCES tenants(id),
    kind TEXT NOT NULL CHECK (kind IN ('school', 'society')),
    group_id TEXT NOT NULL REFERENCES dues_groups(id),
    name TEXT NOT NULL, amount_minor BIGINT NOT NULL CHECK (amount_minor > 0),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (group_id, name)
);
-- A student (school) or a member (society). A student is kept as a name, a class and the school's own reference,
-- nothing else; the person who pays is an ordinary customer.
CREATE TABLE dues_members (
    id TEXT PRIMARY KEY, tenant_id TEXT NOT NULL REFERENCES tenants(id),
    kind TEXT NOT NULL CHECK (kind IN ('school', 'society')),
    group_id TEXT NOT NULL REFERENCES dues_groups(id),
    name TEXT NOT NULL, member_ref TEXT NOT NULL,
    payer_customer_id TEXT REFERENCES customers(id),
    active BOOLEAN NOT NULL DEFAULT true,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (tenant_id, kind, member_ref)
);
CREATE INDEX dues_members_group ON dues_members (group_id);
CREATE TABLE dues_assignments (
    id TEXT PRIMARY KEY, tenant_id TEXT NOT NULL REFERENCES tenants(id),
    member_id TEXT NOT NULL REFERENCES dues_members(id),
    label TEXT NOT NULL, total_minor BIGINT NOT NULL CHECK (total_minor > 0), instalments INT NOT NULL CHECK (instalments BETWEEN 1 AND 60),
    plan_id TEXT REFERENCES installment_plans(id),
    heads JSONB NOT NULL DEFAULT '[]',
    created_by TEXT NOT NULL, created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (member_id, label)
);
CREATE TABLE dues_assignment_invoices (
    invoice_id TEXT PRIMARY KEY REFERENCES invoices(id), tenant_id TEXT NOT NULL REFERENCES tenants(id),
    assignment_id TEXT NOT NULL REFERENCES dues_assignments(id), installment_no INT NOT NULL
);
CREATE INDEX dues_assignment_invoices_assignment ON dues_assignment_invoices (assignment_id);
-- Words the business wants printed on every receipt (a registration line, an address). Printed as typed.
CREATE TABLE vertical_settings (
    tenant_id TEXT NOT NULL REFERENCES tenants(id), kind TEXT NOT NULL CHECK (kind IN ('school', 'society')),
    receipt_note TEXT NOT NULL DEFAULT '', updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (tenant_id, kind)
);

-- ---------- donations ----------
CREATE TABLE donation_campaigns (
    id TEXT PRIMARY KEY, tenant_id TEXT NOT NULL REFERENCES tenants(id),
    name TEXT NOT NULL, description TEXT NOT NULL DEFAULT '',
    suggested_minor BIGINT[] NOT NULL DEFAULT '{}',
    min_minor BIGINT NOT NULL DEFAULT 100 CHECK (min_minor > 0), max_minor BIGINT NOT NULL DEFAULT 100000000 CHECK (max_minor > 0),
    goal_minor BIGINT CHECK (goal_minor > 0),
    -- words the merchant wants on a receipt (registration text, exemption text, a note): printed as typed, never checked
    receipt_fields JSONB NOT NULL DEFAULT '{}',
    recurring_enabled BOOLEAN NOT NULL DEFAULT false,
    status TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'closed')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(), updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (min_minor <= max_minor)
);
CREATE TABLE donations (
    id TEXT PRIMARY KEY, tenant_id TEXT NOT NULL REFERENCES tenants(id),
    campaign_id TEXT NOT NULL REFERENCES donation_campaigns(id),
    intent_id TEXT NOT NULL UNIQUE REFERENCES payment_intents(id),
    donor_name TEXT, donor_email TEXT, donor_phone TEXT, anonymous BOOLEAN NOT NULL DEFAULT false,
    amount_minor BIGINT NOT NULL CHECK (amount_minor > 0), currency TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'paid')),
    receipt_no TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(), paid_at TIMESTAMPTZ
);
CREATE INDEX donations_campaign ON donations (campaign_id, created_at DESC);
CREATE TABLE donation_pledges (
    id TEXT PRIMARY KEY, tenant_id TEXT NOT NULL REFERENCES tenants(id),
    campaign_id TEXT NOT NULL REFERENCES donation_campaigns(id),
    customer_id TEXT NOT NULL REFERENCES customers(id),
    mandate_id TEXT NOT NULL REFERENCES mandates(id),
    amount_minor BIGINT NOT NULL CHECK (amount_minor > 0), frequency TEXT NOT NULL,
    created_by TEXT NOT NULL, created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- ---------- row-level security ----------
DO $$ DECLARE t TEXT; BEGIN
  FOREACH t IN ARRAY ARRAY['storefronts', 'storefront_images', 'storefront_items', 'storefront_orders', 'storefront_order_items',
      'dues_groups', 'dues_heads', 'dues_members', 'dues_assignments', 'dues_assignment_invoices', 'vertical_settings',
      'donation_campaigns', 'donations', 'donation_pledges'] LOOP
    EXECUTE format('ALTER TABLE %I ENABLE ROW LEVEL SECURITY', t);
    EXECUTE format('ALTER TABLE %I FORCE ROW LEVEL SECURITY', t);
    EXECUTE format($p$CREATE POLICY tenant_isolation ON %I USING (tenant_id = current_setting('zyvor.tenant_id', true)) WITH CHECK (tenant_id = current_setting('zyvor.tenant_id', true))$p$, t);
  END LOOP;
END $$;
