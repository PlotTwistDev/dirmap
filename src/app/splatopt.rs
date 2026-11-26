//! The storage optimiser card under a splat scene's preview: how big the scene would be in
//! each other format, measured by encoding it in memory, and converting it to one of them as
//! a new file next to the original, which is kept. The new file is read back and checked.
//! Converting one file is free; batch conversion is Pro (a later stage).

use super::*;
use crate::splat::write::{self, Target};
use crate::splat::{Limits, Scene};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// One format's measured size, or why it couldn't be made.
type Measured = Vec<(Target, Result<u64, String>)>;

#[derive(Default)]
struct State {
    /// Measuring or converting now.
    busy: Option<String>,
    measured: Option<Result<Measured, String>>,
    /// The save picker for a conversion, while it's open (a cancelled picker leaves it here,
    /// harmlessly, until another is opened).
    picking: Option<(Target, crate::asking::Mailbox)>,
    /// What the last conversion did.
    said: Option<Result<String, String>>,
}

type Shared = Arc<Mutex<State>>;

/// The scene whole, for writing: the preview's when it has everything, otherwise read again.
fn full_scene(path: &Path, shown: &Arc<Scene>) -> Result<Arc<Scene>, String> {
    if write::complete(shown) {
        return Ok(shown.clone());
    }
    let scene = crate::splat::read(path, Limits::CONVERT)?;
    match write::unwritable(&scene) {
        Some(why) => Err(why),
        None => Ok(Arc::new(scene)),
    }
}

fn measure(path: &Path, shown: &Arc<Scene>) -> Result<Measured, String> {
    let scene = full_scene(path, shown)?;
    Ok(Target::ALL.into_iter().filter(|t| !t.is_format_of(&scene)).map(|t| (t, write::encode(&scene, t).map(|b| b.len() as u64))).collect())
}

/// The card, for a splat scene on this computer.
pub(super) fn card(ui: &mut egui::Ui, path: &Path, scene: &Arc<Scene>, file_size: u64) {
    if scene.kind != crate::splat::SceneKind::Splat {
        return;
    }
    let id = egui::Id::new(("splat-optimiser", path));
    let st: Shared = ui.data_mut(|d| d.get_temp_mut_or_default::<Shared>(id).clone());
    ui.add_space(10.0);
    egui::Frame::new().fill(ui.visuals().faint_bg_color).stroke(ui.visuals().widgets.noninteractive.bg_stroke).corner_radius(4.0).inner_margin(8.0).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(egui::RichText::new("Storage optimiser").strong());
        ui.weak(format!("Now {} as {}.", crate::scan::human(file_size), scene.format));
        ui.add_space(4.0);
        let mut s = st.lock().unwrap();
        if let Some(what) = &s.busy {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(what.as_str());
            });
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(200));
        } else if s.measured.is_none() {
            if ui.button("Measure other formats").on_hover_text("Work out how big this scene would be in each format Filescape can write. Nothing is written yet.").clicked() {
                s.busy = Some("Measuring…".into());
                let (st2, p, sc, ctx) = (st.clone(), path.to_path_buf(), scene.clone(), ui.ctx().clone());
                std::thread::spawn(move || {
                    let m = measure(&p, &sc);
                    let mut s = st2.lock().unwrap();
                    s.measured = Some(m);
                    s.busy = None;
                    ctx.request_repaint();
                });
            }
        }
        match &s.measured {
            Some(Err(e)) => {
                ui.add(egui::Label::new(format!("Can't convert this scene: {e}")).wrap());
            }
            Some(Ok(rows)) => {
                let mut start: Option<Target> = None;
                egui::Grid::new(("splat-optimiser-rows", path)).num_columns(3).spacing([10.0, 4.0]).show(ui, |ui| {
                    for (t, size) in rows {
                        ui.label(t.name()).on_hover_text(t.about());
                        match size {
                            Ok(n) => {
                                let change = *n as f64 / file_size.max(1) as f64;
                                let note = if change < 0.995 {
                                    format!("{}, {:.0}% smaller", crate::scan::human(*n), (1.0 - change) * 100.0)
                                } else if change > 1.005 {
                                    format!("{}, {:.1}× the size", crate::scan::human(*n), change)
                                } else {
                                    format!("{}, about the same", crate::scan::human(*n))
                                };
                                ui.label(note);
                                if ui.add_enabled(s.busy.is_none(), egui::Button::new("Save as…")).on_hover_text("Write a converted copy next to the original, which is kept").clicked() {
                                    start = Some(*t);
                                }
                            }
                            Err(e) => {
                                ui.add(egui::Label::new(egui::RichText::new(e).weak()).wrap());
                                ui.label("");
                            }
                        }
                        ui.end_row();
                    }
                });
                if !rows.iter().any(|(_, r)| r.as_ref().is_ok_and(|n| *n < file_size)) {
                    ui.label("None of these is smaller than the file is now.");
                }
                if let Some(t) = start {
                    let suggested = write::suggested_name(path, t);
                    let ext = if t == Target::CompressedPly { "ply" } else { t.extension() };
                    let mut d = crate::asking::Dialog::new().set_file_name(suggested.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()).add_filter(t.name(), &[ext]);
                    if let Some(dir) = path.parent() {
                        d = d.set_directory(dir);
                    }
                    s.picking = Some((t, crate::asking::ask_into(d, crate::asking::Pick::Save, ui.ctx())));
                    s.said = None;
                }
                ui.add_space(2.0);
                ui.add(egui::Label::new(egui::RichText::new("SPZ and compressed PLY store colours and shapes a little more coarsely than the trainers do; it rarely shows. The original is always kept, and each new file is read back and checked.").weak()).wrap());
            }
            None => {}
        }
        // The picker's answer starts the conversion.
        let picked = s.picking.as_ref().and_then(|(t, m)| m.lock().unwrap().take().map(|p| (*t, p)));
        if let Some((t, dest)) = picked {
            s.picking = None;
            s.busy = Some(format!("Writing {} and checking it…", dest.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()));
            let (st2, p, sc, ctx) = (st.clone(), path.to_path_buf(), scene.clone(), ui.ctx().clone());
            std::thread::spawn(move || {
                let r = convert(&p, &sc, t, &dest);
                let mut s = st2.lock().unwrap();
                s.said = Some(r);
                s.busy = None;
                ctx.request_repaint();
            });
        } else if s.picking.is_some() {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
        }
        match &s.said {
            Some(Ok(m)) => {
                ui.add_space(4.0);
                ui.add(egui::Label::new(egui::RichText::new(m).color(Color32::from_rgb(70, 160, 90))).wrap());
            }
            Some(Err(e)) => {
                ui.add_space(4.0);
                ui.add(egui::Label::new(egui::RichText::new(e).color(Color32::from_rgb(210, 80, 70))).wrap());
            }
            None => {}
        }
    });
}

fn convert(path: &Path, shown: &Arc<Scene>, t: Target, dest: &PathBuf) -> Result<String, String> {
    let scene = full_scene(path, shown)?;
    let checked = write::convert(&scene, path, t, dest)?;
    let size = std::fs::metadata(dest).map(|m| m.len()).unwrap_or(0);
    Ok(format!("Saved {} ({}). {}", dest.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(), crate::scan::human(size), checked.summary()))
}

#[cfg(test)]
mod tests {
    #[test]
    fn converting_from_the_card_says_what_it_saved_and_checked() {
        let dir = std::env::temp_dir().join(format!("filescape-splatopt-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let src = dir.join("tiny.spz");
        std::fs::write(&src, include_bytes!("../splat/testdata/minimal-v3.spz")).unwrap();
        let shown = std::sync::Arc::new(crate::splat::read(&src, crate::splat::Limits::PREVIEW).unwrap());
        let m = super::measure(&src, &shown).unwrap();
        assert_eq!(m.iter().map(|(t, _)| *t).collect::<Vec<_>>(), [super::Target::CompressedPly, super::Target::Ply], "SPZ isn't offered for an SPZ");
        assert!(m.iter().all(|(_, r)| r.as_ref().is_ok_and(|n| *n > 0)));
        let dest = dir.join("tiny.ply");
        let said = super::convert(&src, &shown, super::Target::Ply, &dest).unwrap();
        assert!(said.starts_with("Saved tiny.ply") && said.contains("all 4 splats"), "{said}");
        assert!(super::convert(&src, &shown, super::Target::Ply, &src).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
