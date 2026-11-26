//! Line icons drawn with the painter, so they're crisp at any scale, take the text colour of the
//! theme, and don't depend on which symbols a font happens to have. Each is drawn in a square on
//! a 16-unit grid with one stroke width, so they read as one family.

use eframe::egui::{self, Color32, Pos2, Rect, Shape, Stroke, pos2, vec2};
use std::f32::consts::{PI, TAU};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Icon {
    Gear,
    Filter,
    List,
    Grid,
    Sliders,
    Key,
    Search,
    Help,
    Palette,
    Drive,
    Refresh,
    Folder,
    File,
    Clock,
    Ruler,
    Cpu,
    Shield,
    Warning,
    Info,
    Check,
    Close,
    Reset,
    Plus,
    Trash,
    Clipboard,
    Compass,
    Sun,
    Wrench,
    Lock,
    Pencil,
    External,
    Copy,
    Columns,
    /// Online only: a cloud outline.
    Cloud,
    /// Downloaded: a tick in a circle.
    CircleCheck,
    /// Always kept on this device: a filled circle with the tick cut out.
    Pinned,
    /// Partly downloaded: a cloud with its lower part filled.
    CloudHalf,
    /// Local copy unknown: a cloud with a question mark.
    CloudQuestion,
    ArrowLeft,
    ArrowRight,
    ArrowUp,
    Home,
    /// Zoom to the selection: a crosshair.
    Target,
    /// The treemap view: nested rectangles.
    Treemap,
    /// The sunburst view: a centre and a ring.
    Sunburst,
    /// Display options: an eye.
    Eye,
    ChevronDown,
    /// The size measure: stacked layers.
    Layers,
    /// More controls (overflow menu): three dots.
    More,
}

/// Draw `icon` filling `rect` (best square), in `color`.
pub fn paint(p: &egui::Painter, rect: Rect, icon: Icon, color: Color32) {
    let side = rect.width().min(rect.height());
    let r = Rect::from_center_size(rect.center(), vec2(side, side));
    let u = side / 16.0;
    // Map a point on the 16-unit grid into the rect.
    let at = |x: f32, y: f32| pos2(r.min.x + x * u, r.min.y + y * u);
    let st = Stroke::new((1.35 * u).max(1.0), color);
    let line = |a: (f32, f32), b: (f32, f32)| p.line_segment([at(a.0, a.1), at(b.0, b.1)], st);
    let poly = |pts: &[(f32, f32)], closed: bool| {
        let v: Vec<Pos2> = pts.iter().map(|&(x, y)| at(x, y)).collect();
        if closed { p.add(Shape::closed_line(v, st)) } else { p.add(Shape::line(v, st)) };
    };
    let circle = |c: (f32, f32), rad: f32| p.circle_stroke(at(c.0, c.1), rad * u, st);
    let dot = |c: (f32, f32), rad: f32| p.circle_filled(at(c.0, c.1), rad * u, color);
    let rrect = |x0: f32, y0: f32, x1: f32, y1: f32, rad: f32| {
        p.rect_stroke(Rect::from_min_max(at(x0, y0), at(x1, y1)), rad * u, st, egui::StrokeKind::Middle)
    };
    let arc = |c: (f32, f32), rad: f32, a0: f32, a1: f32| {
        let n = 18;
        let v: Vec<Pos2> = (0..=n).map(|i| {
            let a = a0 + (a1 - a0) * i as f32 / n as f32;
            at(c.0 + rad * a.cos(), c.1 + rad * a.sin())
        }).collect();
        p.add(Shape::line(v, st));
    };
    match icon {
        Icon::Gear => {
            // A ring with eight square teeth.
            let c = (8.0, 8.0);
            let mut pts = Vec::new();
            for i in 0..8 {
                let a = i as f32 / 8.0 * TAU;
                for (da, rad) in [(-0.30, 5.0), (-0.17, 6.8), (0.17, 6.8), (0.30, 5.0)] {
                    pts.push((c.0 + rad * (a + da).cos(), c.1 + rad * (a + da).sin()));
                }
            }
            poly(&pts, true);
            circle(c, 2.2);
        }
        Icon::Filter => poly(&[(2.0, 3.0), (14.0, 3.0), (9.5, 8.5), (9.5, 13.5), (6.5, 12.0), (6.5, 8.5)], true),
        Icon::List => {
            for y in [4.0, 8.0, 12.0] {
                dot((2.8, y), 0.9);
                line((5.5, y), (14.0, y));
            }
        }
        Icon::Grid => {
            rrect(2.0, 2.0, 7.0, 7.0, 1.0);
            rrect(9.0, 2.0, 14.0, 7.0, 1.0);
            rrect(2.0, 9.0, 7.0, 14.0, 1.0);
            rrect(9.0, 9.0, 14.0, 14.0, 1.0);
        }
        Icon::Sliders => {
            for (y, k) in [(4.0, 10.0), (8.0, 5.0), (12.0, 8.5)] {
                line((2.0, y), (14.0, y));
                p.circle_filled(at(k, y), 1.9 * u, color);
            }
        }
        Icon::Key => {
            circle((5.0, 11.0), 3.0);
            line((7.2, 8.8), (13.5, 2.5));
            line((11.0, 5.0), (13.0, 7.0));
            line((12.6, 3.4), (14.4, 5.2));
        }
        Icon::Search => {
            circle((7.0, 7.0), 4.5);
            line((10.3, 10.3), (14.0, 14.0));
        }
        Icon::Help => {
            circle((8.0, 8.0), 6.3);
            arc((8.0, 6.4), 2.0, PI, TAU + 0.6);
            line((9.5, 7.7), (8.0, 9.0));
            line((8.0, 9.0), (8.0, 10.0));
            dot((8.0, 12.0), 0.85);
        }
        Icon::Palette => {
            arc((8.0, 8.0), 6.2, 0.2, TAU - 0.9);
            poly(&[(13.3, 5.0), (11.0, 7.4), (12.6, 9.6), (14.2, 9.0)], false);
            dot((5.0, 6.0), 1.0);
            dot((8.0, 4.4), 1.0);
            dot((5.0, 10.0), 1.0);
            dot((8.5, 11.6), 1.0);
        }
        Icon::Drive => {
            rrect(1.5, 4.0, 14.5, 12.0, 2.0);
            line((1.5, 8.6), (14.5, 8.6));
            dot((11.5, 10.4), 0.8);
            dot((9.2, 10.4), 0.8);
        }
        Icon::Refresh => {
            arc((8.0, 8.0), 5.5, -PI * 0.15, PI * 0.95);
            arc((8.0, 8.0), 5.5, PI * 0.85, PI * 1.95);
            poly(&[(13.4, 3.2), (13.4, 6.6), (10.0, 6.6)], false);
            poly(&[(2.6, 12.8), (2.6, 9.4), (6.0, 9.4)], false);
        }
        Icon::Folder => poly(&[(1.5, 4.0), (6.0, 4.0), (7.5, 5.6), (14.5, 5.6), (14.5, 13.0), (1.5, 13.0)], true),
        Icon::File => {
            poly(&[(3.5, 1.5), (9.5, 1.5), (12.5, 4.5), (12.5, 14.5), (3.5, 14.5)], true);
            poly(&[(9.5, 1.5), (9.5, 4.5), (12.5, 4.5)], false);
        }
        Icon::Clock => {
            circle((8.0, 8.0), 6.3);
            poly(&[(8.0, 4.2), (8.0, 8.0), (10.8, 9.6)], false);
        }
        Icon::Ruler => {
            rrect(1.5, 5.0, 14.5, 11.0, 1.0);
            for (i, x) in [4.0, 6.5, 9.0, 11.5].iter().enumerate() {
                line((*x, 5.0), (*x, if i % 2 == 0 { 8.4 } else { 7.2 }));
            }
        }
        Icon::Cpu => {
            rrect(4.0, 4.0, 12.0, 12.0, 1.2);
            rrect(6.4, 6.4, 9.6, 9.6, 0.4);
            for k in [6.0, 8.0, 10.0] {
                line((k, 1.6), (k, 4.0));
                line((k, 12.0), (k, 14.4));
                line((1.6, k), (4.0, k));
                line((12.0, k), (14.4, k));
            }
        }
        Icon::Shield => {
            poly(&[(8.0, 1.5), (13.5, 3.5), (13.0, 9.0), (8.0, 14.5), (3.0, 9.0), (2.5, 3.5)], true);
        }
        Icon::Warning => {
            poly(&[(8.0, 1.8), (14.8, 13.8), (1.2, 13.8)], true);
            line((8.0, 6.0), (8.0, 9.6));
            dot((8.0, 11.8), 0.85);
        }
        Icon::Info => {
            circle((8.0, 8.0), 6.3);
            line((8.0, 7.2), (8.0, 11.5));
            dot((8.0, 4.9), 0.85);
        }
        Icon::Check => poly(&[(2.8, 8.4), (6.4, 12.0), (13.4, 4.2)], false),
        Icon::Close => {
            line((3.5, 3.5), (12.5, 12.5));
            line((12.5, 3.5), (3.5, 12.5));
        }
        Icon::Reset => {
            arc((8.5, 8.5), 5.2, PI * 1.05, PI * 2.75);
            poly(&[(2.5, 4.2), (3.3, 8.2), (7.0, 6.8)], false);
        }
        Icon::Plus => {
            line((8.0, 2.5), (8.0, 13.5));
            line((2.5, 8.0), (13.5, 8.0));
        }
        Icon::Trash => {
            line((2.0, 4.0), (14.0, 4.0));
            poly(&[(6.0, 4.0), (6.5, 2.0), (9.5, 2.0), (10.0, 4.0)], false);
            poly(&[(3.5, 4.0), (4.5, 14.2), (11.5, 14.2), (12.5, 4.0)], false);
            line((6.8, 6.8), (6.8, 11.6));
            line((9.2, 6.8), (9.2, 11.6));
        }
        Icon::Clipboard => {
            rrect(3.0, 3.0, 13.0, 14.6, 1.4);
            rrect(5.6, 1.4, 10.4, 4.4, 0.8);
            line((5.6, 8.0), (10.4, 8.0));
            line((5.6, 11.0), (9.0, 11.0));
        }
        Icon::Compass => {
            circle((8.0, 8.0), 6.3);
            poly(&[(10.8, 5.2), (9.0, 9.0), (5.2, 10.8), (7.0, 7.0)], true);
        }
        Icon::Sun => {
            circle((8.0, 8.0), 2.8);
            for i in 0..8 {
                let a = i as f32 / 8.0 * TAU;
                line((8.0 + 4.6 * a.cos(), 8.0 + 4.6 * a.sin()), (8.0 + 6.6 * a.cos(), 8.0 + 6.6 * a.sin()));
            }
        }
        Icon::Wrench => {
            arc((10.8, 5.2), 3.4, PI * 0.95, PI * 2.55);
            line((8.2, 7.8), (2.6, 13.4));
        }
        Icon::Lock => {
            rrect(3.0, 7.0, 13.0, 14.5, 1.4);
            arc((8.0, 7.0), 3.0, PI, TAU);
            line((5.0, 7.0), (5.0, 7.0));
            dot((8.0, 10.8), 0.9);
        }
        Icon::Pencil => {
            poly(&[(11.0, 2.4), (13.6, 5.0), (5.4, 13.2), (2.4, 13.6), (2.8, 10.6)], true);
            line((9.4, 4.0), (12.0, 6.6));
        }
        Icon::External => {
            poly(&[(9.0, 2.5), (13.5, 2.5), (13.5, 7.0)], false);
            line((13.5, 2.5), (7.0, 9.0));
            poly(&[(11.5, 9.5), (11.5, 13.5), (2.5, 13.5), (2.5, 4.5), (6.5, 4.5)], false);
        }
        Icon::Copy => {
            rrect(5.5, 5.5, 14.0, 14.0, 1.4);
            poly(&[(10.5, 5.5), (10.5, 2.0), (2.0, 2.0), (2.0, 10.5), (5.5, 10.5)], false);
        }
        Icon::Columns => {
            rrect(1.5, 2.5, 14.5, 13.5, 1.4);
            line((6.0, 2.5), (6.0, 13.5));
            line((10.5, 2.5), (10.5, 13.5));
        }
        Icon::Cloud | Icon::CloudHalf | Icon::CloudQuestion => {
            if icon == Icon::CloudHalf {
                p.add(Shape::convex_polygon(
                    [(3.2, 10.6), (12.8, 10.6), (14.3, 11.4), (12.3, 12.9), (3.7, 12.9), (1.8, 11.6)].iter().map(|&(x, y)| at(x, y)).collect(),
                    color,
                    Stroke::NONE,
                ));
            }
            poly(&cloud_outline(), true);
            if icon == Icon::CloudQuestion {
                arc((8.0, 8.2), 1.3, PI * 1.0, PI * 2.4);
                line((8.0 + 1.3 * (PI * 2.4).cos(), 8.2 + 1.3 * (PI * 2.4).sin()), (8.0, 10.1));
                dot((8.0, 11.5), 0.6);
            }
        }
        Icon::CircleCheck => {
            circle((8.0, 8.0), 5.8);
            poly(&[(5.2, 8.2), (7.2, 10.2), (10.9, 6.0)], false);
        }
        Icon::Pinned => {
            p.circle_filled(at(8.0, 8.0), 6.5 * u, color);
            let cut = Stroke::new((1.6 * u).max(1.0), Color32::WHITE);
            let v: Vec<Pos2> = [(5.0, 8.2), (7.2, 10.4), (11.1, 5.9)].iter().map(|&(x, y)| at(x, y)).collect();
            p.add(Shape::line(v, cut));
        }
        Icon::ArrowLeft => {
            line((13.5, 8.0), (2.8, 8.0));
            poly(&[(7.0, 3.8), (2.8, 8.0), (7.0, 12.2)], false);
        }
        Icon::ArrowRight => {
            line((2.5, 8.0), (13.2, 8.0));
            poly(&[(9.0, 3.8), (13.2, 8.0), (9.0, 12.2)], false);
        }
        Icon::ArrowUp => {
            line((8.0, 13.5), (8.0, 2.8));
            poly(&[(3.8, 7.0), (8.0, 2.8), (12.2, 7.0)], false);
        }
        Icon::Home => {
            poly(&[(1.8, 8.2), (8.0, 2.4), (14.2, 8.2)], false);
            poly(&[(3.8, 6.6), (3.8, 13.6), (12.2, 13.6), (12.2, 6.6)], false);
            poly(&[(6.6, 13.6), (6.6, 9.8), (9.4, 9.8), (9.4, 13.6)], false);
        }
        Icon::Target => {
            circle((8.0, 8.0), 4.6);
            dot((8.0, 8.0), 1.1);
            line((8.0, 0.8), (8.0, 3.4));
            line((8.0, 12.6), (8.0, 15.2));
            line((0.8, 8.0), (3.4, 8.0));
            line((12.6, 8.0), (15.2, 8.0));
        }
        Icon::Treemap => {
            rrect(1.8, 2.5, 14.2, 13.5, 1.5);
            line((9.0, 2.5), (9.0, 13.5));
            line((1.8, 9.0), (9.0, 9.0));
            line((9.0, 6.5), (14.2, 6.5));
        }
        Icon::Sunburst => {
            circle((8.0, 8.0), 2.4);
            circle((8.0, 8.0), 6.2);
            line((8.0, 1.8), (8.0, 5.6));
            line((13.4, 11.1), (10.1, 9.2));
            line((2.6, 11.1), (5.9, 9.2));
        }
        Icon::Eye => {
            arc((8.0, 13.0), 7.6, PI * 1.18, PI * 1.82);
            arc((8.0, 3.0), 7.6, PI * 0.18, PI * 0.82);
            circle((8.0, 8.0), 2.2);
        }
        Icon::ChevronDown => poly(&[(4.0, 6.0), (8.0, 10.0), (12.0, 6.0)], false),
        Icon::Layers => {
            poly(&[(8.0, 2.5), (14.0, 5.5), (8.0, 8.5), (2.0, 5.5)], true);
            poly(&[(2.0, 8.3), (8.0, 11.3), (14.0, 8.3)], false);
            poly(&[(2.0, 11.0), (8.0, 14.0), (14.0, 11.0)], false);
        }
        Icon::More => {
            for x in [3.5, 8.0, 12.5] {
                dot((x, 8.0), 1.25);
            }
        }
    }
}

/// A cloud on the 16-unit grid: three bumps over a flat base.
fn cloud_outline() -> Vec<(f32, f32)> {
    let mut pts = vec![(4.0, 13.0)];
    let bump = |pts: &mut Vec<(f32, f32)>, c: (f32, f32), r: f32, a0: f32, a1: f32| {
        for i in 0..=8 {
            let a = (a0 + (a1 - a0) * i as f32 / 8.0).to_radians();
            pts.push((c.0 + r * a.cos(), c.1 + r * a.sin()));
        }
    };
    bump(&mut pts, (12.0, 10.4), 2.6, 90.0, -95.0);
    bump(&mut pts, (8.0, 7.9), 3.7, -20.0, -175.0);
    bump(&mut pts, (4.0, 10.5), 2.5, -110.0, -270.0);
    pts
}

/// Allocate an icon-sized space in the layout and draw `icon` there.
pub fn show(ui: &mut egui::Ui, icon: Icon, size: f32, color: Color32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(size, size), egui::Sense::hover());
    paint(ui.painter(), rect, icon, color);
    resp
}
