"""Build the Filescape landing page.

    python3 build.py [preview-output.html]

Writes ../site/index.html and ../site/img/ for Cloudflare Pages (the other pages there are
hand-written and share /style.css). With an argument it also writes a
single-file preview with the images inlined as data: URIs (no <html>/<head> wrapper).
"""
import base64, pathlib, re, shutil, sys

SRC = pathlib.Path(__file__).resolve().parent
DIST = SRC.parent / "site"  # the Cloudflare Pages folder, next to the buy and legal pages
head = (SRC / "head.html").read_text()
body = (SRC / "body.html").read_text()
IMG = re.compile(r"\{\{img:([\w-]+)\}\}")

# Live site only: the Paddle settings (token, price, success page) in ../site/config.js, then Paddle.js.
PADDLE_TAG = '<script src="/config.js"></script>\n<script src="https://cdn.paddle.com/paddle/v2/paddle.js"></script>\n'

DIST.mkdir(exist_ok=True)
(DIST / "img").mkdir(exist_ok=True)
for f in (SRC / "img").glob("*.webp"):
    shutil.copy(f, DIST / "img" / f.name)
page = (
    '<!doctype html>\n<html lang="en">\n<head>\n<meta charset="utf-8">\n'
    '<meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover">\n'
    + head + PADDLE_TAG + "</head>\n<body>\n"
    + IMG.sub(lambda m: f"img/{m.group(1)}.webp", body)
    + "</body>\n</html>\n"
)
(DIST / "index.html").write_text(page)
print("wrote", DIST / "index.html")

if len(sys.argv) > 1:
    def inline(m):
        data = (SRC / "img" / f"{m.group(1)}.webp").read_bytes()
        return "data:image/webp;base64," + base64.b64encode(data).decode()
    pathlib.Path(sys.argv[1]).write_text(head + IMG.sub(inline, body))
    print("wrote", sys.argv[1])
