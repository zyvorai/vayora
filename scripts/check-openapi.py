#!/usr/bin/env python3
"""Compare the routes the API serves (crates/zyvor-api/src/lib.rs, under /v1) with openapi/zyvor.yaml.

Exit 1 when a route has no operation in the spec, or the spec lists one the router does not serve. A route that is
deliberately left out goes in UNDOCUMENTED below with the reason. Run from the repository root; needs PyYAML.
"""
import re
import sys

import yaml

# (VERB, path): why the spec leaves it out
UNDOCUMENTED = {}

VERBS = ("get", "post", "put", "patch", "delete")


def router_routes(src: str) -> set[tuple[str, str]]:
    start = src.index("let v1 = Router::new()")
    end = src.index("\n    let v1 = v1")
    body = src[start:end]
    out = set()
    for m in re.finditer(r"\.route\(", body):
        i, depth = m.end(), 1
        while depth:
            c = body[i]
            depth += (c == "(") - (c == ")")
            i += 1
        call = body[m.end() : i - 1]
        path = re.match(r'\s*"([^"]+)"', call)
        if not path:
            continue
        # drop string literals and comments so only the method builders are left
        rest = re.sub(r'"[^"]*"', '""', call[path.end() :])
        rest = re.sub(r"//[^\n]*", "", rest)
        for verb in re.findall(r"\b(get|post|put|patch|delete)\(", rest):
            out.add((verb.upper(), "/v1" + path.group(1)))
    return out


def spec_routes(path: str) -> set[tuple[str, str]]:
    doc = yaml.safe_load(open(path))
    return {(v.upper(), p) for p, ops in doc["paths"].items() for v in ops if v in VERBS}


def norm(p: str) -> str:
    return re.sub(r"\{[^}]*\}", "{}", p)


def main() -> int:
    served = router_routes(open("crates/zyvor-api/src/lib.rs").read())
    spec = spec_routes("openapi/zyvor.yaml")
    sn = {(v, norm(p)) for v, p in spec}
    rn = {(v, norm(p)) for v, p in served}
    skip = {(v, norm(p)) for v, p in UNDOCUMENTED}
    missing = sorted(rn - sn - skip)
    extra = sorted(sn - rn)
    for v, p in missing:
        print(f"served but not in openapi: {v} {p}")
    for v, p in extra:
        print(f"in openapi but not served: {v} {p}")
    print(f"{len(served)} routes served, {len(spec)} operations in the spec, {len(missing)} undocumented, {len(extra)} not served")
    return 1 if missing or extra else 0


if __name__ == "__main__":
    sys.exit(main())
