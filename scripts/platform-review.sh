#!/usr/bin/env bash
# For Zyvor reviewers: look at businesses asking for live mode, and approve or reject them
# (docs/zyvor/PLATFORM_REVIEW.md). Needs a reviewer key (zp_...), which is not a merchant API key.
#
#   ZYVOR_API_URL=https://api.example.com ZYVOR_PLATFORM_KEY=zp_... scripts/platform-review.sh list [pending|approved|rejected]
#   ... show <organisation_id>                 what the business submitted and the checks that ran
#   ... approve <organisation_id> ["note"]     opens live mode for that business
#   ... reject <organisation_id> "what to fix" the merchant is shown this note (required)
#   ... revoke <organisation_id> "why"         takes live mode away from an approved business (note required)
#   ... restore <organisation_id> ["note"]     gives it back
#   ... list <status> [me|none|<reviewer id>]  only the reviews one reviewer holds, or that nobody holds
#   ... claim <organisation_id>                take a pending review so only you decide it
#   ... release <organisation_id>              hand it back to the queue
#   ... take-over <organisation_id>            take it from the reviewer holding it
#   ... assign <organisation_id> <reviewer id> give it to another reviewer
#   ... change-claim|change-release|change-take-over <id>, change-assign <id> <reviewer id>   the same for a change
#   ... escalate <organisation_id> "why"      ask for a second, different reviewer (note required)
#   ... changes [status] [me|none|<reviewer id>]   what live businesses changed after approval (a new bank account, or new legal details)
#   ... change-approve <id> ["note"]           accepts the change; live payouts are no longer held for it
#   ... change-escalate <id> "why"             ask for a second, different reviewer on a pending change (note required)
#   ... change-reject <id> "why"               removes the account; the merchant is shown why (note required)
#
# Never prints the key.
set -euo pipefail
: "${ZYVOR_API_URL:?set ZYVOR_API_URL to the API address}"
: "${ZYVOR_PLATFORM_KEY:?set ZYVOR_PLATFORM_KEY to your reviewer key (zp_...)}"
case "$ZYVOR_PLATFORM_KEY" in zp_*) ;; *) echo "refusing: that is not a reviewer key (it starts zp_)" >&2; exit 2;; esac
base="${ZYVOR_API_URL%/}"

call() { # method path [json-body]
  local args=(-sS -X "$1" "$base$2" -H "authorization: Bearer $ZYVOR_PLATFORM_KEY" -H "content-type: application/json")
  if [ -n "${3:-}" ]; then args+=(-d "$3"); fi
  curl "${args[@]}"
}

case "${1:-}" in
  list)
    call GET "/v1/platform/reviews?${2:+status=$2&}${3:+assigned=$3}" | python3 -c '
import sys, json
d = json.load(sys.stdin)
if "error" in d: print("error:", d["error"]["message"]); sys.exit(1)
for r in d["data"]:
    flags = (" OVERDUE" if r.get("overdue") else "") + (" [awaiting 2nd after " + r["first_approved_by"] + "]" if r.get("stage") == "awaiting_second" else (" [needs 2 reviewers]" if r.get("needs_second_because") and r["status"] == "pending" else ""))
    print("%-8s %-34s %-28s asked %s (round %s)  checks not passed: %s%s%s" % (r["status"], r["organisation_id"], (r["legal_name"] or r["organisation"])[:28], r["requested_at"][:16], r["round"], r["checks_failed"], flags, (" {held by " + r["assigned_to_name"] + "}") if r.get("assigned_to_name") else ""))
if not d["data"]: print("nothing here")' ;;
  show) call GET "/v1/platform/reviews/${2:?usage: show <organisation_id>}" | python3 -c '
import sys, json
d = json.load(sys.stdin)
if "error" in d: print("error:", d["error"]["message"]); sys.exit(1)
p = d.get("profile") or {}
print("%s  [%s, round %s]  asked by %s" % (d["organisation"], d["status"], d["round"], d["requested_by"]))
print("legal name :", p.get("legal_name"), "|", (p.get("business_type") or "").replace("_", " "))
print("PAN / GSTIN:", p.get("pan"), "|", p.get("gstin") or "none")
print("address    :", p.get("address"))
print("onboarding :", p.get("status"), "by", p.get("reviewed_by"), "|", "; ".join(p.get("review_reasons") or []) or "no reasons recorded")
print("documents  :"); [print("   -", x["kind"], x["reference"], "(" + x["status"] + ")") for x in d["documents"]]
print("banks      :"); [print("   -", b["holder_name"], b["ifsc"], "xxxx" + b["account_last4"], "verified" if b["verified"] else "NOT verified") for b in d["bank_accounts"]]
print("checks     :"); [print("   -", c["kind"], c["outcome"], "-", c["detail"]) for c in d["checks"]]
if d.get("note"): print("last note  :", d["note"])' ;;
  approve) call POST "/v1/platform/reviews/${2:?usage: approve <organisation_id> [note]}/decision" "$(python3 -c 'import sys,json; print(json.dumps({"decision":"approve","note":sys.argv[1] or None}))' "${3:-}")" | python3 -c 'import sys,json; d=json.load(sys.stdin); print(d["error"]["message"] if "error" in d else "approved: live mode is open for " + d["organisation_id"])' ;;
  reject)
    if [ -z "${3:-}" ]; then echo "say what the merchant must fix:  $0 reject <organisation_id> \"what to fix\"" >&2; exit 2; fi
    call POST "/v1/platform/reviews/${2:?usage: reject <organisation_id> \"what to fix\"}/decision" "$(python3 -c 'import sys,json; print(json.dumps({"decision":"reject","note":sys.argv[1]}))' "${3:?say what the merchant must fix}")" | python3 -c 'import sys,json; d=json.load(sys.stdin); print(d["error"]["message"] if "error" in d else "rejected: the merchant will see your note")' ;;
  revoke)
    if [ -z "${3:-}" ]; then echo "say why:  $0 revoke <organisation_id> \"why\"" >&2; exit 2; fi
    call POST "/v1/platform/reviews/${2:?usage: revoke <organisation_id> \"why\"}/revoke" "$(python3 -c 'import sys,json; print(json.dumps({"note":sys.argv[1]}))' "$3")" | python3 -c 'import sys,json; d=json.load(sys.stdin); print(d["error"]["message"] if "error" in d else "revoked: no new payments or payouts for " + d["organisation_id"])' ;;
  restore) call POST "/v1/platform/reviews/${2:?usage: restore <organisation_id> [note]}/restore" "$(python3 -c 'import sys,json; print(json.dumps({"note":sys.argv[1] or None}))' "${3:-}")" | python3 -c 'import sys,json; d=json.load(sys.stdin); print(d["error"]["message"] if "error" in d else "restored: live mode is open again for " + d["organisation_id"])' ;;
  claim|release|take-over|assign|change-claim|change-release|change-take-over|change-assign)
    verb="${1#change-}"
    what=reviews; [ "$verb" != "$1" ] && what=changes
    id="${2:?usage: $1 <id>}"
    case "$verb" in
      claim) body='{"to":"me"}' ;;
      release) body='{}' ;;
      take-over) body='{"to":"me","take_over":true}' ;;
      assign) body="$(python3 -c 'import sys,json; print(json.dumps({"to":sys.argv[1],"take_over":True}))' "${3:?usage: $1 <id> <reviewer id>}")" ;;
    esac
    call POST "/v1/platform/$what/$id/assign" "$body" | python3 -c 'import sys,json; d=json.load(sys.stdin); print(d["error"]["message"] if "error" in d else (d.get("organisation_id") or d["id"]) + (" is held by " + d["assigned_to_name"] if d.get("assigned_to_name") else " is back in the queue"))' ;;
  escalate)
    if [ -z "${3:-}" ]; then echo "say why:  $0 escalate <organisation_id> \"why\"" >&2; exit 2; fi
    call POST "/v1/platform/reviews/${2:?usage: escalate <organisation_id> \"why\"}/escalate" "$(python3 -c 'import sys,json; print(json.dumps({"note":sys.argv[1]}))' "$3")" | python3 -c 'import sys,json; d=json.load(sys.stdin); print(d["error"]["message"] if "error" in d else "escalated: a second reviewer must approve " + d["organisation_id"])' ;;
  changes)
    call GET "/v1/platform/changes?${2:+status=$2&}${3:+assigned=$3}" | python3 -c '
import sys, json
d = json.load(sys.stdin)
if "error" in d: print("error:", d["error"]["message"]); sys.exit(1)
for c in d["data"]:
    print("%-8s %-30s %-28s %s  (%s, %s)" % (c["status"], c["id"], c["organisation"][:28], c["summary"], c["requested_by"], c["requested_at"][:16]) + (" [awaiting 2nd after " + c["first_approved_by"] + "]" if c.get("stage") == "awaiting_second" else (" [needs 2 reviewers]" if c.get("needs_second_because") and c["status"] == "pending" else "")) + (" {held by " + c["assigned_to_name"] + "}" if c.get("assigned_to_name") else ""))
if not d["data"]: print("nothing here")' ;;
  change-approve) call POST "/v1/platform/changes/${2:?usage: change-approve <id> [note]}/decision" "$(python3 -c 'import sys,json; print(json.dumps({"decision":"approve","note":sys.argv[1] or None}))' "${3:-}")" | python3 -c 'import sys,json; d=json.load(sys.stdin); print(d["error"]["message"] if "error" in d else "approved: payouts are no longer held for " + d["id"])' ;;
  change-escalate)
    if [ -z "${3:-}" ]; then echo "say why:  $0 change-escalate <id> \"why\"" >&2; exit 2; fi
    call POST "/v1/platform/changes/${2:?usage: change-escalate <id> \"why\"}/escalate" "$(python3 -c 'import sys,json; print(json.dumps({"note":sys.argv[1]}))' "$3")" | python3 -c 'import sys,json; d=json.load(sys.stdin); print(d["error"]["message"] if "error" in d else "escalated: a second reviewer must approve " + d["id"])' ;;
  change-reject)
    if [ -z "${3:-}" ]; then echo "say why:  $0 change-reject <id> \"why\"" >&2; exit 2; fi
    call POST "/v1/platform/changes/${2:?usage: change-reject <id> \"why\"}/decision" "$(python3 -c 'import sys,json; print(json.dumps({"decision":"reject","note":sys.argv[1]}))' "$3")" | python3 -c 'import sys,json; d=json.load(sys.stdin); print(d["error"]["message"] if "error" in d else "rejected: the account was removed and the merchant will see why")' ;;
  *) sed -n '2,23p' "$0"; exit 1 ;;
esac
