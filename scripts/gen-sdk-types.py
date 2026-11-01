#!/usr/bin/env python3
"""Generate the response types of the Python, Go and Node SDKs from openapi/zyvor.yaml.

Each operation with an object response gets a type named from its verb and path (GET /v1/plans/{id} ->
GetPlansByIdResponse). Writes sdks/python/zyvor/types.py (TypedDicts), sdks/go/types.go (structs plus Decode) and
packages/zyvor-node/src/types.ts, then points the Python methods and the untyped Node calls at them. `--check` exits 1
when a committed file differs from what the spec says. Run from the repository root; needs PyYAML.
"""
import re
import subprocess
import sys
from pathlib import Path

import yaml

SPEC = yaml.safe_load(Path("openapi/zyvor.yaml").read_text())
PY, GO, TS = Path("sdks/python/zyvor/types.py"), Path("sdks/go/types.go"), Path("packages/zyvor-node/src/types.ts")
PYINIT, NODE = Path("sdks/python/zyvor/__init__.py"), Path("packages/zyvor-node/src/index.ts")
VERBS = ("get", "post", "put", "patch", "delete")


def camel(s: str) -> str:
    return "".join(w[:1].upper() + w[1:] for w in re.split(r"[^A-Za-z0-9]+", s) if w)


def type_name(verb: str, path: str) -> str:
    parts = []
    for seg in path.removeprefix("/v1").strip("/").split("/"):
        parts.append("By" + camel(seg[1:-1]) if seg.startswith("{") else camel(seg))
    return camel(verb) + "".join(parts) + "Response"


def norm(path: str) -> str:
    return re.sub(r"\$\{[^}]*\}|\{[^}]*\}", "{}", path.split("?")[0])


def resolve(s):
    while isinstance(s, dict) and "$ref" in s:
        s = SPEC["components"]["schemas"][s["$ref"].split("/")[-1]]
    return s


# ---- a schema tree flattened into named object types ----
class Model:
    def __init__(self):
        self.objects = {}  # name -> [(field, shape)]; shape is a tuple tree

    def shape(self, s, name):
        ref = s.get("$ref", "").split("/")[-1] if isinstance(s, dict) else ""
        if ref:
            self.component(ref)
            return ("obj", ref, False)
        t = s.get("type")
        null = isinstance(t, list) and "null" in t
        if isinstance(t, list):
            t = next((x for x in t if x != "null"), None)
        if "allOf" in s:  # an object extended with more properties
            props = {}
            for part in s["allOf"]:
                part = resolve(part)
                props.update(part.get("properties", {}))
            return self.shape({"type": "object", "properties": props}, name)
        if t == "object" and s.get("properties"):
            self.objects[name] = [(k, self.shape(v, name + camel(k))) for k, v in s["properties"].items()]
            return ("obj", name, False)
        if t == "array":
            return ("arr", self.shape(s.get("items", {}), name + "Item"), False)
        if t in ("string", "integer", "number", "boolean"):
            return (t, None, null)
        return ("any", None, False)

    def component(self, ref):
        if ref not in self.objects:
            self.objects[ref] = []  # guards recursion
            s = SPEC["components"]["schemas"][ref]
            self.objects[ref] = [(k, self.shape(v, ref + camel(k))) for k, v in s.get("properties", {}).items()]


def ok_schema(op):
    for code, r in op["responses"].items():
        if str(code).startswith("2") and "application/json" in r.get("content", {}):
            return r["content"]["application/json"]["schema"]
    return None


def collect():
    m, ops = Model(), {}
    for path, item in SPEC["paths"].items():
        for verb in VERBS:
            op = item.get(verb)
            sch = op and ok_schema(op)
            if not sch:
                continue
            name = type_name(verb, path)
            sh = m.shape(sch, name)
            if sh[0] == "obj" and sh[1] != name:  # a bare component: alias by subclassing
                m.objects[name] = list(m.objects[sh[1]])
            if sh[0] in ("obj",):
                ops[(verb.upper(), norm(path))] = name
    return m, ops


# ---- emitters ----
def py_t(sh):
    k, v, null = sh
    base = {"string": "str", "integer": "int", "number": "float", "boolean": "bool", "any": "Any"}.get(k)
    if k == "obj":
        base = v
    if k == "arr":
        base = f"List[{py_t(v)}]"
    return f"Optional[{base}]" if null else base


def emit_py(m):
    out = ['"""Response types, generated from openapi/zyvor.yaml by scripts/gen-sdk-types.py. Do not edit."""',
           "from __future__ import annotations", "", "from typing import Any, List, Optional, TypedDict", ""]
    for name, fields in sorted(m.objects.items()):
        # the functional form, because some keys ("from") are Python keywords; types are strings: they refer forward
        out += ["", f'{name} = TypedDict("{name}", {{']
        out += [f'    "{k}": "{py_t(sh)}",' for k, sh in fields]
        out += ["})"]
    return "\n".join(out) + "\n"


GO_INIT = {"id": "ID", "url": "URL", "gstin": "GSTIN", "upi": "UPI", "vpa": "VPA", "api": "API", "ip": "IP", "pan": "PAN"}


def go_name(k):
    return "".join(GO_INIT.get(w, w[:1].upper() + w[1:]) for w in k.split("_"))


def go_t(sh):
    k, v, null = sh
    if k == "obj":
        return "*" + v if null else v
    base = {"string": "string", "integer": "int64", "number": "float64", "boolean": "bool", "any": "any"}.get(k)
    if k == "arr":
        return "[]" + go_t(v)
    return "*" + base if null else base


def emit_go(m):
    out = ["// Code generated from openapi/zyvor.yaml by scripts/gen-sdk-types.py. DO NOT EDIT.", "", "package zyvor", "",
           'import "encoding/json"', "",
           "// Decode turns the map a call returns into one of the response types:",
           "//", "//\tplans, err := zyvor.Decode[zyvor.GetPlansResponse](c.ListPlans())",
           "func Decode[T any](m map[string]any, err error) (T, error) {", "\tvar out T", "\tif err != nil {", "\t\treturn out, err", "\t}",
           "\tb, err := json.Marshal(m)", "\tif err != nil {", "\t\treturn out, err", "\t}", "\terr = json.Unmarshal(b, &out)", "\treturn out, err", "}"]
    for name, fields in sorted(m.objects.items()):
        out += ["", f"type {name} struct {{"]
        out += [f'\t{go_name(k)} {go_t(sh)} `json:"{k}"`' for k, sh in fields]
        out += ["}"]
    text = "\n".join(out) + "\n"
    try:  # gofmt aligns the struct fields
        return subprocess.run(["gofmt"], input=text, capture_output=True, text=True, check=True).stdout
    except (OSError, subprocess.CalledProcessError):
        return text


def ts_t(sh):
    k, v, null = sh
    base = {"string": "string", "integer": "number", "number": "number", "boolean": "boolean", "any": "unknown"}.get(k)
    if k == "obj":
        base = v
    if k == "arr":
        base = f"{ts_t(v)}[]"
    return f"{base} | null" if null else base


def emit_ts(m):
    out = ["// Response types, generated from openapi/zyvor.yaml by scripts/gen-sdk-types.py. Do not edit."]
    for name, fields in sorted(m.objects.items()):
        out += ["", f"export interface {name} {{"] + [f"  {k}: {ts_t(sh)};" for k, sh in fields] + ["}"]
    return "\n".join(out) + "\n"


# ---- pointing the hand-kept SDK sources at them ----
def patch_python(src, ops):
    names = set(ops.values())
    used = set()

    def fix(m):
        sig, ret, call = m.group(1), m.group(2), m.group(4)
        verb, path = m.group(3), re.sub(r"\{[^}]*\}", "{}", m.group(4).split("?")[0])
        t = ops.get((verb, path))
        if t and (ret in ("dict", "Any") or ret in names or ret == "types"):
            used.add(t)
            return f"{sig}-> types.{t}:"
        return m.group(0)

    pat = re.compile(r'(def \w+\([^)]*\)\s*)->\s*(\w+):(?=(?:\s*""".*?""")?\s*return self\._req\("([A-Z]+)",\s*f?"([^"]*)")', re.S)
    out = pat.sub(fix, src)
    out = out.replace("\nfrom . import types\n", "", 1)
    return out.replace("from typing import Any, Optional\n", "from typing import Any, Optional\n\nfrom . import types\n", 1), used


def patch_node(src, ops, m):
    used = set()

    def fix(mm):
        verb, path = mm.group(2), norm(mm.group(3).strip("`\""))
        t = ops.get((verb, path))
        if not t:
            return mm.group(0)
        used.add(t)
        return f'this.req<{t}>("{verb}", {mm.group(3)}'

    pat = re.compile(r'this\.req<(Record<string, unknown>|Page<Record<string, unknown>>|unknown)>\("([A-Z]+)",\s*([`"][^`"]*[`"])')
    out = pat.sub(fix, src)
    out = re.sub(r"^import type \{[^}]*\} from \"./types\.js\";\n", "", out, flags=re.M)
    used = {n for n in set(ops.values()) if re.search(r"\b" + n + r"\b", out)}
    imp = 'import type { ' + ", ".join(sorted(used)) + ' } from "./types.js";\n'
    first_import_end = [x for x in re.finditer(r"^import .*\n", out, re.M)][-1].end()
    return out[:first_import_end] + imp + out[first_import_end:]


def main():
    check = "--check" in sys.argv
    m, ops = collect()
    py_src, used_py = patch_python(PYINIT.read_text(), ops)
    files = {PY: emit_py(m), GO: emit_go(m), TS: emit_ts(m), PYINIT: py_src, NODE: patch_node(NODE.read_text(), ops, m)}
    if check:
        bad = [str(p) for p, t in files.items() if p.read_text() != t]
        for b in bad:
            print(f"{b} is out of date: run scripts/gen-sdk-types.py")
        return 1 if bad else 0
    for p, t in files.items():
        p.write_text(t)
    print(f"{len(m.objects)} types; {len(used_py)} Python methods annotated")
    return 0


if __name__ == "__main__":
    sys.exit(main())
