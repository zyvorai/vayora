-- Copyright 2026 Zyvor AI Labs · https://zyvor.dev
-- SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

-- A metered plan's free units can be per seat: with this on, the allowance on each invoice is included_units times the
-- subscription's quantity. Off (the default) keeps one allowance per invoice, as before.
ALTER TABLE plans ADD COLUMN included_per_seat BOOLEAN NOT NULL DEFAULT false;
