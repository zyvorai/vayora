"""Zyvor Payments Python SDK (stdlib only). Amounts are integer minor units (paise)."""
from __future__ import annotations

import hashlib
import hmac
import json
import time
import urllib.error
import urllib.request
import uuid
from typing import Any, Optional

__all__ = ["Zyvor", "ZyvorError", "verify_webhook"]


class ZyvorError(Exception):
    def __init__(self, status: int, code: str, message: str):
        super().__init__(f"{status} {code}: {message}")
        self.status, self.code, self.message = status, code, message


class Zyvor:
    def __init__(self, api_key: str, base_url: str = "http://localhost:8080", actor: Optional[str] = None, opener=None):
        self.api_key, self.base, self.actor = api_key, base_url.rstrip("/"), actor
        self._open = opener or urllib.request.urlopen

    def _req(self, method: str, path: str, body: Any = None, idem: Optional[str] = None) -> Any:
        headers = {"authorization": f"Bearer {self.api_key}"}
        data = None
        if body is not None:
            data, headers["content-type"] = json.dumps(body).encode(), "application/json"
        if idem:
            headers["idempotency-key"] = idem
        if self.actor:
            headers["zyvor-actor"] = self.actor
        req = urllib.request.Request(self.base + path, data=data, method=method, headers=headers)
        try:
            with self._open(req, timeout=30) as res:
                return json.loads(res.read() or b"null")
        except urllib.error.HTTPError as e:
            payload = json.loads(e.read() or b"{}").get("error", {})
            raise ZyvorError(e.code, payload.get("code", "unknown"), payload.get("message", e.reason)) from None

    def create_intent(self, amount_minor: int, currency: str = "INR", idempotency_key: Optional[str] = None, **extra) -> dict:
        if not isinstance(amount_minor, int) or isinstance(amount_minor, bool) or amount_minor <= 0:
            raise ValueError("amount_minor must be a positive integer (paise)")
        return self._req("POST", "/v1/payment_intents", {"amount_minor": amount_minor, "currency": currency, **extra}, idempotency_key or str(uuid.uuid4()))

    def get_intent(self, intent_id: str) -> dict:
        return self._req("GET", f"/v1/payment_intents/{intent_id}")

    def create_attempt(self, intent_id: str, method: str, vpa: Optional[str] = None, idempotency_key: Optional[str] = None) -> dict:
        return self._req("POST", f"/v1/payment_intents/{intent_id}/attempts", {"method": method, "vpa": vpa}, idempotency_key or str(uuid.uuid4()))

    def refund(self, intent_id: str, amount_minor: Optional[int] = None, idempotency_key: Optional[str] = None, **extra) -> dict:
        body = {k: v for k, v in {"amount_minor": amount_minor, **extra}.items() if v is not None}
        return self._req("POST", f"/v1/payment_intents/{intent_id}/refunds", body, idempotency_key or str(uuid.uuid4()))

    def explain_settlement(self, settlement_id: str) -> dict:
        return self._req("GET", f"/v1/settlements/{settlement_id}/explain")


def verify_webhook(secret: str, header: str, raw_body: bytes, tolerance_seconds: int = 300, now: Optional[int] = None) -> bool:
    """Verify `Zyvor-Signature: t=<unix>,v1=<hex>` against the RAW body. De-duplicate on event id too."""
    parts = dict(p.split("=", 1) for p in header.split(",") if "=" in p)
    try:
        t = int(parts["t"])
        given = bytes.fromhex(parts["v1"])
    except (KeyError, ValueError):
        return False
    if abs((now if now is not None else int(time.time())) - t) > tolerance_seconds:
        return False
    expected = hmac.new(secret.encode(), f"{t}.".encode() + raw_body, hashlib.sha256).digest()
    return hmac.compare_digest(expected, given)
