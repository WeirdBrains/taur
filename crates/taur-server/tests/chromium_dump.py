#!/usr/bin/env python3
"""Wait until a headless Chromium page's text contains a marker, then print the DOM.

Chromium's --dump-dom returns before this app's fetch() calls finish. The
DevTools socket lets us poll document.body.innerText.
"""
import base64
import json
import os
import socket
import subprocess
import sys
import time
import urllib.request


def connect(url: str) -> socket.socket:
    rest = url.split("://", 1)[1]
    hostport, path = rest.split("/", 1)
    host, port = hostport.split(":")
    sock = socket.create_connection((host, int(port)), timeout=5)
    key = base64.b64encode(os.urandom(16)).decode()
    req = (
        f"GET /{path} HTTP/1.1\r\n"
        f"Host: {hostport}\r\n"
        "Upgrade: websocket\r\n"
        "Connection: Upgrade\r\n"
        f"Sec-WebSocket-Key: {key}\r\n"
        "Sec-WebSocket-Version: 13\r\n\r\n"
    )
    sock.sendall(req.encode())
    buf = b""
    while b"\r\n\r\n" not in buf:
        chunk = sock.recv(4096)
        if not chunk:
            raise SystemExit("devtools handshake failed")
        buf += chunk
    return sock


def send(sock: socket.socket, payload: str) -> None:
    data = payload.encode()
    frame = bytearray([0x81])
    n = len(data)
    if n < 126:
        frame.append(0x80 | n)
    else:
        frame.append(0x80 | 126)
        frame.extend(n.to_bytes(2, "big"))
    mask = os.urandom(4)
    frame.extend(mask)
    frame.extend(b ^ mask[i % 4] for i, b in enumerate(data))
    sock.sendall(frame)


def recv(sock: socket.socket) -> str:
    def read(n: int) -> bytes:
        buf = b""
        while len(buf) < n:
            chunk = sock.recv(n - len(buf))
            if not chunk:
                raise SystemExit("devtools closed")
            buf += chunk
        return buf

    header = read(2)
    n = header[1] & 0x7F
    if n == 126:
        n = int.from_bytes(read(2), "big")
    elif n == 127:
        n = int.from_bytes(read(8), "big")
    return read(n).decode()


def main() -> None:
    url, profile, marker = sys.argv[1:4]
    hook = sys.argv[4] if len(sys.argv) > 4 else ""
    port = 0
    sock = socket.socket()
    sock.bind(("127.0.0.1", 0))
    port = sock.getsockname()[1]
    sock.close()
    proc = subprocess.Popen(
        [
            "chromium",
            "--headless=new",
            "--no-sandbox",
            "--disable-gpu",
            "--no-first-run",
            f"--remote-debugging-port={port}",
            f"--user-data-dir={profile}",
            url,
        ],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    try:
        tabs = None
        for _ in range(50):
            try:
                with urllib.request.urlopen(f"http://127.0.0.1:{port}/json", timeout=1) as r:
                    tabs = json.load(r)
                break
            except Exception:
                time.sleep(0.1)
        if not tabs:
            raise SystemExit("devtools did not start")
        page = next(
            (
                t
                for t in tabs
                if t.get("type") == "page" and str(t.get("url", "")).startswith(url.split("?")[0])
            ),
            None,
        )
        if page is None:
            page = next((t for t in tabs if t.get("type") == "page"), None)
        if page is None:
            raise SystemExit(f"no page target in {tabs}")
        ws = connect(page["webSocketDebuggerUrl"])
        send(ws, json.dumps({"id": 0, "method": "Runtime.enable"}))
        expr = "document.documentElement.outerHTML"
        notes = []
        limit = 80 if hook else 40
        for i in range(limit):
            if hook:
                send(ws, json.dumps({"id": i + 500, "method": "Runtime.evaluate", "params": {"expression": hook, "returnByValue": True}}))
                while True:
                    msg = json.loads(recv(ws))
                    if msg.get("id") == i + 500:
                        break
            send(ws, json.dumps({"id": i + 1, "method": "Runtime.evaluate", "params": {"expression": "document.body.innerText", "returnByValue": True}}))
            while True:
                msg = json.loads(recv(ws))
                if msg.get("method") == "Runtime.exceptionThrown":
                    notes.append(str(msg.get("params", {}).get("exceptionDetails", {}))[:400])
                if msg.get("id") == i + 1:
                    text = msg.get("result", {}).get("result", {}).get("value") or ""
                    break
            if marker in text:
                send(ws, json.dumps({"id": 1000, "method": "Runtime.evaluate", "params": {"expression": expr, "returnByValue": True}}))
                while True:
                    msg = json.loads(recv(ws))
                    if msg.get("id") == 1000:
                        print(msg["result"]["result"]["value"])
                        return
            time.sleep(0.1)
        raise SystemExit(
            f"marker {marker!r} not seen; last text={text[:500]!r}; page={page.get('url')}; notes={notes[:3]}"
        )
    finally:
        proc.terminate()
        try:
            proc.wait(timeout=3)
        except subprocess.TimeoutExpired:
            proc.kill()


if __name__ == "__main__":
    main()
