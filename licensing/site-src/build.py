"""Build the Filescape website's generated pages.

    python3 build.py [preview-output.html]

Writes ../site/index.html, every page in PAGES (feature, buy, contact, Mac and legal pages),
404.html, ../site/img/, sitemap.xml and robots.txt for Cloudflare Pages. With an argument it also writes a single-file preview of
the homepage with the images inlined as data: URIs (no <html>/<head> wrapper).

Every page shares head.html (styles and search tags), nav.html and foot.html. The Supported
formats page reads its table from docs/src/reference/formats.md, which the app's tests keep in
step with the code, so the website never lists a format the app doesn't preview.
"""
import base64, datetime, html, json, pathlib, re, shutil, subprocess, sys

SRC = pathlib.Path(__file__).resolve().parent
DIST = SRC.parent / "site"  # the Cloudflare Pages folder, next to the buy and legal pages
REPO = SRC.parent.parent
SITE = "https://filescape.app"
IMG = re.compile(r"\{\{img:([\w-]+)\}\}")

head = (SRC / "head.html").read_text()
nav = (SRC / "nav.html").read_text()
foot = (SRC / "foot.html").read_text()

# Live site only: the shown prices and Paddle settings (token, price, success page) in ../site/config.js.
# Paddle.js itself is loaded by the homepage only when someone is about to buy.
CONFIG_TAG = '<script src="/config.js"></script>\n'
# The Pro price search engines show comes from the price the checkout uses (../site/config.js).
PRICE = re.search(r'priceLabel:\s*"\$([\d.]+)"', (DIST / "config.js").read_text()).group(1)

# The version shown on the homepage is the app's own, from Cargo.toml, so the two always match.
VERSION = re.search(r'^version\s*=\s*"([^"]+)"', (REPO / "Cargo.toml").read_text(), re.M).group(1)

HOME_DESCRIPTION = ("Explore your storage, manage your files, preview hundreds of formats and work "
                    "with local AI models, all in one desktop app for Windows, macOS and Linux. Free, "
                    "with a 15-day trial of Pro.")

# (path, output file, body source, title, link-preview title, description, nav key, sitemap priority)
PAGES = [
    ("/", "index.html", "body.html",
     "Filescape: file manager, disk space analyzer, file previews and local AI",
     "Filescape: see your files differently", HOME_DESCRIPTION, "", "1.0"),
    ("/formats", "formats.html", "pages/formats.html",
     "Supported file formats: preview 300+ file types · Filescape",
     "Preview 300+ file types with Filescape",
     "Preview camera RAW, HEIC, PSD, Office, PDF, EPUB, fonts, audio, MIDI, SQLite, 3D models, CAD, "
     "Gaussian splats and more, free, without opening another app.", "formats", "0.8"),
    ("/ai-models", "ai-models.html", "pages/ai-models.html",
     "Find, inspect and run local AI models · Filescape",
     "Find, inspect and run the AI models on your computer",
     "Find every AI model on your computer, see what's inside it, check whether it will run, chat "
     "with it offline and get the space back. Free, and nothing leaves your computer.", "ai-models", "0.8"),
    ("/vfx", "vfx.html", "pages/vfx.html",
     "Filescape for VFX, 3D and creative projects",
     "Filescape for VFX, 3D and creative projects",
     "Frame sequences as one row with missing frames marked, an EXR and DPX player, 3D, CAD and "
     "Gaussian splat previews, and the media every project uses.", "vfx", "0.7"),
]
PAGES += [
    ("/buy", "buy.html", "pages/buy.html", "Buy Filescape Pro", "Buy Filescape Pro",
     "Buy Filescape Pro: one payment, no subscription, active on 3 of your computers at once, with "
     "every update for a year and a 14-day refund.", "", "0.6"),
    ("/mac", "mac.html", "pages/mac.html", "Opening Filescape on a Mac · Filescape",
     "Opening Filescape on a Mac",
     "How to open Filescape the first time on macOS, when it says it can't verify the app, and how "
     "to give it Full Disk Access.", "", "0.5"),
    ("/contact", "contact.html", "pages/contact.html", "Contact and support · Filescape",
     "Contact Filescape", "Send a question, bug report or licence request to Filescape.", "", "0.4"),
    ("/terms", "terms.html", "pages/terms.html", "Terms of sale · Filescape", "Terms of sale",
     "Filescape terms of sale and licence.", "", "0.2"),
    ("/privacy", "privacy.html", "pages/privacy.html", "Privacy policy · Filescape", "Privacy policy",
     "What Filescape collects and why.", "", "0.2"),
    ("/refunds", "refunds.html", "pages/refunds.html", "Refund policy · Filescape", "Refund policy",
     "Filescape refund policy: a full refund within 14 days of purchase.", "", "0.2"),
]
# Pages served but kept out of the sitemap: (output file, body source, title).
UNLISTED = [("404.html", "pages/404.html", "Page not found · Filescape")]

JSONLD = {
    "@context": "https://schema.org", "@type": "SoftwareApplication", "name": "Filescape",
    "url": SITE + "/", "applicationCategory": "UtilitiesApplication",
    "operatingSystem": "Windows 10, Windows 11, macOS 11 or later, Linux",
    "description": HOME_DESCRIPTION, "image": SITE + "/img/og.png",
    "offers": [
        {"@type": "Offer", "name": "Filescape Free", "price": "0", "priceCurrency": "USD"},
        {"@type": "Offer", "name": "Filescape Pro", "price": PRICE, "priceCurrency": "USD",
         "url": SITE + "/#pricing"},
    ],
}


def formats_table():
    """The Supported formats table, from the generated part of the Help Centre's formats page."""
    md = (REPO / "docs/src/reference/formats.md").read_text()
    part = md[md.index("<!-- BEGIN GENERATED"):md.index("<!-- END GENERATED")]
    rows = [l for l in part.splitlines() if l.startswith("| ")][2:]  # skip the header and |---|
    out, exts = [], 0

    def inline(text):
        text = html.escape(text, quote=False)
        text = re.sub(r"`([^`]+)`", r"<code>\1</code>", text)
        return re.sub(r"\*\*([^*]+)\*\*", r"<b>\1</b>", text)

    for row in rows:
        cells = [c.strip() for c in row.strip("|").split("|")]
        name, ext, shows = cells[0], cells[1], cells[2]
        exts += ext.count("`") // 2
        out.append(f"<tr><th scope=\"row\">{inline(name)}</th><td class=\"exts\">{inline(ext)}</td>"
                   f"<td>{inline(shows)}</td></tr>")
    return "\n".join(out), len(rows), exts


def lastmod(*files):
    """The newest commit date of the files a page is made from (today if any are uncommitted)."""
    today = datetime.date.today().isoformat()
    try:
        if subprocess.run(["git", "diff", "--quiet", "HEAD", "--", *files], cwd=SRC).returncode:
            return today
        if subprocess.run(["git", "ls-files", "--error-unmatch", *files], cwd=SRC,
                          capture_output=True).returncode:
            return today  # not committed yet
        out = subprocess.run(["git", "log", "-1", "--format=%cs", "--", *files], cwd=SRC,
                             capture_output=True, text=True).stdout.strip()
        return out or today
    except OSError:
        return today


DIST.mkdir(exist_ok=True)
(DIST / "img").mkdir(exist_ok=True)
for f in [f for ext in ("webp", "png", "mp4", "webm") for f in (SRC / "img").glob("*." + ext)]:
    shutil.copy(f, DIST / "img" / f.name)
(DIST / "fonts").mkdir(exist_ok=True)
for f in (SRC / "fonts").glob("*.woff2"):
    shutil.copy(f, DIST / "fonts" / f.name)

table, n_formats, n_exts = formats_table()
urls = []
# Download checksums: published only when ../site/SHA256SUMS.txt exists (licensing/SETUP.md step 9
# says how to make it), so the site never shows sums for files it can't vouch for.
CHECKSUMS = ('<p class="small" style="margin-top:16px">Check your download: '
             '<a href="/SHA256SUMS.txt">SHA-256 checksums</a> for every file above, for the version '
             'listed in the file.</p>\n      ') if (DIST / "SHA256SUMS.txt").exists() else ""


def render(path, out, src, title, og_title, desc, key, extra_head=""):
    """Write one page: head.html, nav.html, the page's body and foot.html."""
    body = (SRC / src).read_text()
    body = (body.replace("{{formats_table}}", table)
                .replace("{{formats_count}}", str(n_formats))
                .replace("{{extensions_count}}", str(n_exts))
                .replace("{{version}}", VERSION)
                .replace("{{checksums}}", CHECKSUMS))
    page_head = (head.replace("{{title}}", html.escape(title))
                     .replace("{{og_title}}", html.escape(og_title))
                     .replace("{{description}}", html.escape(desc))
                     .replace("{{path}}", path)
                     .replace("{{jsonld}}", '<script type="application/ld+json">\n'
                              + json.dumps(JSONLD, indent=1) + "\n</script>\n" if path == "/" else ""))
    page_nav = re.sub(r"\{\{cur:([\w-]+)\}\}",
                      lambda m: ' aria-current="page"' if m.group(1) == key and key else "", nav)
    page = (
        '<!doctype html>\n<html lang="en">\n<head>\n<meta charset="utf-8">\n'
        '<meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover">\n'
        + extra_head + page_head + CONFIG_TAG + "</head>\n<body>\n"
        + page_nav + IMG.sub(lambda m: f"/img/{m.group(1)}.webp", body) + foot
        + "</body>\n</html>\n"
    )
    for leftover in re.findall(r"\{\{[\w:-]+\}\}", page):
        sys.exit(f"{src}: {leftover} isn't filled in by build.py")
    for img in re.findall(r'src="/img/([\w-]+\.webp)"', page):
        assert (DIST / "img" / img).exists(), f"{src} uses img/{img}, which isn't in site-src/img"
    for href in re.findall(r'href="(/[\w-]*)(?:#[\w-]*)?"', page):
        assert href == "/" or href.strip("/") + ".html" in KNOWN, f"{src} links to {href}, which isn't a page"
    (DIST / out).write_text(page)
    print("wrote", DIST / out)


# Every page a link may point to: generated ones, and files already in ../site.
KNOWN = {out for _, out, *_ in PAGES} | {p.name for p in DIST.glob("*.html")}
for path, out, src, title, og_title, desc, key, priority in PAGES:
    render(path, out, src, title, og_title, desc, key)
    sources = [SRC / "head.html", SRC / "nav.html", SRC / "foot.html", SRC / src]
    if out == "formats.html":
        sources.append(REPO / "docs/src/reference/formats.md")
    urls.append((path, lastmod(*sources), priority))
for out, src, title in UNLISTED:
    render("/" + out.removesuffix(".html"), out, src, title, title, "This page doesn't exist.", "",
           '<meta name="robots" content="noindex">\n')

# Search engines: robots.txt and sitemap.xml. The account and admin sites are on their own hosts
# and say noindex themselves.
(DIST / "sitemap.xml").write_text(
    '<?xml version="1.0" encoding="UTF-8"?>\n'
    '<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">\n'
    + "".join(f"  <url><loc>{SITE}{p}</loc><lastmod>{d}</lastmod><priority>{pr}</priority></url>\n"
              for p, d, pr in urls)
    + "</urlset>\n")
(DIST / "robots.txt").write_text(f"User-agent: *\nAllow: /\n\nSitemap: {SITE}/sitemap.xml\n")
print("wrote", DIST / "sitemap.xml", "and robots.txt")

if len(sys.argv) > 1:
    def inline_img(m):
        data = (SRC / "img" / f"{m.group(1)}.webp").read_bytes()
        return "data:image/webp;base64," + base64.b64encode(data).decode()
    home = (SRC / "body.html").read_text()
    preview = head.replace("{{jsonld}}", "")
    for k, v in (("title", PAGES[0][3]), ("og_title", PAGES[0][4]), ("description", HOME_DESCRIPTION), ("path", "/")):
        preview = preview.replace("{{" + k + "}}", html.escape(v))
    pathlib.Path(sys.argv[1]).write_text(
        preview + re.sub(r"\{\{cur:[\w-]+\}\}", "", nav) + IMG.sub(inline_img, home.replace("{{version}}", VERSION)) + foot)
    print("wrote", sys.argv[1])
