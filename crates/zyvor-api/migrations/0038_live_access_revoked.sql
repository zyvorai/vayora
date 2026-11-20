-- Copyright 2026 Zyvor AI Labs · https://zyvor.dev
-- SPDX-License-Identifier: LicenseRef-Zyvor-Production-1.0

-- A reviewer can take live mode away from a business that is already live (fraud signal, a failed re-check, a
-- request from a provider). The row keeps who did it and why; restoring puts it back to 'approved'.
ALTER TABLE live_access DROP CONSTRAINT live_access_status_check;
ALTER TABLE live_access ADD CONSTRAINT live_access_status_check CHECK (status IN ('pending', 'approved', 'rejected', 'revoked'));
