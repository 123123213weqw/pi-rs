#!/usr/bin/env python3
"""Mock OpenAI-compatible chat-completions server for pi-rs E2E tests.

Reads a scenario JSON file (list of scripted assistant responses) and serves
them in order as SSE streams. Usage:
  python3 mock_server.py <scenario.json> [port]

Scenario entry format (one per model turn):
  {"tool_calls": [{"id": "call_1", "name": "bash", "arguments": {"command": "echo hi"}}],
   "text": "", "usage": {"prompt_tokens": 10, "completion_tokens": 5}}
or
  {"text": "done", "usage": {...}}
"""
import json
import sys
from http.server import BaseHTTPRequestHandler, HTTPServer

with open(sys.argv[1]) as f:
    SCENARIO = json.load(f)
CALLS = {"n": 0}


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *a):
        pass

    def do_POST(self):
        length = int(self.headers.get("Content-Length", 0))
        body = json.loads(self.rfile.read(length))
        i = CALLS["n"]
        CALLS["n"] += 1
        if i >= len(SCENARIO):
            self.send_error(500, "scenario exhausted")
            return
        step = SCENARIO[i]

        self.send_response(200)
        self.send_header("Content-Type", "text/event-stream")
        self.end_headers()

        def emit(obj):
            self.wfile.write(f"data: {json.dumps(obj)}\n\n".encode())
            self.wfile.flush()

        # echo mode: return the tool results we received as text (format check)
        if step.get("echo_tool_results"):
            parts = []
            for m in body.get("messages", []):
                if m.get("role") == "tool":
                    parts.append(f"[{m.get('tool_call_id','')}] {m.get('content','')}")
            step = {"text": "\n===\n".join(parts) or "(no tool results)"}

        # first chunk: role
        emit({"choices": [{"index": 0, "delta": {"role": "assistant", "content": ""}}]})
        # reasoning (optional)
        if step.get("reasoning"):
            emit({"choices": [{"index": 0, "delta": {"reasoning_content": step["reasoning"]}}]})
        # text content
        if step.get("text"):
            emit({"choices": [{"index": 0, "delta": {"content": step["text"]}}]})
        # tool calls
        for idx, tc in enumerate(step.get("tool_calls", [])):
            emit({"choices": [{"index": 0, "delta": {"tool_calls": [{
                "index": idx, "id": tc["id"],
                "function": {"name": tc["name"], "arguments": json.dumps(tc["arguments"])},
            }]}}]})
        # finish
        finish = "tool_calls" if step.get("tool_calls") else "stop"
        usage = step.get("usage", {"prompt_tokens": 100, "completion_tokens": 20,
                                   "prompt_tokens_details": {"cached_tokens": 64}})
        emit({"choices": [{"index": 0, "delta": {}, "finish_reason": finish}], "usage": usage})
        emit("data: [DONE]\n\n")


if __name__ == "__main__":
    port = int(sys.argv[2]) if len(sys.argv) > 2 else 8999
    HTTPServer(("127.0.0.1", port), Handler).serve_forever()
