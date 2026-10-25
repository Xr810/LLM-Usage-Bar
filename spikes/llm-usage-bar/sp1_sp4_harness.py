#!/usr/bin/env python3
"""Throwaway spike harness for SP-1..SP-3.

No real provider credentials. Tokens below are synthetic shapes taken from
public docs (prefixes and header names only). This file is not product code
and is not wired into the Tauri app.

What it measures
----------------
SP-1  Classify inbound Claude / Codex / Gemini / coding-plan requests.
      Also show that the current route/ whitelist (content-type + accept,
      drop authorization / x-api-key) destroys that signal before upstream.
SP-2  Replay the Codex base-URL resolution rules copied from
      openai/codex model-provider-info (to_api_provider + merge of built-ins)
      and send the resulting URL at a local mock. WebSocket upgrades are
      counted separately from POST /v1/responses.
SP-3  Local mock upstream SSE. Client aborts mid-stream. Compare
      (a) tokens the mock had already generated,
      (b) usage a response.completed / message_delta parser can see,
      (c) usage visible if the proxy keeps reading after the client leaves
          AND the upstream keeps generating (OpenAI-like "drain" hypothesis).
      Real provider invoices are not queried.
"""

from __future__ import annotations

import json
import socket
import threading
import time
from dataclasses import dataclass, asdict
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from typing import Callable
from urllib.request import Request, urlopen


# --- classification -------------------------------------------------------

API_KEY = "api_key"  # hard-stop eligible
OAUTH = "subscription_oauth"  # alert only, never hard-stop
UNKNOWN = "unknown"  # fail-closed hold; not treated as subscription


def _bearer(value: str | None) -> str | None:
    if not value:
        return None
    parts = value.split(None, 1)
    if len(parts) == 2 and parts[0].lower() == "bearer":
        return parts[1].strip()
    return None


def _is_jwt(token: str) -> bool:
    pieces = token.split(".")
    return len(pieces) == 3 and token.startswith("eyJ") and all(pieces)


def _header(headers: dict[str, str], name: str) -> str | None:
    for key, value in headers.items():
        if key.lower() == name.lower():
            text = value.strip()
            return text or None
    return None


def classify_claude(headers: dict[str, str]) -> str:
    """Claude Code billing identity.

    Official gateway docs (code.claude.com/docs/en/llm-gateway):
    a gateway credential variable replaces the claude.ai login.
    ANTHROPIC_API_KEY is sent as x-api-key; ANTHROPIC_AUTH_TOKEN is sent
    as Authorization: Bearer. Subscription login stays only when neither
    credential variable is set, and then anthropic-beta carries the OAuth
    capability. Token prefix sk-ant-oat / sk-ant-api is from third-party
    writeups (LiteLLM, ARC), not from the Anthropic gateway page itself.
    """
    api_key = _header(headers, "x-api-key")
    bearer = _bearer(_header(headers, "authorization"))
    beta = (_header(headers, "anthropic-beta") or "").lower()
    if api_key:
        return API_KEY
    if bearer:
        if bearer.startswith("sk-ant-oat"):
            return OAUTH
        if "oauth-2025-04-20" in beta and not bearer.startswith("sk-"):
            # capability present but prefix is not the published oat shape
            return UNKNOWN
        return API_KEY
    return UNKNOWN


def classify_openai_codex(headers: dict[str, str], path: str = "") -> str:
    """OpenAI / Codex billing identity on the inbound request.

    API keys use the sk- / sk-proj- / sk-svcacct- / sk-admin- prefixes.
    ChatGPT Codex auth is a JWT access token plus ChatGPT-Account-Id on the
    backend-api path (repo: providers/codex/subscription.rs; Codex source:
    AuthMode::Chatgpt uses CHATGPT_CODEX_BASE_URL). A JWT without the
    account header is not a confirmed subscription: fail-closed unknown,
    so a hard-stop cannot treat it as OAuth and an allow-all cannot treat
    it as a known API key.
    """
    api_key = _header(headers, "x-api-key")
    bearer = _bearer(_header(headers, "authorization")) or ""
    token = api_key or bearer
    if not token:
        return UNKNOWN
    if token.startswith(("sk-", "sk-proj-", "sk-svcacct-", "sk-admin-")):
        return API_KEY
    account = _header(headers, "chatgpt-account-id")
    on_codex_backend = "/backend-api/codex" in path
    if _is_jwt(token) and (account or on_codex_backend):
        return OAUTH
    if _is_jwt(token):
        return UNKNOWN
    return UNKNOWN


def classify_gemini(headers: dict[str, str]) -> str:
    """Gemini CLI. API key auth sends x-goog-api-key. Code Assist OAuth
    sends a Google access token (ya29.) as Bearer. GOOGLE_GEMINI_BASE_URL
    is documented only for gemini-api-key auth.
    """
    goog = _header(headers, "x-goog-api-key")
    if goog:
        return API_KEY
    bearer = _bearer(_header(headers, "authorization")) or ""
    if bearer.startswith("ya29."):
        return OAUTH
    if bearer.startswith("AIza"):
        return API_KEY
    if bearer:
        return UNKNOWN
    return UNKNOWN


def classify_coding_plan_key(headers: dict[str, str]) -> str:
    """Kimi / GLM / MiniMax / Volcengine coding plans authenticate with an
    API key (Bearer or raw Authorization). That is API-key traffic for
    ADR-014, not claude.ai / ChatGPT OAuth. A missing credential is unknown.
    """
    if _header(headers, "x-api-key"):
        return API_KEY
    raw = _header(headers, "authorization")
    if not raw:
        return UNKNOWN
    bearer = _bearer(raw)
    material = bearer if bearer is not None else raw
    if material:
        return API_KEY
    return UNKNOWN


def whitelist_forward_headers(headers: dict[str, str]) -> dict[str, str]:
    """Mirror route/server.rs build_upstream_headers: only content-type and
    accept survive. authorization / x-api-key / cookie are dropped.
    """
    kept = {}
    for key, value in headers.items():
        if key.lower() in {"content-type", "accept"}:
            kept[key] = value
    return kept


# --- fixtures -------------------------------------------------------------

def _claude_api(i: int) -> dict:
    shapes = [
        {"x-api-key": f"sk-ant-api03-synthetic-{i:02d}", "anthropic-version": "2023-06-01"},
        {"x-api-key": f"sk-ant-api03-synthetic-{i:02d}", "anthropic-beta": "context-management-2025-06-27"},
        {"X-Api-Key": f"sk-ant-api03-synthetic-{i:02d}"},
        {"authorization": f"Bearer sk-ant-api03-synthetic-{i:02d}"},
        {"Authorization": "Bearer sk-gateway-synthetic"},
        {"x-api-key": f"sk-ant-api03-synthetic-{i:02d}", "authorization": "Bearer sk-ant-oat01-synthetic"},
    ]
    base = shapes[i % len(shapes)]
    base["content-type"] = "application/json"
    return base


def _claude_oauth(i: int) -> dict:
    beta_variants = [
        "oauth-2025-04-20",
        "oauth-2025-04-20,context-management-2025-06-27",
        "claude-code-20250219,oauth-2025-04-20",
        "OAUTH-2025-04-20",
    ]
    return {
        "authorization": f"Bearer sk-ant-oat01-synthetic-{i:02d}",
        "anthropic-beta": beta_variants[i % len(beta_variants)],
        "anthropic-version": "2023-06-01",
        "x-claude-code-session-id": f"sess-{i:02d}",
        "content-type": "application/json",
    }


def _codex_api(i: int) -> dict:
    prefixes = ["sk-proj-synthetic-", "sk-svcacct-synthetic-", "sk-admin-synthetic-", "sk-synthetic-"]
    token = prefixes[i % len(prefixes)] + f"{i:02d}"
    headers = {"authorization": f"Bearer {token}", "content-type": "application/json"}
    if i % 3 == 0:
        headers["OpenAI-Organization"] = "org-synthetic"
        headers["OpenAI-Project"] = "proj-synthetic"
    if i % 5 == 0:
        headers = {"x-api-key": token, "content-type": "application/json"}
    return headers


def _codex_oauth(i: int) -> dict:
    # three-segment JWT shape, not a real signature
    token = f"eyJhbGciOiJub25lIn0.eyJzdWIiOiJzeW50aGV0aWMta{i:02d}In0.c2ln"
    return {
        "authorization": f"Bearer {token}",
        "ChatGPT-Account-Id": f"acct_synthetic_{i:02d}",
        "OpenAI-Beta": "responses=v1",
        "originator": "codex-cli",
        "content-type": "application/json",
    }


def _gemini_api(i: int) -> dict:
    if i % 2 == 0:
        return {"x-goog-api-key": f"AIza-synthetic-{i:02d}", "content-type": "application/json"}
    return {"authorization": f"Bearer AIza-synthetic-{i:02d}", "content-type": "application/json"}


def _gemini_oauth(i: int) -> dict:
    return {
        "authorization": f"Bearer ya29.synthetic-{i:02d}",
        "content-type": "application/json",
    }


def _plan_key(i: int, style: str) -> dict:
    token = f"plan-synthetic-{style}-{i:02d}"
    if style == "zhipu":
        # repo sends the key as Authorization WITHOUT a Bearer prefix
        return {"authorization": token, "content-type": "application/json"}
    return {"authorization": f"Bearer {token}", "content-type": "application/json"}


@dataclass
class CaseResult:
    suite: str
    index: int
    expected: str
    got: str
    ok: bool


def run_suite(name: str, count: int, build: Callable[[int], dict], classify: Callable[[dict], str], expected: str) -> list[CaseResult]:
    rows = []
    for i in range(count):
        got = classify(build(i))
        rows.append(CaseResult(name, i, expected, got, got == expected))
    return rows


def naive_header_only(headers: dict[str, str]) -> str:
    """The ADR-010 sketch: Bearer means OAuth, x-api-key means API key.
    Included to show where that sketch mis-labels gateway bearer keys.
    """
    if _header(headers, "x-api-key"):
        return API_KEY
    if _bearer(_header(headers, "authorization")):
        return OAUTH
    if _header(headers, "authorization"):
        return API_KEY
    return UNKNOWN


# --- Codex URL resolution (source-derived, not the Codex binary) ---------

CHATGPT_CODEX_BASE_URL = "https://chatgpt.com/backend-api/codex"
OPENAI_API_BASE_URL = "https://api.openai.com/v1"
RESERVED_NO_OVERRIDE = {"openai", "ollama", "lmstudio"}
CHATGPT_AUTH_MODES = {
    "chatgpt",
    "chatgptAuthTokens",
    "headers",
    "agentIdentity",
    "personalAccessToken",
}


def merge_providers(configured: dict[str, dict]) -> dict[str, dict]:
    """merge_configured_model_providers: built-in ids other than Bedrock are
    not replaced. A new id is inserted.
    """
    catalog = {
        "openai": {
            "base_url": None,
            "requires_openai_auth": True,
            "supports_websockets": True,
            "wire_api": "responses",
        }
    }
    for key, provider in configured.items():
        if key in RESERVED_NO_OVERRIDE:
            continue
        catalog.setdefault(key, dict(provider))
    return catalog


def resolve_endpoint(provider: dict, auth_mode: str) -> dict:
    """ModelProviderInfo::to_api_provider plus the websocket transport choice.
    supports_websockets defaults to false for a custom provider (serde default)
    and true on the built-in openai provider.
    """
    if auth_mode in CHATGPT_AUTH_MODES:
        default_base = CHATGPT_CODEX_BASE_URL
    else:
        default_base = OPENAI_API_BASE_URL
    base = provider.get("base_url") or default_base
    base = base.rstrip("/")
    http_url = base + "/responses"
    websockets = bool(provider.get("supports_websockets", False))
    if http_url.startswith("https://"):
        ws_url = "wss://" + http_url[len("https://"):]
    elif http_url.startswith("http://"):
        ws_url = "ws://" + http_url[len("http://"):]
    else:
        ws_url = http_url
    return {
        "base_url": base,
        "http_url": http_url,
        "websocket_url": ws_url if websockets else None,
        "preferred_transport": "websocket" if websockets else "http",
        "hits_local_http_proxy": http_url.startswith("http://127.0.0.1") or http_url.startswith("http://localhost"),
    }


# --- local mock proxy / upstream ------------------------------------------

class Recorder:
    def __init__(self) -> None:
        self.lock = threading.Lock()
        self.events: list[dict] = []

    def add(self, event: dict) -> None:
        with self.lock:
            self.events.append(event)


RECORDER = Recorder()


class ProxyHandler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, fmt: str, *args) -> None:
        return

    def do_POST(self) -> None:  # noqa: N802
        length = int(self.headers.get("Content-Length", "0"))
        body = self.rfile.read(length) if length else b""
        inbound = {k: v for k, v in self.headers.items()}
        forwarded = whitelist_forward_headers(inbound)
        kind = classify_claude(inbound) if self.path.startswith("/claude") else classify_openai_codex(inbound, self.path)
        if self.path.startswith("/gemini"):
            kind = classify_gemini(inbound)
        if self.path.startswith("/plan"):
            kind = classify_coding_plan_key(inbound)
        RECORDER.add({
            "path": self.path,
            "inbound_class": kind,
            "forwarded_has_authorization": any(k.lower() == "authorization" for k in forwarded),
            "forwarded_has_x_api_key": any(k.lower() == "x-api-key" for k in forwarded),
            "body_len": len(body),
        })
        payload = json.dumps({"echo_class": kind}).encode()
        self.send_response(200)
        self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(payload)))
        self.end_headers()
        self.wfile.write(payload)

    def do_GET(self) -> None:  # noqa: N802
        upgrade = (self.headers.get("Upgrade") or "").lower()
        RECORDER.add({"path": self.path, "upgrade": upgrade or None, "method": "GET"})
        if upgrade == "websocket":
            # HTTP-only router does not complete the Codex responses websocket.
            self.send_response(404)
            self.send_header("content-length", "0")
            self.end_headers()
            return
        self.send_response(404)
        self.send_header("content-length", "0")
        self.end_headers()


def start_server(handler: type[BaseHTTPRequestHandler]) -> tuple[ThreadingHTTPServer, int]:
    server = ThreadingHTTPServer(("127.0.0.1", 0), handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    return server, server.server_address[1]


def post_json(url: str, headers: dict[str, str], body: dict) -> dict:
    data = json.dumps(body).encode()
    request = Request(url, data=data, method="POST")
    for key, value in headers.items():
        request.add_header(key, value)
    with urlopen(request, timeout=5) as response:
        return json.loads(response.read().decode())


# --- SP-3 streaming abort -------------------------------------------------

PLANNED_OUTPUT_TOKENS = 20
ABORT_AFTER_DELTAS = 5
INPUT_TOKENS = 1000


class StreamUpstream(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def log_message(self, fmt: str, *args) -> None:
        return

    def do_POST(self) -> None:  # noqa: N802
        length = int(self.headers.get("Content-Length", "0"))
        if length:
            self.rfile.read(length)
        mode = self.path.strip("/") or "anthropic"
        self.send_response(200)
        self.send_header("content-type", "text/event-stream")
        self.send_header("cache-control", "no-cache")
        self.end_headers()
        generated = 0
        try:
            if mode == "openai":
                self._write_sse("response.created", {"type": "response.created", "response": {"id": "resp_synthetic"}})
                for i in range(PLANNED_OUTPUT_TOKENS):
                    generated += 1
                    self._write_sse(
                        "response.output_text.delta",
                        {"type": "response.output_text.delta", "delta": "x", "output_index": 0},
                    )
                self._write_sse(
                    "response.completed",
                    {
                        "type": "response.completed",
                        "response": {
                            "usage": {
                                "input_tokens": INPUT_TOKENS,
                                "output_tokens": PLANNED_OUTPUT_TOKENS,
                            }
                        },
                    },
                )
            else:
                self._write_sse(
                    "message_start",
                    {
                        "type": "message_start",
                        "message": {
                            "usage": {"input_tokens": INPUT_TOKENS, "output_tokens": 0},
                        },
                    },
                )
                for _ in range(PLANNED_OUTPUT_TOKENS):
                    generated += 1
                    self._write_sse(
                        "content_block_delta",
                        {"type": "content_block_delta", "delta": {"type": "text_delta", "text": "x"}},
                    )
                self._write_sse(
                    "message_delta",
                    {
                        "type": "message_delta",
                        "usage": {"output_tokens": PLANNED_OUTPUT_TOKENS},
                        "delta": {"stop_reason": "end_turn"},
                    },
                )
                self._write_sse("message_stop", {"type": "message_stop"})
        except (BrokenPipeError, ConnectionResetError):
            pass
        RECORDER.add({
            "stream_mode": mode,
            "generated_before_close_or_end": generated,
            "planned_output_tokens": PLANNED_OUTPUT_TOKENS,
            "final_usage_emitted": generated == PLANNED_OUTPUT_TOKENS,
        })

    def _write_sse(self, event: str, data: dict) -> None:
        blob = f"event: {event}\ndata: {json.dumps(data)}\n\n".encode()
        self.wfile.write(blob)
        self.wfile.flush()
        time.sleep(0.01)


def parse_sse_usage(chunks: list[bytes]) -> dict:
    """Parsers shaped like route/server.rs usage_from_completed_event, plus
    the Anthropic message_start / message_delta fields that parser ignores.
    """
    text = b"".join(chunks).decode("utf-8", errors="replace")
    events = []
    for block in text.split("\n\n"):
        data_lines = [line[5:].lstrip() for line in block.splitlines() if line.startswith("data:")]
        if not data_lines:
            continue
        try:
            events.append(json.loads("\n".join(data_lines)))
        except json.JSONDecodeError:
            continue
    completed = None
    message_start_input = None
    message_delta_output = None
    deltas = 0
    for event in events:
        kind = event.get("type")
        if kind == "response.completed":
            usage = (event.get("response") or {}).get("usage") or {}
            if "input_tokens" in usage and "output_tokens" in usage:
                completed = (usage["input_tokens"], usage["output_tokens"])
        elif kind == "response.output_text.delta":
            deltas += 1
        elif kind == "message_start":
            usage = (event.get("message") or {}).get("usage") or {}
            message_start_input = usage.get("input_tokens")
        elif kind == "content_block_delta":
            deltas += 1
        elif kind == "message_delta":
            message_delta_output = (event.get("usage") or {}).get("output_tokens")
    return {
        "router_response_completed_usage": completed,
        "anthropic_message_start_input": message_start_input,
        "anthropic_message_delta_output": message_delta_output,
        "content_deltas_seen": deltas,
        "events_seen": len(events),
    }


def read_until_deltas(port: int, path: str, stop_after: int | None) -> tuple[list[bytes], dict]:
    sock = socket.create_connection(("127.0.0.1", port), timeout=5)
    request = (
        f"POST {path} HTTP/1.1\r\n"
        "Host: 127.0.0.1\r\n"
        "Content-Type: application/json\r\n"
        "Content-Length: 2\r\n"
        "Connection: close\r\n"
        "\r\n"
        "{}"
    )
    sock.sendall(request.encode())
    sock.settimeout(2)
    buf = b""
    # read headers
    while b"\r\n\r\n" not in buf:
        chunk = sock.recv(4096)
        if not chunk:
            break
        buf += chunk
    head, _, rest = buf.partition(b"\r\n\r\n")
    body_chunks = [rest] if rest else []
    deltas = rest.decode("utf-8", errors="replace").count("data:")
    try:
        while stop_after is None or deltas < stop_after:
            chunk = sock.recv(4096)
            if not chunk:
                break
            body_chunks.append(chunk)
            deltas += chunk.decode("utf-8", errors="replace").count("data:")
    except (TimeoutError, socket.timeout):
        pass
    sock.close()
    parsed = parse_sse_usage(body_chunks)
    return body_chunks, parsed


def websocket_upgrade(port: int) -> int:
    sock = socket.create_connection(("127.0.0.1", port), timeout=5)
    key = "dGhlIHNhbXBsZSBub25jZQ=="
    request = (
        "GET /v1/responses HTTP/1.1\r\n"
        "Host: 127.0.0.1\r\n"
        "Upgrade: websocket\r\n"
        "Connection: Upgrade\r\n"
        f"Sec-WebSocket-Key: {key}\r\n"
        "Sec-WebSocket-Version: 13\r\n"
        "\r\n"
    )
    sock.sendall(request.encode())
    data = sock.recv(256).decode("utf-8", errors="replace")
    sock.close()
    status = int(data.split(" ", 2)[1])
    return status


def main() -> None:
    report: dict = {"suites": {}, "notes": []}

    suites = []
    suites += run_suite("claude_api_key", 24, _claude_api, classify_claude, API_KEY)
    suites += run_suite("claude_oauth", 24, _claude_oauth, classify_claude, OAUTH)
    suites += run_suite("codex_api_key", 24, _codex_api, classify_openai_codex, API_KEY)
    suites += run_suite(
        "codex_oauth",
        24,
        _codex_oauth,
        lambda headers: classify_openai_codex(headers, "/backend-api/codex/responses"),
        OAUTH,
    )
    suites += run_suite("gemini_api_key", 24, _gemini_api, classify_gemini, API_KEY)
    suites += run_suite("gemini_oauth", 24, _gemini_oauth, classify_gemini, OAUTH)
    for style in ("kimi", "zhipu", "minimax", "volcengine"):
        suites += run_suite(
            f"coding_plan_{style}",
            24,
            lambda i, style=style: _plan_key(i, style),
            classify_coding_plan_key,
            API_KEY,
        )

    # adversarial rows are reported separately; they are not part of the
    # "clear shape" accuracy number.
    adversarial = [
        ("claude_bearer_gateway_key", {"authorization": "Bearer sk-gateway-synthetic"}, classify_claude, API_KEY),
        ("claude_auth_token_is_api_prefix", {"authorization": "Bearer sk-ant-api03-synthetic"}, classify_claude, API_KEY),
        ("claude_oat_without_beta", {"authorization": "Bearer sk-ant-oat01-synthetic"}, classify_claude, OAUTH),
        ("claude_beta_without_oat_prefix", {"authorization": "Bearer not-a-known-prefix", "anthropic-beta": "oauth-2025-04-20"}, classify_claude, UNKNOWN),
        ("claude_both_key_and_oat", {"x-api-key": "sk-ant-api03-synthetic", "authorization": "Bearer sk-ant-oat01-synthetic"}, classify_claude, API_KEY),
        ("claude_empty", {}, classify_claude, UNKNOWN),
        ("codex_jwt_without_account_or_backend_path", {"authorization": "Bearer eyJhbGciOiJub25lIn0.eyJzdWIiOiJ4In0.c2ln"}, lambda h: classify_openai_codex(h, "/v1/responses"), UNKNOWN),
        ("codex_jwt_on_backend_path", {"authorization": "Bearer eyJhbGciOiJub25lIn0.eyJzdWIiOiJ4In0.c2ln"}, lambda h: classify_openai_codex(h, "/backend-api/codex/responses"), OAUTH),
        ("gemini_empty", {}, classify_gemini, UNKNOWN),
        ("plan_empty", {}, classify_coding_plan_key, UNKNOWN),
    ]
    adversarial_rows = []
    naive_misses = []
    for i in range(24):
        headers = _claude_api(i)
        if naive_header_only(headers) != API_KEY:
            naive_misses.append({"index": i, "naive": naive_header_only(headers), "headers": list(headers)})
    for name, headers, fn, expected in adversarial:
        got = fn(headers)
        adversarial_rows.append({"name": name, "expected": expected, "got": got, "ok": got == expected})

    by_suite: dict[str, dict] = {}
    for row in suites:
        bucket = by_suite.setdefault(row.suite, {"n": 0, "ok": 0, "bad": []})
        bucket["n"] += 1
        bucket["ok"] += int(row.ok)
        if not row.ok:
            bucket["bad"].append(asdict(row))
    report["suites"] = by_suite
    report["adversarial"] = adversarial_rows
    report["naive_bearer_means_oauth_misses_on_claude_api_fixtures"] = {
        "n": 24,
        "misses": len(naive_misses),
        "examples": naive_misses[:6],
    }

    # inbound vs stripped
    stripped_indistinguishable = 0
    identifiable_before = 0
    checked = 0
    for builder, classifier in (
        (_claude_api, classify_claude),
        (_claude_oauth, classify_claude),
        (_codex_api, classify_openai_codex),
        (_codex_oauth, lambda h: classify_openai_codex(h, "/backend-api/codex/responses")),
    ):
        for i in range(24):
            headers = builder(i)
            before = classifier(headers)
            after = classifier(whitelist_forward_headers(headers))
            checked += 1
            if before != UNKNOWN:
                identifiable_before += 1
            if before != UNKNOWN and after == UNKNOWN:
                stripped_indistinguishable += 1
    report["current_router_whitelist"] = {
        "requests": checked,
        "identifiable_before_strip": identifiable_before,
        "become_unknown_after_strip": stripped_indistinguishable,
    }

    proxy, proxy_port = start_server(ProxyHandler)
    try:
        for i in range(24):
            post_json(
                f"http://127.0.0.1:{proxy_port}/claude/v1/messages",
                _claude_api(i),
                {"model": "claude-synthetic", "max_tokens": 1, "messages": []},
            )
            post_json(
                f"http://127.0.0.1:{proxy_port}/claude/v1/messages",
                _claude_oauth(i),
                {"model": "claude-synthetic", "max_tokens": 1, "messages": []},
            )
            post_json(
                f"http://127.0.0.1:{proxy_port}/v1/responses",
                _codex_api(i),
                {"model": "gpt-synthetic", "input": "x"},
            )
            post_json(
                f"http://127.0.0.1:{proxy_port}/backend-api/codex/responses",
                _codex_oauth(i),
                {"model": "gpt-synthetic", "input": "x"},
            )
        ws_status = websocket_upgrade(proxy_port)
    finally:
        proxy.shutdown()

    inbound = [e for e in RECORDER.events if "inbound_class" in e]
    report["live_mock_proxy"] = {
        "port": proxy_port,
        "posts": len(inbound),
        "class_counts": {
            API_KEY: sum(1 for e in inbound if e["inbound_class"] == API_KEY),
            OAUTH: sum(1 for e in inbound if e["inbound_class"] == OAUTH),
        },
        "forwarded_kept_authorization": sum(1 for e in inbound if e["forwarded_has_authorization"]),
        "websocket_upgrade_status": ws_status,
    }

    # Codex resolution matrix. Not executed inside the Codex binary.
    pointer_provider = {
        "llm_usage_bar_router": {
            "base_url": "http://127.0.0.1:43123/v1",
            "wire_api": "responses",
        }
    }
    subscription_http = {
        "openai_https": {
            "base_url": "http://127.0.0.1:43123/v1",
            "requires_openai_auth": True,
            "supports_websockets": False,
            "wire_api": "responses",
            "name": "OpenAI",
        }
    }
    builtin_override_attempt = {
        "openai": {
            "base_url": "http://127.0.0.1:43123/v1",
            "requires_openai_auth": True,
            "supports_websockets": False,
            "wire_api": "responses",
        }
    }
    api_key_provider = {
        "proxy": {
            "base_url": "http://127.0.0.1:43123/v1",
            "env_key": "OPENAI_API_KEY",
            "requires_openai_auth": False,
            "wire_api": "responses",
        }
    }
    matrix = {}
    scenarios = {
        "current_pointer_with_chatgpt_login": (pointer_provider, "llm_usage_bar_router", "chatgpt"),
        "current_pointer_with_api_key_login": (pointer_provider, "llm_usage_bar_router", "api"),
        "custom_https_provider_chatgpt_login": (subscription_http, "openai_https", "chatgpt"),
        "reserved_openai_id_override_ignored": (builtin_override_attempt, "openai", "chatgpt"),
        "custom_provider_api_key": (api_key_provider, "proxy", "api"),
        "default_chatgpt_no_config": ({}, "openai", "chatgpt"),
    }
    for name, (configured, selected, auth_mode) in scenarios.items():
        catalog = merge_providers(configured)
        provider = catalog[selected]
        matrix[name] = {
            "selected_existed_after_merge": selected in catalog,
            "provider_base_url_field": provider.get("base_url"),
            "supports_websockets": provider.get("supports_websockets", False),
            **resolve_endpoint(provider, auth_mode),
        }
    report["codex_resolution_matrix"] = matrix

    # SP-3
    upstream, upstream_port = start_server(StreamUpstream)
    try:
        _, abort_anthropic = read_until_deltas(upstream_port, "/anthropic", ABORT_AFTER_DELTAS + 1)
        time.sleep(0.05)
        _, full_anthropic = read_until_deltas(upstream_port, "/anthropic", None)
        time.sleep(0.05)
        _, abort_openai = read_until_deltas(upstream_port, "/openai", ABORT_AFTER_DELTAS + 1)
        time.sleep(0.05)
        _, full_openai = read_until_deltas(upstream_port, "/openai", None)
    finally:
        upstream.shutdown()
    stream_events = [e for e in RECORDER.events if "stream_mode" in e]
    report["stream_abort"] = {
        "planned_output_tokens": PLANNED_OUTPUT_TOKENS,
        "input_tokens": INPUT_TOKENS,
        "abort_after_content_deltas": ABORT_AFTER_DELTAS,
        "client_view_anthropic_abort": abort_anthropic,
        "client_view_anthropic_full": full_anthropic,
        "client_view_openai_abort": abort_openai,
        "client_view_openai_full": full_openai,
        "upstream_generation_log": stream_events,
        "gap_definition": (
            "provider_generated_output_tokens_minus_usage_field_the_client_parsed; "
            "real invoices were not read"
        ),
    }

    # gaps
    def gap(client_view: dict, generated_key_guess: int, usage_output: int | None) -> dict:
        parsed = usage_output if usage_output is not None else 0
        return {
            "mock_generated_output_tokens": generated_key_guess,
            "parsed_output_tokens": usage_output,
            "delta_generated_minus_parsed": generated_key_guess - parsed,
        }

    report["stream_gaps"] = {
        "anthropic_abort_vs_message_delta": gap(
            abort_anthropic,
            abort_anthropic["content_deltas_seen"],
            abort_anthropic["anthropic_message_delta_output"],
        ),
        "anthropic_abort_input_known": abort_anthropic["anthropic_message_start_input"],
        "openai_abort_vs_response_completed": gap(
            abort_openai,
            abort_openai["content_deltas_seen"],
            None if abort_openai["router_response_completed_usage"] is None else abort_openai["router_response_completed_usage"][1],
        ),
        "openai_full_response_completed": full_openai["router_response_completed_usage"],
        "anthropic_full_message_delta_output": full_anthropic["anthropic_message_delta_output"],
    }

    print(json.dumps(report, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
