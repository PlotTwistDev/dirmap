"""Make the sample pictures and video the homepage treemap shows on its tiles.

    python3 tools/demo_media.py

Every picture is drawn here from noise (made-up Icelandic landscapes), so the demo never uses a
real photo or anyone's files and the licence is ours. Writes, next to the other site images:

- img/demo-photos.webp: 16 photos, 480x320 each, in a 4x4 grid (the .cr3, .jpg and .psd tiles)
- img/demo-shots.webp: two film shots, 960x400 each, one above the other (the .exr, .dpx and .png
  frame sequences, which crop a little further along each frame, as a camera move would)
- img/demo-film.mp4 and .webm: a 6 second loop of the first shot, played on .mov tiles under the
  pointer (H.264, and VP9 for browsers without it)

The page fetches them after the map first draws (the film only when a .mov tile is hovered), so
they never hold up the page.
Needs numpy, Pillow and ffmpeg (with libx264). The output is the same every run.
"""
import pathlib, subprocess, sys, tempfile
import numpy as np
from PIL import Image

OUT = pathlib.Path(__file__).resolve().parent.parent / "img"


def noise1(rng, n, x, octaves=6, rough=.5):
    """Fractal value noise along x (0..1), roughly -1..1."""
    total, amp, norm = np.zeros_like(x), 1.0, 0.0
    for o in range(octaves):
        k = n * 2 ** o
        pts = rng.uniform(-1, 1, k + 2)
        xs = np.linspace(0, 1, k + 2)
        v = np.interp(x % 1.0, xs, pts)
        total += v * amp; norm += amp; amp *= rough
    return total / norm


def noise2(rng, w, h, cells, octaves=5, rough=.55, stretch=1.0):
    """Fractal value noise over an h x w picture, 0..1."""
    total, amp, norm = np.zeros((h, w)), 1.0, 0.0
    for o in range(octaves):
        cw = max(2, int(cells * 2 ** o / stretch) + 2); ch = max(2, int(cells * 2 ** o * h / w) + 2)
        g = rng.uniform(0, 1, (ch, cw)).astype(np.float32)
        img = Image.fromarray(g, mode="F").resize((w, h), Image.BICUBIC)
        total += np.asarray(img) * amp; norm += amp; amp *= rough
    return np.clip(total / norm, 0, 1)


def mix(a, b, t):
    t = np.asarray(t)[..., None] if np.ndim(t) else t
    return a + (np.asarray(b, float) - a) * t


def render(p, w, h, seed, t=0.0, pan=0.0, span=1.0):
    """One landscape. p: scene settings; t: time (seconds) for moving water, clouds and aurora;
    pan/span: which part of a wider scene is in frame (0..1 of the scene's width)."""
    rng = np.random.default_rng(seed)
    X = pan + np.linspace(0, span, w)
    ys = np.linspace(0, 1, h)[:, None]
    hz = p["horizon"]
    sky_top, sky_low = np.array(p["sky"][0], float), np.array(p["sky"][1], float)
    img = mix(np.broadcast_to(sky_top, (h, w, 3)).copy(), sky_low, np.broadcast_to((ys / hz) ** 1.3, (h, w)))
    # The sun, or a bright patch of overcast sky.
    if "sun" in p:
        sx, sy, sr = p["sun"]
        d = np.sqrt(((X[None, :] - sx) * w / h / span) ** 2 + (ys - sy) ** 2)
        img += np.array(p.get("sun_rgb", (255, 220, 170)), float) * (np.exp(-d / sr) * .9 + (d < sr * .25) * .5)[..., None]
    if p.get("stars"):
        s = rng.uniform(0, 1, (h, w)) > .9985
        img += (s * rng.uniform(90, 230, (h, w)) * (ys < hz))[..., None]
    if "aurora" in p:
        a = p["aurora"]
        base = a[0] + a[1] * noise1(np.random.default_rng(seed + 7), 3, X / 1.5 + t * .004)
        streak = .55 + .45 * noise1(np.random.default_rng(seed + 8), 24, X / 1.5 + t * .01, 4, .7)
        above = base[None, :] - ys
        glow = np.where(above > 0, np.exp(-above / a[2]), np.exp(above / .012)) * streak[None, :]
        colour = mix(np.broadcast_to(np.array((60, 255, 150), float), (h, w, 3)), (200, 80, 255), np.clip(above / (a[2] * 3), 0, 1))
        img += colour * (glow * a[3])[..., None]
    if "clouds" in p:
        c = noise2(np.random.default_rng(seed + 3), w, h, 3 / span, 5, .55, 3.0)
        if t:
            c = np.roll(c, int(t * 3), axis=1)
        k = np.clip((c - p["clouds"][0]) * 3, 0, 1) * np.clip(1 - ys / hz, 0, 1) ** .4
        img = mix(img, p["clouds"][1], k * .8)
    haze = np.array(p["sky"][1], float)
    # Mountain ridges, far to near, each a little darker and less hazy.
    for i, (top, amp, freq, rgb, snow) in enumerate(p["ridges"]):
        r = np.random.default_rng(seed * 31 + i)
        n = noise1(r, freq, X / 2, 7, .52)
        ridge = top - amp * (.65 * (1 - np.abs(n)) ** 2 + .35 * (n * .5 + .5))  # sharp peaks
        depth = (i + 1) / len(p["ridges"])
        colour = mix(np.array(rgb, float), haze, (1 - depth) * .55)
        tex = noise2(np.random.default_rng(seed * 17 + i), w, h, 40 / span, 5, .62)
        inside = ys >= ridge[None, :]
        shade = mix(np.broadcast_to(colour, (h, w, 3)).copy(), colour * .55, np.clip((ys - ridge[None, :]) * 4, 0, 1))
        # Light from the left on slopes facing it: the ridge's slope, carried down the face.
        k = max(3, w // 50)
        slope = np.gradient(np.convolve(np.pad(ridge, k, mode="edge"), np.ones(k) / k, "same")[k:-k]) * w / span
        lit = np.tanh(-slope * 1.5)[None, :] * np.exp(-(ys - ridge[None, :]) / .08)
        shade *= (.8 + .4 * tex + .22 * lit)[..., None]
        if snow:
            # Snow on whatever rises above the snow line, with a ragged edge.
            line = top - amp * (1 - snow) + .05 * (tex - .5)
            cap = ys < line
            shade = np.where(cap[..., None], mix(shade, (236, 241, 248), np.broadcast_to(.8 + .2 * np.clip(lit, 0, 1), (h, w))), shade)
        img = np.where(inside[..., None], shade, img)
    if "fall" in p:  # a waterfall down the nearest cliff
        fx, fw, f0, f1 = p["fall"]
        streaks = noise1(np.random.default_rng(seed + 9), 40, (X - fx) * 8, 3, .6)
        fallx = np.exp(-((X - fx) / fw) ** 2 * 3)
        water = np.clip(fallx * (.7 + .3 * streaks), 0, 1)[None, :] * ((ys > f0) & (ys < f1))
        img = mix(img, (240, 246, 250), water * .95)
        mist = np.exp(-((X - fx) / (fw * 4)) ** 2)[None, :] * np.exp(-((ys - f1) / .06) ** 2)
        img = mix(img, (225, 232, 238), np.clip(mist * .8, 0, 1))
    if "fog" in p:
        fy, fs, frgb = p["fog"]
        n = noise2(np.random.default_rng(seed + 5), w, h, 4 / span, 4, .5, 2.5)
        k = np.exp(-((ys - fy) / fs) ** 2) * (.5 + .7 * n)
        img = mix(img, frgb, np.clip(k, 0, .9))
    wl = p.get("water")
    if wl:
        # Water below wl reflects what's above it, rippled and darkened.
        rows = np.arange(h)
        top = int(wl * h)
        src = np.clip(2 * top - rows, 0, h - 1)
        ripple = noise2(np.random.default_rng(seed + 11), w, h, 2 / span, 3, .5, 8.0)
        shift = ((ripple - .5) * 14 * (rows[:, None] - top) / h * 3 + np.sin(rows[:, None] * .9 + t * 3) * 1.5).astype(int)
        cols = np.clip(np.arange(w)[None, :] + shift, 0, w - 1)
        refl = img[src[:, None], cols]
        tint = np.array(p.get("water_rgb", (30, 50, 70)), float)
        deep = np.clip((rows - top) / (h - top + 1), 0, 1)[:, None]
        refl = mix(refl, tint, .35 + .45 * deep)
        img = np.where((rows >= top)[:, None, None], refl, img)
        if p.get("ice"):
            r = np.random.default_rng(seed + 13)
            for _ in range(p["ice"]):
                cx = r.uniform(0, 1); cy = r.uniform(wl + .02, .95); bw = r.uniform(.02, .07) * (cy - wl + .1) * 3
                xx = (np.linspace(pan, pan + span, w) - cx) / bw
                lumps = noise1(np.random.default_rng(int(cx * 1e6)), 5, xx * .2 + .5, 3)
                hgt = np.clip(1 - np.abs(xx) ** 3, 0, 1) * bw * (.35 + .15 * lumps) * w / h
                m = (ys < cy) & (ys > cy - hgt[None, :])
                facet = .8 + .2 * np.sign(xx)[None, :]
                img = np.where(m[..., None], mix(np.array((175, 215, 240), float) * facet[..., None], (245, 250, 255), .4), img)
        if p.get("surf"):
            foam = noise2(np.random.default_rng(seed + 15), w, h, 3 / span, 4, .6, 6.0)
            shore = p["sand"][0]
            bands = (np.sin((ys - wl) * 140 - t * 2 + foam * 9) > .9) & (foam > .45)
            near = np.clip(1 - (shore - ys) / .14, 0, 1) * (ys > wl)
            img = mix(img, (225, 232, 236), bands * near * .85 + near ** 3 * .35)
    if "sand" in p:
        sy, srgb = p["sand"]
        g = noise2(np.random.default_rng(seed + 17), w, h, 30 / span, 3, .7)
        sand = np.array(srgb, float) * (.75 + .5 * g)[..., None]
        edge = sy + .02 * noise1(np.random.default_rng(seed + 18), 6, X / 2)
        img = np.where((ys > edge[None, :])[..., None], sand, img)
    # Grain and a soft vignette, as a camera would.
    img = img * (1 - .35 * (((X[None, :] - pan) / span - .5) ** 2 + (ys - .5) ** 2))[..., None]
    img += np.random.default_rng(seed + 19 + int(t * 25)).normal(0, 4, (h, w, 1))
    return np.clip(img, 0, 255).astype(np.uint8)


# The scenes. ridges: (top, height, wiggle, colour, snow)
SCENES = [
    dict(name="sunset lake", sky=((60, 70, 140), (250, 150, 90)), horizon=.62, sun=(.62, .55, .05),
         ridges=[(.5, .2, 3, (120, 90, 140), .3), (.6, .14, 4, (60, 45, 70), 0)], water=.62, water_rgb=(40, 30, 50),
         clouds=(.55, (255, 170, 140))),
    dict(name="aurora", sky=((6, 10, 26), (20, 40, 70)), horizon=.66, stars=True, aurora=(.35, .12, .12, .9),
         ridges=[(.55, .18, 3, (40, 50, 70), .35), (.64, .1, 5, (14, 18, 26), 0)], water=.66, water_rgb=(6, 10, 20)),
    dict(name="glacier lagoon", sky=((110, 160, 215), (210, 228, 240)), horizon=.55, clouds=(.5, (245, 248, 252)),
         ridges=[(.4, .18, 3, (150, 160, 175), .7), (.52, .1, 6, (70, 75, 85), .2)], water=.55, water_rgb=(40, 80, 100), ice=14),
    dict(name="black sand beach", sky=((130, 138, 150), (200, 205, 210)), horizon=.5, clouds=(.4, (170, 175, 182)),
         ridges=[(.45, .25, 18, (45, 45, 50), 0)], water=.5, water_rgb=(60, 75, 85), surf=True, sand=(.78, (40, 40, 44))),
    dict(name="waterfall", sky=((150, 180, 210), (215, 225, 230)), horizon=.4, clouds=(.5, (235, 238, 240)),
         ridges=[(.3, .1, 3, (75, 85, 70), 0), (.86, .05, 8, (70, 105, 50), 0)], fall=(.5, .025, .27, .88),
         fog=(.88, .06, (220, 228, 232))),
    dict(name="green valley", sky=((120, 160, 200), (220, 225, 215)), horizon=.5, sun=(.25, .2, .06), sun_rgb=(120, 110, 80),
         ridges=[(.45, .2, 3, (110, 140, 90), 0), (.6, .18, 4, (80, 120, 60), 0), (.8, .12, 3, (60, 95, 45), 0)],
         fog=(.6, .07, (230, 235, 232)), clouds=(.55, (250, 250, 248))),
    dict(name="blue hour", sky=((30, 50, 100), (120, 140, 190)), horizon=.6, stars=True,
         ridges=[(.48, .2, 4, (90, 105, 150), .4), (.6, .15, 5, (40, 50, 85), .15)], water=.6, water_rgb=(20, 30, 60)),
    dict(name="midnight sun", sky=((90, 120, 170), (255, 205, 130)), horizon=.6, sun=(.3, .5, .04), sun_rgb=(255, 210, 150),
         ridges=[(.5, .15, 3, (150, 120, 110), 0), (.62, .12, 4, (110, 85, 60), 0), (.85, .1, 6, (70, 60, 40), 0)],
         clouds=(.6, (255, 200, 160))),
]


def main():
    # 16 photos: each scene twice, from different seeds.
    cw, ch = 480, 320
    sheet = Image.new("RGB", (cw * 4, ch * 4))
    for i in range(16):
        sc = SCENES[i % len(SCENES)]
        sheet.paste(Image.fromarray(render(sc, cw, ch, 100 + i)), ((i % 4) * cw, (i // 4) * ch))
        print("photo", i, sc["name"], file=sys.stderr)
    sheet.save(OUT / "demo-photos.webp", quality=72, method=6)
    # Two shots for the frame sequences: a lagoon (sh010) and a beach (sh020), wider than the frame.
    shots = Image.new("RGB", (960, 800))
    shots.paste(Image.fromarray(render(SCENES[2], 960, 400, 501, span=1.6)), (0, 0))
    shots.paste(Image.fromarray(render(SCENES[3], 960, 400, 502, span=1.6)), (0, 400))
    shots.save(OUT / "demo-shots.webp", quality=72, method=6)
    # The film: the first shot, the camera drifting along it while the water and ice move.
    with tempfile.TemporaryDirectory() as tmp:
        frames = 6 * 24
        for f in range(frames):
            t = f / 24
            pan = .3 * (.5 - .5 * np.cos(2 * np.pi * f / frames))  # there and back, so it loops
            Image.fromarray(render(SCENES[2], 640, 360, 501, t=t, pan=pan, span=1.0)).save(f"{tmp}/f{f:04d}.png")
        subprocess.run(["ffmpeg", "-y", "-loglevel", "error", "-framerate", "24", "-i", f"{tmp}/f%04d.png",
                        "-c:v", "libx264", "-preset", "slow", "-crf", "30", "-pix_fmt", "yuv420p", "-movflags", "+faststart",
                        "-an", str(OUT / "demo-film.mp4")], check=True)
        # VP9 too, for browsers without H.264 (some Linux builds); the page picks what plays.
        subprocess.run(["ffmpeg", "-y", "-loglevel", "error", "-framerate", "24", "-i", f"{tmp}/f%04d.png",
                        "-c:v", "libvpx-vp9", "-b:v", "0", "-crf", "42", "-row-mt", "1", "-pix_fmt", "yuv420p",
                        "-an", str(OUT / "demo-film.webm")], check=True)
    for f in ("demo-photos.webp", "demo-shots.webp", "demo-film.mp4", "demo-film.webm"):
        print(f, (OUT / f).stat().st_size // 1024, "KB")


if __name__ == "__main__":
    main()
