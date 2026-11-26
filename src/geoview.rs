//! Maps: GeoJSON, GPX (tracks from GPS devices and sports apps) and KML/KMZ (Google Earth) drawn
//! as outlines, with their distance, climb, times and counts.
//!
//! There's no map underneath: drawing one would mean fetching tiles from a server, and a preview
//! never goes online. The shapes are drawn on a plain background with a faint grid of latitude and
//! longitude.

use resvg::tiny_skia;
use std::io::Read;
use std::path::Path;

/// Files larger than this aren't read.
const FILE_MAX: u64 = 48 << 20;
/// Coordinates kept; the rest of a huge file is left out of the picture.
const POINTS_MAX: usize = 2_000_000;

type P = [f64; 2];

/// What's read: shapes in (longitude, latitude), or (x, y) for files in another projection.
#[derive(Default)]
pub struct Geo {
    pub points: Vec<P>,
    pub lines: Vec<Vec<P>>,
    /// Polygons: an outer ring, then holes.
    pub areas: Vec<Vec<Vec<P>>>,
    pub facts: Vec<(&'static str, String)>,
    /// Coordinates outside latitude and longitude's ranges: drawn flat, no distances.
    planar: bool,
    count: usize,
}

pub fn read(path: &Path) -> Result<Geo, String> {
    let ext = path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    let text = || -> Result<String, String> {
        let meta = std::fs::metadata(path).map_err(|e| e.to_string())?;
        if meta.len() > FILE_MAX {
            return Err(format!("Too big to draw ({})", crate::scan::human(meta.len())));
        }
        let mut b = Vec::new();
        std::fs::File::open(path).and_then(|f| f.take(FILE_MAX).read_to_end(&mut b)).map_err(|e| e.to_string())?;
        Ok(String::from_utf8_lossy(&b).into_owned())
    };
    let mut g = match ext.as_str() {
        "gpx" => gpx(&text()?)?,
        "kml" => kml(&text()?)?,
        "kmz" => kml(&kmz_doc(path)?)?,
        _ => geojson(&text()?)?,
    };
    let planar = g.all().any(|p| !(-180.0..=360.0).contains(&p[0]) || !(-90.0..=90.0).contains(&p[1]));
    g.planar = planar;
    if g.count == 0 {
        return Err("No shapes in it".into());
    }
    if let Some((a, b)) = g.bounds()
        && !g.planar
    {
        g.facts.push(("Covers", format!("{} to {}", latlon(a), latlon(b))));
    }
    if g.count >= POINTS_MAX {
        g.facts.push(("Drawn", format!("The first {} points", crate::inspect::group(POINTS_MAX as u64))));
    }
    Ok(g)
}

/// The KML document inside a KMZ (a zip).
fn kmz_doc(path: &Path) -> Result<String, String> {
    let mut zip = zip::ZipArchive::new(std::fs::File::open(path).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let name = (0..zip.len())
        .filter_map(|i| zip.by_index(i).ok().and_then(|f| f.name().ok().map(|n| n.to_string())))
        .filter(|n| n.to_ascii_lowercase().ends_with(".kml"))
        .min_by_key(|n| (n.to_ascii_lowercase() != "doc.kml", n.matches('/').count()))
        .ok_or("No KML inside")?;
    let f = zip.by_name(&name).map_err(|e| e.to_string())?;
    if f.size() > FILE_MAX {
        return Err("Too big to draw".into());
    }
    let mut s = String::new();
    f.take(FILE_MAX).read_to_string(&mut s).map_err(|e| e.to_string())?;
    Ok(s)
}

fn latlon(p: P) -> String {
    let (lon, lat) = (p[0], p[1]);
    format!("{:.4}°{} {:.4}°{}", lat.abs(), if lat < 0.0 { "S" } else { "N" }, lon.abs(), if lon < 0.0 { "W" } else { "E" })
}

/// Distance along the ground in metres (haversine).
fn metres(a: P, b: P) -> f64 {
    let r = 6_371_000.0;
    let (la1, la2) = (a[1].to_radians(), b[1].to_radians());
    let dla = la2 - la1;
    let dlo = (b[0] - a[0]).to_radians();
    let h = (dla / 2.0).sin().powi(2) + la1.cos() * la2.cos() * (dlo / 2.0).sin().powi(2);
    2.0 * r * h.sqrt().asin()
}

fn distance(m: f64) -> String {
    if m < 1000.0 { format!("{m:.0} m") } else { format!("{:.2} km ({:.2} miles)", m / 1000.0, m / 1609.344) }
}

/// "1 line", "3 lines".
fn count(n: usize, plural: &str) -> String {
    format!("{} {}", crate::inspect::group(n as u64), if n == 1 { plural.trim_end_matches('s') } else { plural })
}

fn duration(secs: i64) -> String {
    let (h, m, s) = (secs / 3600, secs / 60 % 60, secs % 60);
    if h > 0 { format!("{h} h {m:02} min") } else { format!("{m} min {s:02} s") }
}

impl Geo {
    fn all(&self) -> impl Iterator<Item = &P> {
        self.points.iter().chain(self.lines.iter().flatten()).chain(self.areas.iter().flatten().flatten())
    }

    fn bounds(&self) -> Option<(P, P)> {
        let mut it = self.all();
        let first = *it.next()?;
        Some(it.fold((first, first), |(lo, hi), p| ([lo[0].min(p[0]), lo[1].min(p[1])], [hi[0].max(p[0]), hi[1].max(p[1])])))
    }

    fn room(&mut self, n: usize) -> bool {
        self.count += n;
        self.count <= POINTS_MAX
    }

    fn add_point(&mut self, p: P) {
        if self.room(1) {
            self.points.push(p);
        }
    }

    fn add_line(&mut self, l: Vec<P>) {
        if !l.is_empty() && self.room(l.len()) {
            self.lines.push(l);
        }
    }

    fn add_area(&mut self, a: Vec<Vec<P>>) {
        let n = a.iter().map(Vec::len).sum();
        if n > 0 && self.room(n) {
            self.areas.push(a);
        }
    }

    /// Width over height of the shapes as drawn.
    pub fn aspect(&self) -> f64 {
        let Some((lo, hi)) = self.bounds() else { return 1.5 };
        let squash = if self.planar { 1.0 } else { ((lo[1] + hi[1]) / 2.0).to_radians().cos().max(0.05) };
        let (w, h) = ((hi[0] - lo[0]) * squash, hi[1] - lo[1]);
        if w <= 0.0 || h <= 0.0 { 1.5 } else { (w / h).clamp(0.5, 3.0) }
    }

    /// A size for the picture, no wider or taller than `max`.
    pub fn fit(&self, max: u32) -> (u32, u32) {
        let a = self.aspect();
        if a >= 1.0 { (max, ((max as f64 / a) as u32).max(16)) } else { (((max as f64 * a) as u32).max(16), max) }
    }

    /// The shapes drawn on a plain background, `w` by `h`, as RGBA.
    pub fn draw(&self, w: u32, h: u32) -> Vec<u8> {
        let Some(mut pix) = tiny_skia::Pixmap::new(w.max(1), h.max(1)) else { return vec![0; (w * h * 4) as usize] };
        pix.fill(tiny_skia::Color::from_rgba8(24, 32, 42, 255));
        let Some((lo, hi)) = self.bounds() else { return pix.take() };
        let squash = if self.planar { 1.0 } else { ((lo[1] + hi[1]) / 2.0).to_radians().cos().max(0.05) };
        let margin = (w.min(h) as f64 * 0.06).max(4.0);
        let (sw, sh) = (((hi[0] - lo[0]) * squash).max(1e-9), (hi[1] - lo[1]).max(1e-9));
        let scale = ((w as f64 - 2.0 * margin) / sw).min((h as f64 - 2.0 * margin) / sh);
        // Centred.
        let (ox, oy) = ((w as f64 - sw * scale) / 2.0, (h as f64 - sh * scale) / 2.0);
        let to = |p: P| -> (f32, f32) { ((ox + (p[0] - lo[0]) * squash * scale) as f32, (h as f64 - oy - (p[1] - lo[1]) * scale) as f32) };
        let ident = tiny_skia::Transform::identity();
        let paint = |r, g, b, a| {
            let mut p = tiny_skia::Paint::default();
            p.set_color_rgba8(r, g, b, a);
            p.anti_alias = true;
            p
        };
        // A faint grid every round number of degrees.
        if !self.planar {
            let span = (hi[0] - lo[0]).max(hi[1] - lo[1]).max(1e-6);
            let step = [0.0005, 0.001, 0.002, 0.005, 0.01, 0.02, 0.05, 0.1, 0.2, 0.5, 1.0, 2.0, 5.0, 10.0, 15.0, 30.0, 45.0]
                .into_iter()
                .find(|s| span / s <= 8.0)
                .unwrap_or(90.0);
            let stroke = tiny_skia::Stroke { width: 1.0, ..Default::default() };
            let grid = paint(255, 255, 255, 18);
            let (lon0, lon1) = (lo[0] - (w as f64 / scale / squash), hi[0] + (w as f64 / scale / squash));
            let (lat0, lat1) = (lo[1] - h as f64 / scale, hi[1] + h as f64 / scale);
            let mut pb = tiny_skia::PathBuilder::new();
            let mut v = (lon0 / step).floor() * step;
            while v <= lon1 {
                let (x, _) = to([v, 0.0]);
                pb.move_to(x, 0.0);
                pb.line_to(x, h as f32);
                v += step;
            }
            let mut v = (lat0 / step).floor() * step;
            while v <= lat1 {
                let (_, y) = to([0.0, v]);
                pb.move_to(0.0, y);
                pb.line_to(w as f32, y);
                v += step;
            }
            if let Some(path) = pb.finish() {
                pix.stroke_path(&path, &grid, &stroke, ident, None);
            }
        }
        let line_w = (w.min(h) as f32 / 150.0).clamp(1.5, 8.0);
        // Areas: filled, then outlined.
        for area in &self.areas {
            let mut pb = tiny_skia::PathBuilder::new();
            for ring in area {
                for (i, p) in ring.iter().enumerate() {
                    let (x, y) = to(*p);
                    if i == 0 { pb.move_to(x, y) } else { pb.line_to(x, y) }
                }
                pb.close();
            }
            if let Some(path) = pb.finish() {
                pix.fill_path(&path, &paint(64, 190, 170, 70), tiny_skia::FillRule::EvenOdd, ident, None);
                pix.stroke_path(&path, &paint(96, 214, 196, 230), &tiny_skia::Stroke { width: line_w * 0.7, ..Default::default() }, ident, None);
            }
        }
        // Lines (tracks, routes, roads).
        let stroke = tiny_skia::Stroke { width: line_w, line_cap: tiny_skia::LineCap::Round, line_join: tiny_skia::LineJoin::Round, ..Default::default() };
        for line in &self.lines {
            let mut pb = tiny_skia::PathBuilder::new();
            let mut last = (f32::NAN, f32::NAN);
            for (i, p) in line.iter().enumerate() {
                let (x, y) = to(*p);
                // Points closer than a third of a pixel add nothing.
                if i > 0 && (x - last.0).abs() < 0.33 && (y - last.1).abs() < 0.33 && i + 1 < line.len() {
                    continue;
                }
                if i == 0 { pb.move_to(x, y) } else { pb.line_to(x, y) }
                last = (x, y);
            }
            if line.len() == 1 {
                let (x, y) = to(line[0]);
                pb.line_to(x + 0.01, y);
            }
            if let Some(path) = pb.finish() {
                pix.stroke_path(&path, &paint(255, 159, 67, 240), &stroke, ident, None);
            }
        }
        // A track's start and end.
        if self.lines.len() <= 20 {
            for line in &self.lines {
                if let (Some(a), Some(b)) = (line.first(), line.last())
                    && line.len() > 1
                {
                    for (p, (r, g, bl)) in [(*a, (80, 200, 90)), (*b, (230, 70, 70))] {
                        let (x, y) = to(p);
                        if let Some(c) = tiny_skia::PathBuilder::from_circle(x, y, line_w * 2.2) {
                            pix.fill_path(&c, &paint(r, g, bl, 255), tiny_skia::FillRule::Winding, ident, None);
                            pix.stroke_path(&c, &paint(255, 255, 255, 220), &tiny_skia::Stroke { width: 1.0, ..Default::default() }, ident, None);
                        }
                    }
                }
            }
        }
        // Points (places, waypoints): smaller when there are many.
        let r = if self.points.len() > 2000 { line_w * 0.7 } else { line_w * 1.8 };
        for p in &self.points {
            let (x, y) = to(*p);
            if let Some(c) = tiny_skia::PathBuilder::from_circle(x, y, r) {
                pix.fill_path(&c, &paint(250, 90, 120, 235), tiny_skia::FillRule::Winding, ident, None);
                if self.points.len() <= 2000 {
                    pix.stroke_path(&c, &paint(255, 255, 255, 200), &tiny_skia::Stroke { width: 1.0, ..Default::default() }, ident, None);
                }
            }
        }
        // Opaque, so premultiplied and straight alpha are the same.
        pix.take()
    }
}

// ---- GeoJSON ----

fn geojson(text: &str) -> Result<Geo, String> {
    let v: serde_json::Value = serde_json::from_str(text).map_err(|e| format!("Not valid GeoJSON: {e}"))?;
    let mut g = Geo::default();
    let mut features = 0usize;
    let mut kinds: Vec<(String, usize)> = Vec::new();
    let mut props: Vec<String> = Vec::new();
    fn pos(v: &serde_json::Value) -> Option<P> {
        let a = v.as_array()?;
        Some([a.first()?.as_f64()?, a.get(1)?.as_f64()?])
    }
    fn ring(v: &serde_json::Value) -> Vec<P> {
        v.as_array().map(|a| a.iter().filter_map(pos).collect()).unwrap_or_default()
    }
    fn geometry(v: &serde_json::Value, g: &mut Geo, kinds: &mut Vec<(String, usize)>, depth: usize) {
        if depth > 16 {
            return;
        }
        let t = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
        let c = v.get("coordinates").unwrap_or(&serde_json::Value::Null);
        match t {
            "Point" => g.add_point(pos(c).unwrap_or([f64::NAN; 2])),
            "MultiPoint" => ring(c).into_iter().for_each(|p| g.add_point(p)),
            "LineString" => g.add_line(ring(c)),
            "MultiLineString" => c.as_array().into_iter().flatten().for_each(|l| g.add_line(ring(l))),
            "Polygon" => g.add_area(c.as_array().into_iter().flatten().map(ring).collect()),
            "MultiPolygon" => c.as_array().into_iter().flatten().for_each(|p| g.add_area(p.as_array().into_iter().flatten().map(ring).collect())),
            "GeometryCollection" => v.get("geometries").and_then(|x| x.as_array()).into_iter().flatten().for_each(|x| geometry(x, g, kinds, depth + 1)),
            _ => return,
        }
        let name = match t {
            "Point" | "MultiPoint" => "points",
            "LineString" | "MultiLineString" => "lines",
            "Polygon" | "MultiPolygon" => "areas",
            _ => return,
        };
        match kinds.iter_mut().find(|k| k.0 == name) {
            Some(k) => k.1 += 1,
            None => kinds.push((name.into(), 1)),
        }
    }
    let mut feature = |f: &serde_json::Value, g: &mut Geo| {
        features += 1;
        if let Some(geom) = f.get("geometry") {
            geometry(geom, g, &mut kinds, 0);
        }
        if props.is_empty()
            && let Some(p) = f.get("properties").and_then(|p| p.as_object())
        {
            props = p.keys().take(8).cloned().collect();
        }
    };
    match v.get("type").and_then(|t| t.as_str()) {
        Some("FeatureCollection") => {
            for f in v.get("features").and_then(|f| f.as_array()).into_iter().flatten() {
                feature(f, &mut g);
            }
        }
        Some("Feature") => feature(&v, &mut g),
        Some("Topology") => return Err("TopoJSON isn't drawn yet".into()),
        Some(_) => geometry(&v, &mut g, &mut kinds, 0),
        None => return Err("Not GeoJSON: it has no \"type\"".into()),
    }
    g.points.retain(|p| p[0].is_finite() && p[1].is_finite());
    if let Some(n) = v.get("name").and_then(|n| n.as_str()) {
        g.facts.push(("Name", n.to_string()));
    }
    if features > 0 {
        g.facts.push(("Features", crate::inspect::group(features as u64)));
    }
    if !kinds.is_empty() {
        g.facts.push(("Shapes", kinds.iter().map(|(k, n)| count(*n, k)).collect::<Vec<_>>().join(", ")));
    }
    if !props.is_empty() {
        g.facts.push(("Properties", props.join(", ")));
    }
    Ok(g)
}

// ---- GPX ----

fn xml(text: &str) -> Result<roxmltree::Document<'_>, String> {
    roxmltree::Document::parse_with_options(text, roxmltree::ParsingOptions { allow_dtd: false, nodes_limit: 20_000_000, ..Default::default() }).map_err(|e| format!("Not valid XML: {e}"))
}

fn child<'a>(n: roxmltree::Node<'a, 'a>, name: &str) -> Option<roxmltree::Node<'a, 'a>> {
    n.children().find(|c| c.is_element() && c.tag_name().name() == name)
}

fn child_text(n: roxmltree::Node, name: &str) -> Option<String> {
    child(n, name).and_then(|c| c.text()).map(|t| t.trim().to_string()).filter(|t| !t.is_empty())
}

fn gpx(text: &str) -> Result<Geo, String> {
    let doc = xml(text)?;
    let root = doc.root_element();
    if root.tag_name().name() != "gpx" {
        return Err("Not a GPX file".into());
    }
    let mut g = Geo::default();
    let pt = |n: roxmltree::Node| -> Option<P> { Some([n.attribute("lon")?.trim().parse().ok()?, n.attribute("lat")?.trim().parse().ok()?]) };
    let (mut tracks, mut routes, mut waypoints) = (0, 0, 0);
    let mut eles: Vec<f64> = Vec::new();
    let mut times: Vec<chrono::DateTime<chrono::FixedOffset>> = Vec::new();
    let mut dist = 0.0;
    let mut climb = 0.0;
    let mut trk_name = None;
    for n in root.children().filter(|c| c.is_element()) {
        match n.tag_name().name() {
            "wpt" => {
                waypoints += 1;
                if let Some(p) = pt(n) {
                    g.add_point(p);
                }
            }
            "trk" | "rte" => {
                if n.tag_name().name() == "trk" {
                    tracks += 1;
                } else {
                    routes += 1;
                }
                if trk_name.is_none() {
                    trk_name = child_text(n, "name");
                }
                // A track's segments, or a route's points directly.
                let segs: Vec<roxmltree::Node> = if n.tag_name().name() == "trk" { n.children().filter(|c| c.is_element() && c.tag_name().name() == "trkseg").collect() } else { vec![n] };
                for seg in segs {
                    let mut line = Vec::new();
                    // Climb counts rises of over 3 m, so GPS jitter doesn't add up.
                    let mut base: Option<f64> = None;
                    for p in seg.children().filter(|c| c.is_element() && matches!(c.tag_name().name(), "trkpt" | "rtept")) {
                        let Some(xy) = pt(p) else { continue };
                        if let Some(prev) = line.last() {
                            dist += metres(*prev, xy);
                        }
                        line.push(xy);
                        if let Some(e) = child_text(p, "ele").and_then(|e| e.parse::<f64>().ok()) {
                            eles.push(e);
                            match base {
                                Some(b) if e - b > 3.0 => {
                                    climb += e - b;
                                    base = Some(e);
                                }
                                Some(b) if e < b => base = Some(e),
                                None => base = Some(e),
                                _ => {}
                            }
                        }
                        if let Some(t) = child_text(p, "time").and_then(|t| chrono::DateTime::parse_from_rfc3339(&t).ok()) {
                            times.push(t);
                        }
                    }
                    g.add_line(line);
                }
            }
            _ => {}
        }
    }
    let name = child(root, "metadata").and_then(|m| child_text(m, "name")).or(trk_name);
    if let Some(n) = name {
        g.facts.push(("Name", n));
    }
    let count = |n: usize, one: &str, many: &str| format!("{} {}", crate::inspect::group(n as u64), if n == 1 { one } else { many });
    let mut what = Vec::new();
    if tracks > 0 {
        what.push(count(tracks, "track", "tracks"));
    }
    if routes > 0 {
        what.push(count(routes, "route", "routes"));
    }
    if waypoints > 0 {
        what.push(count(waypoints, "waypoint", "waypoints"));
    }
    if !what.is_empty() {
        g.facts.push(("Contains", what.join(", ")));
    }
    let pts: usize = g.lines.iter().map(Vec::len).sum();
    if pts > 0 {
        g.facts.push(("Points", crate::inspect::group(pts as u64)));
        g.facts.push(("Distance", distance(dist)));
    }
    if let (Some(lo), Some(hi)) = (eles.iter().copied().reduce(f64::min), eles.iter().copied().reduce(f64::max)) {
        g.facts.push(("Climb", format!("{climb:.0} m")));
        g.facts.push(("Height", format!("{lo:.0} m to {hi:.0} m")));
    }
    if let (Some(a), Some(b)) = (times.iter().min(), times.iter().max()) {
        g.facts.push(("Started", a.format("%a %-d %b %Y, %H:%M").to_string()));
        let secs = (*b - *a).num_seconds();
        if secs > 0 {
            g.facts.push(("Time", duration(secs)));
            if dist > 0.0 {
                g.facts.push(("Average speed", format!("{:.1} km/h", dist / 1000.0 / (secs as f64 / 3600.0))));
            }
        }
    }
    if let Some(creator) = root.attribute("creator").filter(|c| !c.trim().is_empty()) {
        g.facts.push(("Made by", creator.trim().to_string()));
    }
    Ok(g)
}

// ---- KML ----

fn kml_coords(s: &str) -> Vec<P> {
    s.split_whitespace()
        .filter_map(|t| {
            let mut it = t.split(',');
            Some([it.next()?.parse().ok()?, it.next()?.parse().ok()?])
        })
        .collect()
}

fn kml(text: &str) -> Result<Geo, String> {
    let doc = xml(text)?;
    if doc.root_element().tag_name().name() != "kml" {
        return Err("Not a KML file".into());
    }
    let mut g = Geo::default();
    let (mut placemarks, mut folders, mut overlays) = (0, 0, 0);
    let mut dist = 0.0;
    let coords = |n: roxmltree::Node| n.descendants().find(|c| c.tag_name().name() == "coordinates").and_then(|c| c.text()).map(kml_coords).unwrap_or_default();
    for n in doc.descendants().filter(|n| n.is_element()) {
        match n.tag_name().name() {
            "Placemark" => placemarks += 1,
            "Folder" => folders += 1,
            "GroundOverlay" | "ScreenOverlay" | "PhotoOverlay" => overlays += 1,
            "Point" => {
                if let Some(p) = coords(n).first() {
                    g.add_point(*p);
                }
            }
            "LineString" => {
                let l = coords(n);
                dist += l.windows(2).map(|w| metres(w[0], w[1])).sum::<f64>();
                g.add_line(l);
            }
            "Polygon" => {
                let rings = n
                    .children()
                    .filter(|c| c.is_element() && matches!(c.tag_name().name(), "outerBoundaryIs" | "innerBoundaryIs"))
                    .map(coords)
                    .collect();
                g.add_area(rings);
            }
            // gx:Track: <gx:coord>lon lat alt</gx:coord>
            "Track" => {
                let l: Vec<P> = n
                    .children()
                    .filter(|c| c.is_element() && c.tag_name().name() == "coord")
                    .filter_map(|c| {
                        let mut it = c.text()?.split_whitespace();
                        Some([it.next()?.parse().ok()?, it.next()?.parse().ok()?])
                    })
                    .collect();
                dist += l.windows(2).map(|w| metres(w[0], w[1])).sum::<f64>();
                g.add_line(l);
            }
            _ => {}
        }
    }
    let name = doc.descendants().find(|n| n.tag_name().name() == "Document").and_then(|d| child_text(d, "name"));
    if let Some(n) = name {
        g.facts.push(("Name", n));
    }
    g.facts.push(("Placemarks", crate::inspect::group(placemarks as u64)));
    if folders > 0 {
        g.facts.push(("Folders", crate::inspect::group(folders as u64)));
    }
    let mut shapes = Vec::new();
    for (n, what) in [(g.points.len(), "points"), (g.lines.len(), "lines"), (g.areas.len(), "areas")] {
        if n > 0 {
            shapes.push(count(n, what));
        }
    }
    if !shapes.is_empty() {
        g.facts.push(("Shapes", shapes.join(", ")));
    }
    if dist > 0.0 {
        g.facts.push(("Length of lines", distance(dist)));
    }
    if overlays > 0 {
        g.facts.push(("Overlays", format!("{overlays} (pictures laid over the map, not shown)")));
    }
    Ok(g)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fact<'a>(g: &'a Geo, k: &str) -> Option<&'a str> {
        g.facts.iter().find(|f| f.0 == k).map(|f| f.1.as_str())
    }

    #[test]
    fn geojson_features() {
        let g = geojson(r#"{"type":"FeatureCollection","name":"Parks","features":[
            {"type":"Feature","properties":{"name":"A","area":3},"geometry":{"type":"Polygon","coordinates":[[[-1.26,51.75],[-1.25,51.75],[-1.25,51.76],[-1.26,51.75]]]}},
            {"type":"Feature","properties":{},"geometry":{"type":"Point","coordinates":[-1.255,51.752]}},
            {"type":"Feature","properties":{},"geometry":{"type":"MultiLineString","coordinates":[[[-1.27,51.74],[-1.24,51.77]],[[0,0]]]}},
            {"type":"Feature","properties":{},"geometry":null}
        ]}"#)
        .unwrap();
        assert_eq!(fact(&g, "Name"), Some("Parks"));
        assert_eq!(fact(&g, "Features"), Some("4"));
        assert_eq!(fact(&g, "Shapes"), Some("1 area, 1 point, 1 line"));
        assert_eq!(fact(&g, "Properties"), Some("area, name"));
        assert_eq!((g.points.len(), g.lines.len(), g.areas.len()), (1, 2, 1));
        let (w, h) = g.fit(400);
        let rgba = g.draw(w, h);
        assert_eq!(rgba.len(), (w * h * 4) as usize);
        // Something other than the background was drawn.
        assert!(rgba.chunks(4).any(|c| c[0] > 200));
    }

    #[test]
    fn gpx_track_distance_climb_and_time() {
        let text = r#"<?xml version="1.0"?><gpx version="1.1" creator="Watch"><metadata><name>Morning run</name></metadata>
            <wpt lat="51.75" lon="-1.25"><name>Start</name></wpt>
            <trk><name>Run</name><trkseg>
              <trkpt lat="51.7500" lon="-1.2500"><ele>60</ele><time>2026-11-26T07:00:00Z</time></trkpt>
              <trkpt lat="51.7590" lon="-1.2500"><ele>61</ele><time>2026-11-26T07:03:00Z</time></trkpt>
              <trkpt lat="51.7680" lon="-1.2500"><ele>75</ele><time>2026-11-26T07:06:00Z</time></trkpt>
              <trkpt lat="51.7770" lon="-1.2500"><ele>70</ele><time>2026-11-26T07:10:00Z</time></trkpt>
            </trkseg></trk></gpx>"#;
        let g = gpx(text).unwrap();
        assert_eq!(fact(&g, "Name"), Some("Morning run"));
        assert_eq!(fact(&g, "Contains"), Some("1 track, 1 waypoint"));
        assert_eq!(fact(&g, "Points"), Some("4"));
        // 0.027° of latitude is about 3 km.
        assert!(fact(&g, "Distance").unwrap().starts_with("3.00 km"), "{:?}", fact(&g, "Distance"));
        assert_eq!(fact(&g, "Climb"), Some("15 m"));
        assert_eq!(fact(&g, "Height"), Some("60 m to 75 m"));
        assert_eq!(fact(&g, "Time"), Some("10 min 00 s"));
        assert_eq!(fact(&g, "Made by"), Some("Watch"));
    }

    #[test]
    fn kml_placemarks() {
        let text = r#"<kml xmlns="http://www.opengis.net/kml/2.2" xmlns:gx="http://www.google.com/kml/ext/2.2"><Document><name>Trip</name><Folder>
            <Placemark><Point><coordinates>2.29,48.85,0</coordinates></Point></Placemark>
            <Placemark><LineString><coordinates>2.29,48.85 2.35,48.86</coordinates></LineString></Placemark>
            <Placemark><Polygon><outerBoundaryIs><LinearRing><coordinates>2.3,48.8 2.4,48.8 2.4,48.9 2.3,48.8</coordinates></LinearRing></outerBoundaryIs>
              <innerBoundaryIs><LinearRing><coordinates>2.32,48.82 2.33,48.82 2.33,48.83 2.32,48.82</coordinates></LinearRing></innerBoundaryIs></Polygon></Placemark>
            <Placemark><gx:Track><gx:coord>2.1 48.1 0</gx:coord><gx:coord>2.2 48.2 0</gx:coord></gx:Track></Placemark>
            </Folder></Document></kml>"#;
        let g = kml(text).unwrap();
        assert_eq!(fact(&g, "Name"), Some("Trip"));
        assert_eq!(fact(&g, "Placemarks"), Some("4"));
        assert_eq!(fact(&g, "Shapes"), Some("1 point, 2 lines, 1 area"));
        assert_eq!(g.areas[0].len(), 2);
    }

    #[test]
    fn projected_coordinates_draw_flat() {
        let dir = std::env::temp_dir().join(format!("filescape-geo-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("site.geojson");
        std::fs::write(&p, r#"{"type":"LineString","coordinates":[[530000,180000],[531000,181500]]}"#).unwrap();
        let g = read(&p).unwrap();
        assert!(g.planar);
        assert!(fact(&g, "Covers").is_none());
        let _ = g.draw(100, 100);
        std::fs::write(&p, r#"{"type":"FeatureCollection","features":[]}"#).unwrap();
        assert!(read(&p).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn broken_input_does_not_panic() {
        for t in ["", "{", "[]", r#"{"type":"Polygon","coordinates":[[["a"]]]}"#, r#"{"type":"Point","coordinates":[1]}"#] {
            let _ = geojson(t);
        }
        assert!(gpx("<gpx><trk><trkseg><trkpt lat='x'/></trkseg></trk></gpx>").is_ok());
        assert!(kml("<html/>").is_err());
        let g = Geo { points: vec![[1.0, 1.0]], count: 1, ..Default::default() };
        assert_eq!(g.draw(10, 10).len(), 400);
    }
}
