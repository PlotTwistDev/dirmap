#!/usr/bin/env python3
# Test proxy for dirmap's live transfer tests: forwards LPORT to 127.0.0.1:DPORT, cutting the first
# CUTS connections after CUT bytes each way, and optionally limiting each to RATE bytes/s.
# Usage: flaky-proxy.py LPORT DPORT CUT CUTS [RATE]   e.g. flaky-proxy.py 2223 2222 2500000 3
import socket, threading, sys
lport, dport, cut, cuts = int(sys.argv[1]), int(sys.argv[2]), int(sys.argv[3]), int(sys.argv[4])
rate = float(sys.argv[5]) if len(sys.argv) > 5 else 0
import time
count = [0]
def pipe(a, b, limit, both):
    n = 0
    try:
        while True:
            d = a.recv(65536)
            if not d: break
            n += len(d)
            if limit and n > limit:
                both(); return
            b.sendall(d)
            if rate: time.sleep(len(d) / rate)
    except OSError: pass
    both()
def handle(c):
    s = socket.create_connection(("127.0.0.1", dport))
    count[0] += 1
    limit = cut if count[0] <= cuts else 0
    def both():
        for x in (c, s):
            try: x.shutdown(socket.SHUT_RDWR)
            except OSError: pass
    threading.Thread(target=pipe, args=(c, s, limit, both), daemon=True).start()
    threading.Thread(target=pipe, args=(s, c, limit, both), daemon=True).start()
l = socket.socket(); l.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
l.bind(("127.0.0.1", lport)); l.listen(16)
while True:
    c, _ = l.accept(); threading.Thread(target=handle, args=(c,), daemon=True).start()
