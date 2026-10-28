#!/usr/bin/env python3
"""Recording mock chat-completions server for behavior-atom diffs.

Serves the scripted scenario in order AND records every request body to
<recdir>/turn-<n>.request.json, so both harnesses' wire behavior can be diffed.

Usage: python3 mock_server.py <scenario.json> <recdir> [port]
"""
import json
import os
import sys
from http.server import BaseHTTPRequestHandler, HTTPServer

with open(sys.argv[1]) as f:
    SCENARIO = json.load(f)
REC = sys.argv[2]
PORT = int(sys.argv[3]) if len(sys.argv) > 3 else 8999
os.makedirs(REC, exist_ok=True)
STATE = {"n": 0}


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *a):
        pass

    def do_POST(self):
        length = int(self.headers.get("Content-Length", 0))
        body = json.loads(self.rfile.read(length))
        i = STATE["n"]
        STATE["n"] += 1
        with open(os.path.join(REC, f"turn-{i:02d}.request.json"), "w") as f:
            json.dump(body, f, ensure_ascii=False, indent=1)
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

        if step.get("echo_tool_results"):
            parts = []
            for m in body.get("messages", []):
                if m.get("role") == "tool":
                    parts.append(f"[{m.get('tool_call_id','')}] {m.get('content','')}")
            step = {"text": "\n===\n".join(parts) or "(no tool results)"}

        emit({"choices": [{"index": 0, "delta": {"role": "assistant", "content": ""}}]})
        if step.get("reasoning"):
            emit({"choices": [{"index": 0, "delta": {"reasoning_content": step["reasoning"]}}]})
        if step.get("text"):
            emit({"choices": [{"index": 0, "delta": {"content": step["text"]}}]})
        for idx, tc in enumerate(step.get("tool_calls", [])):
            emit({"choices": [{"index": 0, "delta": {"tool_calls": [{
                "index": idx, "id": tc["id"],
                "function": {"name": tc["name"], "arguments": json.dumps(tc["arguments"])},
            }]}}]})
        finish = "tool_calls" if step.get("tool_calls") else "stop"
        usage = step.get("usage", {"prompt_tokens": 100, "completion_tokens": 20,
                                   "prompt_tokens_details": {"cached_tokens": 64}})
        emit({"choices": [{"index": 0, "delta": {}, "finish_reason": finish}], "usage": usage})
        emit("data: [DONE]\n\n")


if __name__ == "__main__":
    HTTPServer(("127.0.0.1", PORT), Handler).serve_forever()
