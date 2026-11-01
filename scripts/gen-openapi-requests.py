#!/usr/bin/env python3
"""Fill openapi/zyvor.yaml with request bodies and query parameters read from the Rust handlers.

For every route under /v1 the handler's signature is read: `Json<T>` becomes the requestBody and `Query<T>` the query
parameters, with T's fields taken from its struct (Option<..> and #[serde(default)] are optional, #[serde(rename)] is
honoured). The schemas land under components/schemas. `--check` exits 1 when the committed file differs from what the
source says. Run from the repository root; needs PyYAML.
"""
import re
import sys
from pathlib import Path

import yaml

SRC = Path("crates/zyvor-api/src")
SPEC = Path("openapi/zyvor.yaml")
VERBS = ("get", "post", "put", "patch", "delete")


def balanced(text: str, start: int, open_c: str, close_c: str) -> int:
    depth = 0
    for i in range(start, len(text)):
        depth += text[i] == open_c
        depth -= text[i] == close_c
        if depth == 0:
            return i
    raise ValueError("unbalanced")


def routes(lib: str):
    body = lib[lib.index("let v1 = Router::new()") : lib.index("\n    let v1 = v1")]
    for m in re.finditer(r"\.route\(", body):
        end = balanced(body, m.end() - 1, "(", ")")
        call = body[m.end() : end]
        path = re.match(r'\s*"([^"]+)"', call)
        if not path:
            continue
        rest = re.sub(r"//[^\n]*", "", call[path.end() :])
        for verb, handler in re.findall(r"\b(get|post|put|patch|delete)\(\s*([\w:]+)\s*\)", rest):
            yield verb, "/v1" + path.group(1), handler


def split_top(body: str):
    parts, depth, cur, in_str, esc = [], 0, "", False, False
    for c in body:
        if in_str:
            cur += c
            if esc:
                esc = False
            elif c == "\\":
                esc = True
            elif c == '"':
                in_str = False
            continue
        if c == '"':
            in_str = True
        elif c in "([{":
            depth += 1
        elif c in ")]}":
            depth -= 1
        if c == "," and depth == 0:
            parts.append(cur)
            cur = ""
        else:
            cur += c
    parts.append(cur)
    return parts


def object_shape(text: str, at: int):
    """The keys of the json!({ ... }) literal whose `{` is at `at`; a key holding a list of json! objects gets those items."""
    end = balanced(text, at, "{", "}")
    props = {}
    for part in split_top(text[at + 1 : end]):
        m = re.match(r'\s*"(\w+)"\s*:\s*(.*)$', part, re.S)
        if not m:
            continue
        key, val = m.group(1), m.group(2)
        inner = re.search(r"json!\(\s*\{", val)
        if inner and (".collect" in val or "map(" in val):
            props[key] = {"type": "array", "items": object_shape(val, inner.end() - 1)}
        elif re.match(r"\s*json!\(\s*\{", val):
            props[key] = object_shape(val, re.match(r"\s*json!\(\s*\{", val).end() - 1)
        else:
            props[key] = {"type": "integer"} if key.endswith("_minor") else {}  # amounts are integer minor units
    return {"type": "object", "properties": props}


def response_of(src: str, fn: str, rows: dict):
    """(status, schema) for a handler: the first Json(json!({..})) it returns, or a row struct it serialises."""
    m = re.search(r"\bfn\s+" + fn + r"\s*\(", src)
    sig_end = balanced(src, m.end() - 1, "(", ")")
    b0 = src.index("{", sig_end)
    body = src[b0 : balanced(src, b0, "{", "}") + 1]
    status = "201" if "StatusCode::CREATED" in body else "200"
    j = re.search(r"Json\(\s*json!\(\s*\{", body)
    if j:
        return status, object_shape(body, j.end() - 1)
    r = re.search(r"Json\(\s*(?:\w+::)?(\w+Row)\b", body) or re.search(r"->\s*Result<Json<(\w+)>", src[sig_end : b0])
    if r and r.group(1) in rows:
        return status, {"$ref": "#/components/schemas/" + r.group(1)}
    return status, None


def strip_comments(s: str) -> str:
    return re.sub(r"//[^\n]*", "", s)


def structs(text: str, derive: str = "Deserialize") -> dict:
    """name -> list of (field, type, optional) for every struct that derives Deserialize."""
    out = {}
    for m in re.finditer(r"#\[derive\(([^)]*)\)\]((?:\s*#\[[^\]]*\])*)\s*(?:pub\s+)?struct\s+(\w+)\s*\{", text):
        if derive not in m.group(1):
            continue
        close = balanced(text, m.end() - 1, "{", "}")
        out[m.group(3)] = fields(text[m.end() : close])
    return out


def fields(body: str):
    body = strip_comments(body)
    res = []
    # split on commas that are not inside <> or ()
    parts, depth, cur = [], 0, ""
    for c in body:
        depth += c in "<(["
        depth -= c in ">)]"
        if c == "," and depth == 0:
            parts.append(cur)
            cur = ""
        else:
            cur += c
    parts.append(cur)
    for p in parts:
        attrs = re.findall(r"#\[serde\(([^)]*)\)\]", p)
        p = re.sub(r"#\[[^\]]*\]", "", p).strip()
        m = re.match(r"(?:pub(?:\([^)]*\))?\s+)?(\w+)\s*:\s*(.+)$", p, re.S)
        if not m:
            continue
        name, ty = m.group(1), " ".join(m.group(2).split())
        a = " ".join(attrs)
        rn = re.search(r'rename\s*=\s*"([^"]+)"', a)
        res.append((rn.group(1) if rn else name, ty, "default" in a or "skip" in a, "flatten" in a))
    return res


def schema_of(ty: str, known: dict, seen=None):
    ty = ty.strip()
    m = re.match(r"Option<(.+)>$", ty)
    if m:
        return schema_of(m.group(1), known, seen)
    m = re.match(r"Vec<(.+)>$", ty)
    if m:
        return {"type": "array", "items": schema_of(m.group(1), known, seen)}
    ty = ty.split("::")[-1]
    simple = {
        "String": {"type": "string"},
        "str": {"type": "string"},
        "i64": {"type": "integer", "format": "int64"},
        "i32": {"type": "integer", "format": "int32"},
        "i16": {"type": "integer", "format": "int32"},
        "u32": {"type": "integer", "format": "int32"},
        "u64": {"type": "integer", "format": "int64"},
        "usize": {"type": "integer"},
        "f64": {"type": "number", "format": "double"},
        "bool": {"type": "boolean"},
        "NaiveDate": {"type": "string", "format": "date"},
        "DateTime": {"type": "string", "format": "date-time"},
        "Value": {},
        "Currency": {"type": "string", "description": "ISO 4217 code, e.g. INR"},
    }
    if ty.startswith("DateTime"):
        return simple["DateTime"]
    if ty in simple:
        return dict(simple[ty])
    if ty in known and (seen is None or ty not in seen):
        return {"$ref": "#/components/schemas/" + known[ty]}
    return {"type": "object"}


def main() -> int:
    check = "--check" in sys.argv
    lib = (SRC / "lib.rs").read_text()
    per_file = {f.stem: structs(f.read_text()) for f in sorted(SRC.glob("*.rs"))}
    fn_src = {f.stem: f.read_text() for f in sorted(SRC.glob("*.rs"))}
    rows = {}  # Serialize structs that handlers return as they are
    for f in sorted(SRC.glob("*.rs")):
        for n, fl in structs(f.read_text(), "Serialize").items():
            if n.endswith("Row"):
                rows[n] = (f.stem, fl)
    handlers = {}  # (verb, path) -> (module, fn)

    # what each handler takes
    wanted = {}  # (verb, path) -> {"json": (mod, name), "query": (mod, name)}
    for verb, path, handler in routes(lib):
        if "::" in handler:
            mod, fn = handler.rsplit("::", 1)
            mod = mod.split("::")[-1]
        else:  # imported by name: find the one file that defines it
            fn = handler
            homes = [m for m, t in fn_src.items() if re.search(r"\bfn\s+" + fn + r"\s*\(", t)]
            if len(homes) > 1:  # two files define it: the one lib.rs imports (`use admin::*;` or a list)
                homes = [m for m in homes if f"use {m}::*;" in lib] or [
                    m
                    for m in homes
                    if re.search(r"\b" + m + r"::\{[^}]*\b" + fn + r"\b[^}]*\}", lib)
                ]
            if len(homes) != 1:
                continue
            mod = homes[0]
        src = fn_src.get(mod)
        if not src:
            continue
        m = re.search(r"\bfn\s+" + fn + r"\s*\(", src)
        if not m:
            continue
        handlers[(verb, path)] = (mod, fn)
        sig = src[m.end() - 1 : balanced(src, m.end() - 1, "(", ")") + 1]
        took = {}
        j = re.search(r"\bJson<\s*(\w+)\s*>", sig)
        if j and j.group(1) != "Value" and j.group(1) in per_file[mod]:
            took["json"] = (mod, j.group(1), "Option<Json<" in sig)
        q = re.search(r"\bQuery<\s*(?:[\w:]*::)?(\w+)\s*>", sig)
        if q and q.group(1) in per_file[mod]:
            took["query"] = (mod, q.group(1), False)
        if took:
            wanted[(verb, path)] = took

    # schema names: the struct's own name when only one module uses it, else prefixed with the module
    used = sorted({t[:2] for v in wanted.values() for t in v.values()})
    count = {}
    for _, n in used:
        count[n] = count.get(n, 0) + 1
    name_of = {(m, n): (n if count[n] == 1 else m.title().replace("_", "") + n) for m, n in used}

    spec = yaml.safe_load(SPEC.read_text())
    schemas = spec["components"]["schemas"]
    for k in [k for k, v in schemas.items() if v.get("x-generated")]:
        del schemas[k]

    def build(mod, name, as_params=False):
        known = {n: name_of[(m, n)] for (m, n) in name_of if m == mod}
        props, required = {}, []
        for fname, ty, has_default, flat in per_file[mod][name]:
            if flat:
                continue
            props[fname] = schema_of(ty, known)
            if not ty.startswith("Option<") and not has_default:
                required.append(fname)
        return props, required

    # nested structs referenced from a body
    queue = list(used)
    done = set()
    while queue:
        mod, name = queue.pop()
        if (mod, name) in done:
            continue
        done.add((mod, name))
        props, required = build(mod, name)
        for f in per_file[mod][name]:
            for inner in re.findall(r"\b([A-Z]\w+)\b", f[1]):
                if inner in per_file[mod] and (mod, inner) not in name_of:
                    name_of[(mod, inner)] = inner if inner not in schemas else mod.title() + inner
                    queue.append((mod, inner))
        props, required = build(mod, name)
        s = {"type": "object", "x-generated": True, "properties": props}
        if required:
            s["required"] = required
        schemas[name_of[(mod, name)]] = s

    for n, (mod, fl) in rows.items():
        props = {f[0]: schema_of(f[1], {}) for f in fl}
        req = [f[0] for f in fl if not f[1].startswith("Option<")]
        schemas[n] = {"type": "object", "x-generated": True, "properties": props, "required": req}

    for (verb, path), (mod, fn) in handlers.items():
        op = spec["paths"].get(path, {}).get(verb)
        if op is None:
            continue
        status, sch = response_of(fn_src[mod], fn, rows)
        if sch is None:
            continue
        resp = {"description": "Created" if status == "201" else "OK",
                "content": {"application/json": {"schema": sch}}}
        op["responses"] = {status: resp, **{k: v for k, v in op["responses"].items() if k not in ("200", "201", 200, 201)}}

    for (verb, path), took in wanted.items():
        op = spec["paths"].get(path, {}).get(verb)
        if op is None:
            continue
        op.pop("requestBody", None)
        op["parameters"] = [p for p in op.get("parameters", []) if not p.get("x-generated")]
        if "json" in took:
            op["requestBody"] = {
                "required": not took["json"][2],
                "content": {
                    "application/json": {"schema": {"$ref": "#/components/schemas/" + name_of[took["json"][:2]]}}
                },
            }
        if "query" in took:
            props, required = build(*took["query"][:2])
            have = {(p.get("name"), p.get("in")) for p in op["parameters"]}
            for n, sch in props.items():
                if (n, "query") in have:  # already written by hand
                    continue
                op["parameters"].append(
                    {"name": n, "in": "query", "required": n in required, "schema": sch, "x-generated": True}
                )
        if not op["parameters"]:
            del op["parameters"]

    out = yaml.dump(spec, sort_keys=False, width=120)
    if check:
        if yaml.safe_load(SPEC.read_text()) != yaml.safe_load(out):  # content, not formatting
            print("openapi/zyvor.yaml is out of date: run scripts/gen-openapi-requests.py")
            return 1
        return 0
    SPEC.write_text(out)
    print(f"{len(wanted)} operations, {len(done)} schemas")
    return 0


if __name__ == "__main__":
    sys.exit(main())
