-- A lead may set other reviewers' weekly hours. Only the operator makes one (`zyvor-api platform-reviewer lead <id>`),
-- so a lead cannot promote themselves or anyone else.
ALTER TABLE platform_reviewers ADD COLUMN is_lead BOOLEAN NOT NULL DEFAULT false;
