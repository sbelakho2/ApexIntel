#!/usr/bin/env bash
# False-success classification guard (semantic, asymmetric).
#
# The historical version of this guard treated an adjacent
# `// false-success-classification: ...` comment as proof that a suppressed
# error was acceptable. That let an *authoritative* write be silently ignored
# merely by writing "authoritative" next to it. Comments are documentation:
# they must never change correctness status.
#
# This guard decides the status from the code itself:
#
#   * Suppressed fallible writes (`let _ = <store/sqlx call>.await;` with no
#     `?` propagation) are AUTHORITATIVE unless the exact call site is on the
#     explicit telemetry allowlist (scripts/ci/false_success_allowlist.txt,
#     every entry carries a reason). A suppressed authoritative write fails
#     with:  authoritative operation still suppresses error
#   * Fallible read defaults (`.unwrap_or_default()` / `.unwrap_or(false)` /
#     `.unwrap_or(0)` / `.unwrap_or("")` / `.ok().flatten()`) are an approved
#     best-effort pattern for display reads; applying them to an authoritative
#     mutation also fails as an authoritative suppression.
#   * `let _ = <expr>.await?;` and `let _ = <expr>.await.map_err(..)?;`
#     propagate the error and are not findings.
#
# Identifier decoding is covered by the companion rule
# scripts/ci/check_identifier_decode.sh.
#
# Run from anywhere; operates on the repository tree.
# Usage: check_false_success.sh [--self-test]
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "${ROOT}"

python3 - "$@" <<'PY'
import pathlib
import re
import sys
import tempfile

SCAN_DIRS = [
    "crates/worker/src/job_execution",
    "crates/api/src/web",
    "crates/api/src/api_handlers",
]

ALLOWLIST_PATH = pathlib.Path("scripts/ci/false_success_allowlist.txt")

FALLBACK = re.compile(
    r"\.unwrap_or_default\(\)|\.unwrap_or\(false\)|\.unwrap_or\(0\)|"
    r"\.unwrap_or\(0\.0\)|\.unwrap_or\(\"\"\)|\.ok\(\)\.flatten\(\)"
)
DB_HINT = re.compile(
    r"\bstore\b|\bsqlx\b|\bquery_as\b|\bquery\(|try_get\(|fetch_one|fetch_all|fetch_optional|\.pool\b"
)
IGNORED_WRITE = re.compile(r"\blet\s+_\s*=")
CFG_TEST = re.compile(r"^\s*#\[cfg\(test\)\]")
STORE_CALL = re.compile(r"sqlx::query|\bstore\b|\.pool\b|\bstate\b")

# Mutation verbs: suppressing one of these is a false success unless the exact
# file+token is on the telemetry allowlist.
AUTHORITATIVE = re.compile(
    r"^(create|update|upsert|delete|insert|save|set|mark|acknowledge|unacknowledge|"
    r"promote|deprecate|replay|queue|submit|publish|write|persist|link|unlink|assign|"
    r"share|complete|close|open|add|remove|toggle|approve|reject|resolve|dismiss|apply|"
    r"enable|disable|reset|restore|sync|record|unbookmark|bookmark|bulk)_"
)

SQL_KEYWORD = re.compile(r"\b(INSERT|UPDATE|DELETE|SELECT)\b", re.IGNORECASE)
SQL_TABLE = re.compile(
    r"\b(?:INSERT\s+INTO|UPDATE|DELETE\s+FROM|FROM)\s+([a-z_][a-z0-9_]*)",
    re.IGNORECASE,
)

# Wrapper methods that are not the operation under classification.
WRAPPERS = {
    "to_string", "to_value", "json", "format", "vec", "some", "none", "new",
    "parse", "from_str", "collect", "len", "is_empty", "to_owned", "to_vec",
    "into", "min", "max", "clamp", "unwrap", "expect", "unwrap_or",
    "unwrap_or_else", "unwrap_or_default", "ok", "flatten", "as_str",
    "as_deref", "as_ref", "map", "map_err", "context", "with_context", "await",
    "clone", "get", "and_then", "or_else", "or", "filter", "is_some",
    "is_none", "print", "iter", "first", "last", "join", "push", "entry",
    "or_default", "or_insert", "contains", "trim", "to_lowercase", "to_ascii_lowercase",
}

MARKER = re.compile(r"false-success-classification:\s*(best-effort|authoritative)")


def mask_noncode(text):
    """Blank out string/char literals and comments, preserving offsets.

    Keeps line structure so statement scanning is not confused by SQL text,
    format! braces, or `//` comments inside multi-line calls.
    """
    out = list(text)
    i = 0
    n = len(text)
    while i < n:
        c = text[i]
        if c == "/" and i + 1 < n and text[i + 1] == "/":
            j = text.find("\n", i)
            j = n if j == -1 else j
            for k in range(i, j):
                if out[k] != "\n":
                    out[k] = " "
            i = j
            continue
        if c == "/" and i + 1 < n and text[i + 1] == "*":
            depth = 1
            out[i] = " "
            out[i + 1] = " "
            i += 2
            while i < n and depth:
                if text[i] == "/" and i + 1 < n and text[i + 1] == "*":
                    depth += 1
                    out[i] = out[i + 1] = " "
                    i += 2
                elif text[i] == "*" and i + 1 < n and text[i + 1] == "/":
                    depth -= 1
                    out[i] = out[i + 1] = " "
                    i += 2
                else:
                    if out[i] != "\n":
                        out[i] = " "
                    i += 1
            continue
        if c == "r" and i + 1 < n and (text[i + 1] == '"' or text[i + 1] == "#"):
            j = i + 1
            hashes = 0
            while j < n and text[j] == "#":
                hashes += 1
                j += 1
            if j < n and text[j] == '"':
                closer = '"' + "#" * hashes
                end = text.find(closer, j + 1)
                end = n if end == -1 else end + len(closer)
                for k in range(i, end):
                    if out[k] != "\n":
                        out[k] = " "
                i = end
                continue
        if c == '"':
            j = i + 1
            while j < n:
                if text[j] == "\\":
                    j += 2
                    continue
                if text[j] == '"':
                    j += 1
                    break
                j += 1
            j = min(j, n)
            for k in range(i, j):
                if out[k] != "\n":
                    out[k] = " "
            i = j
            continue
        if c == "'":
            # Char literal or lifetime. Only mask a real char literal.
            j = i + 1
            if j < n and text[j] == "\\":
                j += 2
            else:
                j += 1
            if j < n and text[j] == "'":
                j += 1
                for k in range(i, j):
                    out[k] = " "
                i = j
                continue
        i += 1
    return "".join(out)


def statement_end(masked, start):
    """Index just past the `;` terminating the statement starting at `start`."""
    depth = 0
    i = start
    n = len(masked)
    limit = min(n, start + 6000)
    while i < limit:
        c = masked[i]
        if c in "([{":
            depth += 1
        elif c in ")]}":
            depth = max(0, depth - 1)
        elif c == ";" and depth == 0:
            return i + 1
        elif c == "\n" and depth == 0 and i > start:
            # Continuations never start a fresh top-level statement without a
            # `;`; bail out defensively if the statement looks broken.
            tail = masked[start:i]
            if ".await" in tail or "(" in tail:
                pass
        i += 1
    return min(n, start + 6000)


def suppressed(statement):
    """True when the statement awaits a fallible call without propagating `?`."""
    last = statement.rfind(".await")
    if last == -1:
        return False
    tail = statement[last + len(".await"):]
    semi = tail.find(";")
    if semi == -1:
        tail = tail
    else:
        tail = tail[:semi]
    return "?" not in tail


def call_token(masked_statement, original_statement):
    """Identify the operation under classification."""
    if "sqlx::query" in masked_statement:
        keyword = SQL_KEYWORD.search(original_statement)
        table = SQL_TABLE.search(original_statement)
        name = keyword.group(1).upper() if keyword else "QUERY"
        if table:
            return f"sqlx::query:{name} {table.group(1)}"
        return f"sqlx::query:{name}"
    for match in re.finditer(r"\.\s*([a-z_][a-z0-9_]*)\s*\(", masked_statement):
        name = match.group(1)
        if name in WRAPPERS:
            continue
        return name
    return None


def method_before(masked_statement, offset):
    """Last non-wrapper method called before `offset` in a statement."""
    last = None
    for match in re.finditer(r"\.\s*([a-z_][a-z0-9_]*)\s*\(", masked_statement[:offset]):
        name = match.group(1)
        if name in WRAPPERS:
            continue
        last = name
    return last


def load_allowlist(path):
    """Return {(relative_path, token): reason}; validates every entry."""
    errors = []
    entries = {}
    if not path.exists():
        return entries, errors
    for lineno, raw in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        if "|" not in line:
            errors.append(f"{path}:{lineno}: expected 'path | token | reason'")
            continue
        parts = [p.strip() for p in line.split("|")]
        if len(parts) != 3 or not all(parts):
            errors.append(f"{path}:{lineno}: expected 'path | token | reason' with a reason")
            continue
        rel, token, reason = parts
        entries[(rel, token)] = reason
    return entries, errors


def is_authoritative(token):
    if token is None:
        return True  # unknown suppressed operation: fail safe
    return bool(AUTHORITATIVE.match(token))


def scan_text(text, allowlist, path_label):
    """Scan one Rust source text; returns (findings, used_allowlist_entries).

    Findings are `(location, source_line, reason)` tuples.
    """
    lines = text.splitlines()
    masked = mask_noncode(text)
    masked_lines = masked.split("\n")

    cfg_cut = len(lines)
    for index, line in enumerate(lines):
        if CFG_TEST.match(line):
            cfg_cut = index
            break

    findings = []
    used = set()
    offset = 0
    for index in range(len(lines)):
        if index >= cfg_cut:
            break
        line = lines[index]
        stripped = line.strip()
        location = f"{path_label}:{index + 1}"

        if not stripped.startswith("//"):
            # Rule 1: suppressed fallible write (`let _ = ...await;`).
            if IGNORED_WRITE.search(masked_lines[index]):
                eq = masked_lines[index].find("=")
                abs_start = offset + (eq if eq != -1 else 0)
                end = statement_end(masked, abs_start)
                statement = masked[abs_start:end]
                original = text[abs_start:end]
                if (
                    ".await" in statement
                    and STORE_CALL.search(statement)
                    and suppressed(statement)
                ):
                    token = call_token(statement, original)
                    if (path_label, token) in allowlist:
                        used.add((path_label, token))
                    elif is_authoritative(token):
                        findings.append(
                            (
                                location,
                                stripped[:140],
                                "authoritative operation still suppresses error "
                                f"(token: {token})",
                            )
                        )
                    else:
                        findings.append(
                            (
                                location,
                                stripped[:140],
                                "suppressed operation is not on the telemetry "
                                f"allowlist (token: {token})",
                            )
                        )
            # Rule 2: fallible read default applied to an authoritative call.
            elif FALLBACK.search(masked_lines[index]) and any(
                DB_HINT.search(masked_lines[lookback])
                for lookback in range(max(0, index - 8), index + 1)
            ):
                match = FALLBACK.search(masked_lines[index])
                fallback_at = offset + match.start()
                # Scan from the start of the enclosing statement so a chain
                # split across lines still identifies the operation.
                statement_start = (
                    max(
                        masked.rfind(";", 0, fallback_at),
                        masked.rfind("{", 0, fallback_at),
                        masked.rfind("}", 0, fallback_at),
                    )
                    + 1
                )
                end = statement_end(masked, statement_start)
                statement = masked[statement_start:end]
                method = method_before(statement, fallback_at - statement_start)
                if method and AUTHORITATIVE.match(method):
                    findings.append(
                        (
                            location,
                            stripped[:140],
                            "authoritative operation still suppresses error "
                            f"(token: {method})",
                        )
                    )
        offset += len(line) + 1

    return findings, used


def scan_repo(root, allowlist):
    findings = []
    used = set()
    for directory in SCAN_DIRS:
        for path in sorted(pathlib.Path(root, directory).rglob("*.rs")):
            text = path.read_text(encoding="utf-8", errors="replace")
            rel = str(path.relative_to(root))
            file_findings, file_used = scan_text(text, allowlist, rel)
            findings.extend(file_findings)
            used |= file_used
    return findings, used


def self_test(root, allowlist_path):
    fixture = tempfile.TemporaryDirectory()
    base = pathlib.Path(fixture.name)

    def write(name, body):
        (base / name).write_text(body, encoding="utf-8")
        return name

    authoritative_with_comment = write(
        "authoritative_with_comment.rs",
        "async fn f(store: &Store) {\n"
        "    // false-success-classification: authoritative — must be counted\n"
        "    let _ = store.create_pipeline_opportunity(None, \"x\").await;\n"
        "}\n",
    )
    authoritative_with_best_effort_comment = write(
        "authoritative_with_best_effort_comment.rs",
        "async fn f(store: &Store) {\n"
        "    // false-success-classification: best-effort — trying to hide it\n"
        "    let _ = store.create_pipeline_opportunity(None, \"x\").await;\n"
        "}\n",
    )
    best_effort_allowlisted = write(
        "best_effort_allowlisted.rs",
        "async fn f(store: &Store) {\n"
        "    // false-success-classification: best-effort — telemetry only\n"
        "    let _ = store.record_audit_event(\"x\").await;\n"
        "}\n",
    )
    propagated = write(
        "propagated.rs",
        "async fn f(store: &Store) -> anyhow::Result<()> {\n"
        "    let _ = store.create_pipeline_opportunity(None, \"x\").await?;\n"
        "    let _ = store.create_pipeline_opportunity(None, \"x\").await"
        ".map_err(|e| anyhow::anyhow!(e))?;\n"
        "    Ok(())\n"
        "}\n",
    )
    authoritative_default = write(
        "authoritative_default.rs",
        "async fn f(store: &Store) {\n"
        "    let updated = store.update_widget(1).await.unwrap_or_default();\n"
        "}\n",
    )
    best_effort_default = write(
        "best_effort_default.rs",
        "async fn f(store: &Store) {\n"
        "    let rows = store.list_widgets().await.unwrap_or_default();\n"
        "}\n",
    )
    authoritative_default_multiline = write(
        "authoritative_default_multiline.rs",
        "async fn f(store: &Store) {\n"
        "    let updated = store\n"
        "        .update_widget(1)\n"
        "        .await\n"
        "        .unwrap_or_default();\n"
        "}\n",
    )

    allow = {("best_effort_allowlisted.rs", "record_audit_event"): "self-test fixture"}
    cases = [
        (authoritative_with_comment, 1, "authoritative operation still suppresses error"),
        (
            authoritative_with_best_effort_comment,
            1,
            "authoritative operation still suppresses error",
        ),
        (best_effort_allowlisted, 0, None),
        (propagated, 0, None),
        (authoritative_default, 1, "authoritative operation still suppresses error"),
        (best_effort_default, 0, None),
        (
            authoritative_default_multiline,
            1,
            "authoritative operation still suppresses error",
        ),
    ]

    failures = []
    for name, expected_count, expected_message in cases:
        findings, _ = scan_text(
            (base / name).read_text(encoding="utf-8"), allow, name
        )
        if len(findings) != expected_count:
            failures.append(
                f"{name}: expected {expected_count} finding(s), got {findings}"
            )
            continue
        if expected_message and expected_message not in findings[0][2]:
            failures.append(
                f"{name}: expected message containing {expected_message!r}, got {findings[0][2]!r}"
            )

    fixture.cleanup()

    if failures:
        for failure in failures:
            print(f"SELF-TEST FAILED: {failure}", file=sys.stderr)
        return 1

    # The repo allowlist must be well formed.
    _, allow_errors = load_allowlist(pathlib.Path(root, ALLOWLIST_PATH))
    if allow_errors:
        for error in allow_errors:
            print(f"SELF-TEST FAILED: {error}", file=sys.stderr)
        return 1

    print("false-success guard self-test passed")
    return 0


def main():
    root = pathlib.Path.cwd()
    if "--self-test" in sys.argv[1:]:
        return self_test(root, ALLOWLIST_PATH)

    allowlist, allow_errors = load_allowlist(pathlib.Path(root, ALLOWLIST_PATH))
    if allow_errors:
        for error in allow_errors:
            print(f"FALSE-SUCCESS: {error}", file=sys.stderr)
        return 1

    findings, used = scan_repo(root, allowlist)
    stale = sorted(set(allowlist) - used)
    for entry in stale:
        print(
            f"FALSE-SUCCESS: stale allowlist entry (no suppressed operation matched): "
            f"{entry[0]} | {entry[1]}",
            file=sys.stderr,
        )
    if findings or stale:
        for location, text, reason in findings:
            print(f"FALSE-SUCCESS: {reason}: {location}: {text}", file=sys.stderr)
        print(
            f"false-success guard FAILED: {len(findings)} suppression(s); "
            "propagate the error or add a reasoned entry to "
            f"{ALLOWLIST_PATH}",
            file=sys.stderr,
        )
        return 1
    print(
        f"false-success guard passed ({len(allowlist)} allowlisted telemetry "
        "suppression(s), all other fallible store calls propagate)"
    )
    return 0


sys.exit(main())
PY
