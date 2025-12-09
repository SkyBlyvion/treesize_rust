use std::collections::hash_map::DefaultHasher;
use std::ffi::OsStr;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use std::thread;

use crossbeam_channel::{unbounded, Receiver};
use eframe::{egui, NativeOptions};
use rayon::prelude::*;

fn main() -> eframe::Result<()> {
    let native_options = NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size(egui::vec2(1100.0, 750.0)),
        ..Default::default()
    };

    eframe::run_native(
        "TreeSize Rust (Windows / Linux)",
        native_options,
        Box::new(|cc| Box::new(TreeSizeApp::new(cc))),
    )
}

#[derive(Debug, Clone)]
struct Node {
    name: String,
    path: PathBuf,
    is_dir: bool,
    size: u64,
    file_count: u64,
    children: Vec<Node>,
}

impl Node {
    fn new_dir(name: String, path: PathBuf, children: Vec<Node>) -> Self {
        let mut size = 0;
        let mut file_count = 0;

        for child in &children {
            size += child.size;
            file_count += child.file_count;
        }

        let mut children = children;
        children.sort_by(|a, b| b.size.cmp(&a.size));

        Self {
            name,
            path,
            is_dir: true,
            size,
            file_count,
            children,
        }
    }

    fn new_file(name: String, path: PathBuf, size: u64) -> Self {
        Self {
            name,
            path,
            is_dir: false,
            size,
            file_count: 1,
            children: Vec::new(),
        }
    }
}

#[derive(Debug)]
struct ScanResult {
    root_path: PathBuf,
    root_node: Option<Node>,
    error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScanMode {
    Folder,
    Drive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ViewMode {
    Tree,
    Treemap,
}

struct TreeSizeApp {
    root_path: Option<PathBuf>,
    root_node: Option<Node>,
    is_scanning: bool,
    status: String,
    scan_receiver: Option<Receiver<ScanResult>>,
    cancel_flag: Option<Arc<AtomicBool>>,

    // Scan options
    available_roots: Vec<PathBuf>,
    selected_root_index: usize,
    scan_mode: ScanMode,

    // UI / sélection
    view_mode: ViewMode,
    selected_node_path: Option<PathBuf>,

    // Suppression
    pending_delete: Option<PathBuf>,

    // Clipboard interne pour copier/couper/coller
    clipboard_path: Option<PathBuf>,
    clipboard_is_cut: bool,
    pending_paste_dest: Option<PathBuf>,
}

impl Default for TreeSizeApp {
    fn default() -> Self {
        let roots = list_roots();

        Self {
            root_path: None,
            root_node: None,
            is_scanning: false,
            status: "Choisis un dossier ou un lecteur puis lance un scan."
                .to_string(),
            scan_receiver: None,
            cancel_flag: None,
            available_roots: roots,
            selected_root_index: 0,
            scan_mode: ScanMode::Folder,
            view_mode: ViewMode::Tree,
            selected_node_path: None,
            pending_delete: None,
            clipboard_path: None,
            clipboard_is_cut: false,
            pending_paste_dest: None,
        }
    }
}

impl TreeSizeApp {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        // Style global + thème dark un peu modernisé
        let mut style = (*cc.egui_ctx.style()).clone();
        style.visuals = egui::Visuals::dark();

        style.spacing.item_spacing = egui::vec2(8.0, 6.0);
        style.spacing.window_margin = egui::Margin::symmetric(10.0, 8.0);

        style.visuals.window_rounding = 6.0.into();
        style.visuals.menu_rounding = 6.0.into();
        style.visuals.widgets.inactive.rounding = 4.0.into();
        style.visuals.widgets.hovered.rounding = 4.0.into();
        style.visuals.widgets.active.rounding = 4.0.into();

        style.text_styles.insert(
            egui::TextStyle::Heading,
            egui::FontId::proportional(20.0),
        );
        style.text_styles.insert(
            egui::TextStyle::Body,
            egui::FontId::proportional(14.0),
        );
        style.text_styles.insert(
            egui::TextStyle::Monospace,
            egui::FontId::monospace(13.0),
        );

        cc.egui_ctx.set_style(style);
        cc.egui_ctx.set_pixels_per_point(1.05);

        Self::default()
    }

    fn draw_top_bar(&self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("top_bar")
            .exact_height(60.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.heading("TreeSize Rust");
                    ui.separator();

                    if let Some(path) = &self.root_path {
                        ui.label(
                            egui::RichText::new(path.to_string_lossy())
                                .monospace()
                                .weak(),
                        );
                    } else {
                        ui.weak("Aucun dossier / lecteur sélectionné");
                    }

                    ui.with_layout(
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| {
                            if self.is_scanning {
                                ui.spinner();
                                ui.label("Scan en cours…");
                            } else {
                                ui.label("Prêt");
                            }
                        },
                    );
                });

                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(&self.status)
                            .small()
                            .weak(),
                    );
                });
            });
    }

    fn draw_left_panel(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("left_panel")
            .resizable(true)
            .default_width(340.0)
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    ui.label(
                        egui::RichText::new("Analyseur d’occupation disque")
                            .small()
                            .italics()
                            .weak(),
                    );
                });

                ui.add_space(8.0);

                section_card(ui, "Mode de scan", |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.selectable_value(
                            &mut self.scan_mode,
                            ScanMode::Folder,
                            "Dossier",
                        );
                        ui.selectable_value(
                            &mut self.scan_mode,
                            ScanMode::Drive,
                            "Lecteur",
                        );
                    });

                    ui.add_space(6.0);

                    match self.scan_mode {
                        ScanMode::Folder => {
                            ui.label("Dossier sélectionné :");
                            if let Some(path) = &self.root_path {
                                ui.monospace(path.to_string_lossy());
                            } else {
                                ui.weak("(aucun dossier sélectionné)");
                            }

                            ui.add_space(4.0);

                            let choose_btn = egui::Button::new(
                                "Choisir un dossier…",
                            )
                            .fill(
                                ui.visuals().widgets.inactive.bg_fill,
                            );

                            if ui
                                .add_enabled(!self.is_scanning, choose_btn)
                                .clicked()
                            {
                                if let Some(path) = pick_directory() {
                                    self.root_path = Some(path.clone());
                                    self.status = format!(
                                        "Dossier sélectionné : {}",
                                        path.to_string_lossy()
                                    );
                                }
                            }
                        }
                        ScanMode::Drive => {
                            ui.label("Lecteur / racine à scanner :");
                            if self.available_roots.is_empty() {
                                ui.weak("Aucun lecteur détecté.");
                            } else {
                                let current_index =
                                    self.selected_root_index.min(
                                        self.available_roots.len()
                                            .saturating_sub(1),
                                    );
                                self.selected_root_index =
                                    current_index;

                                let current_root = &self.available_roots
                                    [self.selected_root_index];

                                egui::ComboBox::from_label(
                                    "Choisir un lecteur",
                                )
                                .selected_text(
                                    current_root.to_string_lossy(),
                                )
                                .show_ui(ui, |ui| {
                                    for (i, root) in self
                                        .available_roots
                                        .iter()
                                        .enumerate()
                                    {
                                        let label = root
                                            .to_string_lossy()
                                            .to_string();
                                        ui.selectable_value(
                                            &mut self
                                                .selected_root_index,
                                            i,
                                            label,
                                        );
                                    }
                                });

                                self.root_path =
                                    Some(current_root.to_path_buf());
                            }
                        }
                    }
                });

                section_card(ui, "Actions", |ui| {
                    ui.horizontal(|ui| {
                        let can_scan =
                            !self.is_scanning && self.root_path.is_some();
                        if ui
                            .add_enabled(
                                can_scan,
                                egui::Button::new("Lancer le scan")
                                    .fill(
                                        ui.visuals()
                                            .selection
                                            .bg_fill,
                                    ),
                            )
                            .clicked()
                        {
                            if let Some(path) =
                                self.root_path.clone()
                            {
                                self.start_scan(path);
                            } else {
                                self.status = "Aucun dossier / lecteur sélectionné"
                                    .to_string();
                            }
                        }

                        let can_stop =
                            self.is_scanning && self.cancel_flag.is_some();
                        if ui
                            .add_enabled(
                                can_stop,
                                egui::Button::new("Arrêter")
                                    .fill(
                                        egui::Color32::from_rgb(
                                            120, 40, 40,
                                        ),
                                    ),
                            )
                            .clicked()
                        {
                            if let Some(flag) = &self.cancel_flag {
                                flag.store(true, Ordering::Relaxed);
                                self.status = "Arrêt du scan demandé…"
                                    .to_string();
                            }
                        }
                    });
                });

                section_card(ui, "Vue", |ui| {
                    ui.horizontal(|ui| {
                        ui.selectable_value(
                            &mut self.view_mode,
                            ViewMode::Tree,
                            "Arborescence",
                        );
                        ui.selectable_value(
                            &mut self.view_mode,
                            ViewMode::Treemap,
                            "Treemap",
                        );
                    });
                });

                section_card(ui, "Élément sélectionné", |ui| {
                    if let Some(node) = self.get_selected_node() {
                        ui.label(format!("Nom : {}", node.name));
                        ui.monospace(node.path.to_string_lossy());
                        ui.label(format!(
                            "Taille : {}",
                            format_bytes(node.size)
                        ));
                        ui.label(format!(
                            "Fichiers : {}",
                            node.file_count
                        ));
                        ui.add_space(6.0);
                        if ui
                            .button(
                                egui::RichText::new(
                                    "Supprimer cet élément…",
                                )
                                .color(egui::Color32::RED),
                            )
                            .clicked()
                        {
                            self.pending_delete =
                                Some(node.path.clone());
                        }
                    } else {
                        ui.weak("Aucun élément sélectionné.");
                    }
                });

                section_card(ui, "Presse-papier (fichiers/dossiers)", |ui| {
                    match &self.clipboard_path {
                        Some(p) => {
                            ui.label(if self.clipboard_is_cut {
                                "Mode : Couper"
                            } else {
                                "Mode : Copier"
                            });
                            ui.monospace(p.to_string_lossy());
                            ui.add_space(4.0);
                            if ui
                                .button("Vider le presse-papier")
                                .clicked()
                            {
                                self.clipboard_path = None;
                                self.clipboard_is_cut = false;
                            }
                        }
                        None => {
                            ui.weak("Presse-papier vide.");
                        }
                    }
                });

                section_card(ui, "Aide rapide", |ui| {
                    ui.small(
                        "• Clic gauche : sélection dans l’arborescence ou la treemap.\n\
                         • Clic droit : menu contextuel (Propriétés, Copier chemin, Copier/Couper, Supprimer, Coller ici).\n\
                         • Les erreurs d’accès (permissions, fichiers spéciaux…) sont ignorées.\n\
                         • L’arrêt du scan est coopératif : les threads finissent proprement.",
                    );
                });
            });
    }

    fn draw_central_panel(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            if let Some(root) = &self.root_node {
                let total_size = root.size;
                let total_files = root.file_count;

                section_card(ui, "Résultats du scan", |ui| {
                    ui.horizontal(|ui| {
                        ui.label("Racine :");
                        ui.monospace(root.path.to_string_lossy());
                    });

                    ui.add_space(4.0);

                    ui.horizontal(|ui| {
                        ui.label(format!(
                            "Taille totale : {}",
                            format_bytes(total_size)
                        ));
                        ui.separator();
                        ui.label(format!(
                            "Nombre de fichiers : {}",
                            total_files
                        ));
                    });
                });

                ui.add_space(4.0);

                match self.view_mode {
                    ViewMode::Tree => {
                        section_card(
                            ui,
                            "Arborescence (triée par taille)",
                            |ui| {
                                egui::ScrollArea::vertical()
                                    .auto_shrink([false, false])
                                    .show(ui, |ui| {
                                        draw_node_recursive(
                                            ui,
                                            root,
                                            total_size,
                                            0,
                                            &mut self
                                                .selected_node_path,
                                            &mut self.pending_delete,
                                            &mut self
                                                .clipboard_path,
                                            &mut self
                                                .clipboard_is_cut,
                                            &mut self
                                                .pending_paste_dest,
                                        );
                                    });
                            },
                        );
                    }
                    ViewMode::Treemap => {
                        section_card(ui, "Treemap façon WinDirStat", |ui| {
                            ui.small(
                                "Chaque bloc représente un dossier/fichier, \
                                 proportionnel à sa taille.",
                            );
                            ui.add_space(6.0);

                            draw_treemap(
                                ui,
                                root,
                                &mut self.selected_node_path,
                                &mut self.pending_delete,
                                &mut self.clipboard_path,
                                &mut self.clipboard_is_cut,
                                &mut self.pending_paste_dest,
                            );
                        });
                    }
                }
            } else {
                ui.centered_and_justified(|ui| {
                    ui.label(
                        "Choisis un dossier ou un lecteur dans le panneau de gauche, puis lance un scan.",
                    );
                });
            }
        });
    }

    fn draw_delete_window(&mut self, ctx: &egui::Context) {
        if let Some(path) = self.pending_delete.clone() {
            egui::Window::new("Confirmer la suppression")
                .collapsible(false)
                .resizable(false)
                .anchor(
                    egui::Align2::CENTER_CENTER,
                    egui::vec2(0.0, 0.0),
                )
                .show(ctx, |ui| {
                    ui.label("Es-tu sûr de vouloir supprimer :");
                    ui.monospace(path.to_string_lossy());
                    ui.add_space(8.0);
                    ui.colored_label(
                        egui::Color32::RED,
                        "Attention : la suppression est définitive.",
                    );
                    ui.add_space(12.0);

                    let mut close = false;

                    ui.horizontal(|ui| {
                        if ui.button("Annuler").clicked() {
                            self.pending_delete = None;
                            close = true;
                        }
                        if ui
                            .button(
                                egui::RichText::new("Supprimer")
                                    .color(egui::Color32::RED),
                            )
                            .clicked()
                        {
                            match delete_path(&path) {
                                Ok(()) => {
                                    self.status = format!(
                                        "Supprimé : {}",
                                        path.to_string_lossy()
                                    );
                                    if let Some(root) =
                                        self.root_path.clone()
                                    {
                                        self.start_scan(root);
                                    }
                                }
                                Err(e) => {
                                    self.status = format!(
                                        "Erreur suppression : {}",
                                        e
                                    );
                                }
                            }
                            self.pending_delete = None;
                            close = true;
                        }
                    });

                    if close {
                        // la fenêtre disparaîtra car pending_delete = None
                    }
                });
        }
    }

    fn start_scan(&mut self, path: PathBuf) {
        self.is_scanning = true;
        self.status =
            format!("Scan en cours pour : {}", path.to_string_lossy());
        self.root_node = None;
        self.selected_node_path = None;
        self.pending_delete = None;

        let (tx, rx) = unbounded::<ScanResult>();
        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_clone = cancel.clone();

        self.scan_receiver = Some(rx);
        self.cancel_flag = Some(cancel);

        thread::spawn(move || {
            let result = scan_directory_parallel(&path, &cancel_clone);
            let _ = tx.send(result);
        });
    }

    fn get_selected_node(&self) -> Option<&Node> {
        let root = self.root_node.as_ref()?;
        let path = self.selected_node_path.as_ref()?;
        find_node_by_path(root, path)
    }

    fn handle_paste(&mut self, dest_dir: &Path) {
        let src = match self.clipboard_path.clone() {
            Some(p) => p,
            None => {
                self.status =
                    "Presse-papier vide, rien à coller.".to_string();
                return;
            }
        };

        let is_cut = self.clipboard_is_cut;

        match copy_or_move(&src, dest_dir, is_cut) {
            Ok(()) => {
                if is_cut {
                    self.clipboard_path = None;
                    self.clipboard_is_cut = false;
                    self.status = format!(
                        "Déplacé vers : {}",
                        dest_dir.to_string_lossy()
                    );
                } else {
                    self.status = format!(
                        "Copié vers : {}",
                        dest_dir.to_string_lossy()
                    );
                }

                if let Some(root) = self.root_path.clone() {
                    self.start_scan(root);
                }
            }
            Err(e) => {
                self.status =
                    format!("Erreur copie/déplacement : {}", e);
            }
        }
    }
}

impl eframe::App for TreeSizeApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.is_scanning {
            ctx.request_repaint();
        }

        // Récupère le résultat du scan si disponible
        if self.is_scanning {
            if let Some(rx) = &self.scan_receiver {
                if let Ok(result) = rx.try_recv() {
                    self.is_scanning = false;
                    self.scan_receiver = None;
                    self.cancel_flag = None;

                    if let Some(err) = result.error {
                        self.root_node = None;
                        self.status = err;
                    } else {
                        self.root_node = result.root_node;
                        self.status = format!(
                            "Scan terminé pour : {}",
                            result.root_path.to_string_lossy()
                        );
                    }

                    ctx.request_repaint();
                }
            }
        }

        self.draw_top_bar(ctx);
        self.draw_left_panel(ctx);
        self.draw_central_panel(ctx);
        self.draw_delete_window(ctx);

        // Traitement différé du "Coller ici"
        if let Some(dest) = self.pending_paste_dest.take() {
            self.handle_paste(&dest);
        }
    }
}

/// Petit helper pour dessiner un panneau type “card”.
fn section_card(
    ui: &mut egui::Ui,
    title: &str,
    body: impl FnOnce(&mut egui::Ui),
) {
    ui.group(|ui| {
        ui.vertical(|ui| {
            ui.label(
                egui::RichText::new(title)
                    .strong()
                    .size(15.0),
            );
            ui.add_space(4.0);
            body(ui);
        });
    });
    ui.add_space(6.0);
}

/// Dessin récursif d'un Node en arbre (vue arborescence) + clic gauche/droit.
fn draw_node_recursive(
    ui: &mut egui::Ui,
    node: &Node,
    root_size: u64,
    indent_level: usize,
    selected_node_path: &mut Option<PathBuf>,
    pending_delete: &mut Option<PathBuf>,
    clipboard_path: &mut Option<PathBuf>,
    clipboard_is_cut: &mut bool,
    pending_paste_dest: &mut Option<PathBuf>,
) {
    let indent = 18.0 * indent_level as f32;
    let percentage = if root_size > 0 {
        (node.size as f64 / root_size as f64) * 100.0
    } else {
        0.0
    };

    let label = if node.is_dir {
        format!(
            "{} ({} | {} fichiers | {:.2}%)",
            node.name,
            format_bytes(node.size),
            node.file_count,
            percentage
        )
    } else {
        format!(
            "{} ({}, {:.2}%)",
            node.name,
            format_bytes(node.size),
            percentage
        )
    };

    let is_selected = selected_node_path
        .as_ref()
        .map_or(false, |p| p == &node.path);

    if node.is_dir {
        let header_label = if is_selected {
            egui::RichText::new(label.clone()).strong()
        } else {
            egui::RichText::new(label.clone())
        };

        let header = egui::CollapsingHeader::new(header_label)
            .default_open(indent_level == 0)
            .id_source(node.path.to_string_lossy().to_string());

        let collapsing = header.show(ui, |ui| {
            for child in &node.children {
                ui.horizontal(|ui| {
                    ui.add_space(indent + 10.0);
                    draw_node_recursive(
                        ui,
                        child,
                        root_size,
                        indent_level + 1,
                        selected_node_path,
                        pending_delete,
                        clipboard_path,
                        clipboard_is_cut,
                        pending_paste_dest,
                    );
                });
            }
        });

        let header_resp = collapsing.header_response;

        if header_resp.clicked() {
            *selected_node_path = Some(node.path.clone());
        }

        header_resp.context_menu(|ui| {
            if ui.button("Propriétés").clicked() {
                *selected_node_path = Some(node.path.clone());
                ui.close_menu();
            }
            if ui.button("Copier le chemin").clicked() {
                let text = node.path.to_string_lossy().to_string();
                ui.output_mut(|o| o.copied_text = text);
                ui.close_menu();
            }
            if ui.button("Copier").clicked() {
                *clipboard_path = Some(node.path.clone());
                *clipboard_is_cut = false;
                ui.close_menu();
            }
            if ui.button("Couper").clicked() {
                *clipboard_path = Some(node.path.clone());
                *clipboard_is_cut = true;
                ui.close_menu();
            }
            if clipboard_path.is_some() {
                if ui.button("Coller ici").clicked() {
                    *pending_paste_dest = Some(node.path.clone());
                    ui.close_menu();
                }
            }
            if ui
                .button(
                    egui::RichText::new("Supprimer…")
                        .color(egui::Color32::RED),
                )
                .clicked()
            {
                *pending_delete = Some(node.path.clone());
                ui.close_menu();
            }
        });
    } else {
        let resp = ui
            .horizontal(|ui| {
                ui.add_space(indent + 10.0);
                ui.selectable_label(is_selected, label)
            })
            .inner;

        if resp.clicked() {
            *selected_node_path = Some(node.path.clone());
        }

        resp.context_menu(|ui| {
            if ui.button("Propriétés").clicked() {
                *selected_node_path = Some(node.path.clone());
                ui.close_menu();
            }
            if ui.button("Copier le chemin").clicked() {
                let text = node.path.to_string_lossy().to_string();
                ui.output_mut(|o| o.copied_text = text);
                ui.close_menu();
            }
            if ui.button("Copier").clicked() {
                *clipboard_path = Some(node.path.clone());
                *clipboard_is_cut = false;
                ui.close_menu();
            }
            if ui.button("Couper").clicked() {
                *clipboard_path = Some(node.path.clone());
                *clipboard_is_cut = true;
                ui.close_menu();
            }

            // Coller dans le même dossier que ce fichier
            if clipboard_path.is_some() {
                if let Some(parent) = node.path.parent() {
                    if ui.button("Coller ici").clicked() {
                        *pending_paste_dest = Some(parent.to_path_buf());
                        ui.close_menu();
                    }
                }
            }

            if ui
                .button(
                    egui::RichText::new("Supprimer…")
                        .color(egui::Color32::RED),
                )
                .clicked()
            {
                *pending_delete = Some(node.path.clone());
                ui.close_menu();
            }
        });
    }
}

/// Un bloc cliquable dans la treemap.
struct Hit {
    rect: egui::Rect,
    path: PathBuf,
    name: String,
    size: u64,
    is_dir: bool,
}

/// Dessin de la treemap façon WinDirStat + clic gauche/droit.
fn draw_treemap(
    ui: &mut egui::Ui,
    root: &Node,
    selected_path: &mut Option<PathBuf>,
    pending_delete: &mut Option<PathBuf>,
    clipboard_path: &mut Option<PathBuf>,
    clipboard_is_cut: &mut bool,
    pending_paste_dest: &mut Option<PathBuf>,
) {
    let total_size = root.size.max(1);

    let available_size = ui.available_size();
    let size = egui::vec2(
        available_size.x.max(200.0),
        available_size.y.max(200.0),
    );

    let (response, painter) =
        ui.allocate_painter(size, egui::Sense::click());
    let rect = response.rect;

    let children = &root.children;
    let sum_children_size = children
        .iter()
        .map(|c| c.size)
        .sum::<u64>()
        .max(1);

    let mut hits: Vec<Hit> = Vec::new();

    layout_treemap_rect(
        &painter,
        rect,
        true,
        children,
        sum_children_size,
        selected_path,
        &mut hits,
        0,
    );

    if let Some(pos) = response.interact_pointer_pos() {
        // Clic gauche => sélection
        if response.clicked() {
            for hit in &hits {
                if hit.rect.contains(pos) {
                    *selected_path = Some(hit.path.clone());
                    break;
                }
            }
        }

        // Tooltip au survol
        for hit in &hits {
            if hit.rect.contains(pos) {
                let percent =
                    (hit.size as f64 / total_size as f64) * 100.0;
                let text = format!(
                    "{}\n{}\n{} ({:.2}%)",
                    hit.name,
                    hit.path.display(),
                    format_bytes(hit.size),
                    percent
                );

                egui::show_tooltip_at_pointer(
                    ui.ctx(),
                    egui::Id::new("treemap_tooltip"),
                    |ui| {
                        ui.label(text);
                    },
                );
                break;
            }
        }
    }

    // Menu contextuel (clic droit) sur la zone treemap
    response.context_menu(|ui| {
        if let Some(pos) = ui.ctx().pointer_latest_pos() {
            if let Some(hit) =
                hits.iter().find(|h| h.rect.contains(pos))
            {
                ui.label(hit.name.clone());
                ui.monospace(hit.path.to_string_lossy());
                ui.separator();

                if ui.button("Propriétés").clicked() {
                    *selected_path = Some(hit.path.clone());
                    ui.close_menu();
                }
                if ui.button("Copier le chemin").clicked() {
                    let text =
                        hit.path.to_string_lossy().to_string();
                    ui.output_mut(|o| o.copied_text = text);
                    ui.close_menu();
                }
                if ui.button("Copier").clicked() {
                    *clipboard_path = Some(hit.path.clone());
                    *clipboard_is_cut = false;
                    ui.close_menu();
                }
                if ui.button("Couper").clicked() {
                    *clipboard_path = Some(hit.path.clone());
                    *clipboard_is_cut = true;
                    ui.close_menu();
                }

                // Coller ici : si on est sur un dossier => dedans, sinon => dans le parent du fichier
                if clipboard_path.is_some() {
                    let dest_dir = if hit.is_dir {
                        Some(hit.path.clone())
                    } else {
                        hit.path.parent().map(|p| p.to_path_buf())
                    };

                    if let Some(dest) = dest_dir {
                        if ui.button("Coller ici").clicked() {
                            *pending_paste_dest = Some(dest);
                            ui.close_menu();
                        }
                    }
                }

                if ui
                    .button(
                        egui::RichText::new("Supprimer…")
                            .color(egui::Color32::RED),
                    )
                    .clicked()
                {
                    *pending_delete = Some(hit.path.clone());
                    ui.close_menu();
                }
            } else {
                ui.weak("Aucun élément ici.");
            }
        } else {
            ui.weak("Aucun pointeur.");
        }
    });
}

/// Algorithme de treemap simple (slice-and-dice) avec alternance horizontal/vertical.
fn layout_treemap_rect(
    painter: &egui::Painter,
    rect: egui::Rect,
    horizontal: bool,
    nodes: &[Node],
    total_size: u64,
    selected_path: &Option<PathBuf>,
    hits: &mut Vec<Hit>,
    depth: usize,
) {
    if nodes.is_empty() || rect.width() <= 2.0 || rect.height() <= 2.0 {
        return;
    }

    let mut offset = if horizontal { rect.left() } else { rect.top() };
    let total_size_f = total_size as f32;

    for node in nodes {
        if node.size == 0 {
            continue;
        }

        let fraction = node.size as f32 / total_size_f;
        if fraction <= 0.0 {
            continue;
        }

        let r = if horizontal {
            let w = rect.width() * fraction;
            let x1 = offset;
            let x2 = (offset + w).min(rect.right());
            offset += w;
            egui::Rect::from_min_max(
                egui::pos2(x1, rect.top()),
                egui::pos2(x2, rect.bottom()),
            )
        } else {
            let h = rect.height() * fraction;
            let y1 = offset;
            let y2 = (offset + h).min(rect.bottom());
            offset += h;
            egui::Rect::from_min_max(
                egui::pos2(rect.left(), y1),
                egui::pos2(rect.right(), y2),
            )
        };

        if r.width() < 2.0 || r.height() < 2.0 {
            continue;
        }

        let is_selected = selected_path
            .as_ref()
            .map_or(false, |p| p == &node.path);

        let base_color = color_for_path(&node.path, depth);
        let fill_color = if is_selected {
            base_color.gamma_multiply(0.8)
        } else {
            base_color
        };

        painter.rect_filled(r, 1.0, fill_color);

        let stroke = if is_selected {
            egui::Stroke {
                width: 2.0,
                color: egui::Color32::WHITE,
            }
        } else {
            egui::Stroke {
                width: 0.5,
                color: egui::Color32::from_gray(40),
            }
        };
        painter.rect_stroke(r, 1.0, stroke);

        if r.width() > 60.0 && r.height() > 30.0 {
            let percent =
                (node.size as f64 / total_size as f64) * 100.0;
            let text = format!(
                "{}\n{} ({:.1}%)",
                node.name,
                format_bytes(node.size),
                percent
            );
            painter.text(
                r.left_top() + egui::vec2(3.0, 3.0),
                egui::Align2::LEFT_TOP,
                text,
                egui::FontId::proportional(10.0),
                egui::Color32::WHITE,
            );
        }

        hits.push(Hit {
            rect: r,
            path: node.path.clone(),
            name: node.name.clone(),
            size: node.size,
            is_dir: node.is_dir,
        });

        if !node.children.is_empty() && depth < 3 {
            let child_total = node
                .children
                .iter()
                .map(|c| c.size)
                .sum::<u64>()
                .max(1);
            layout_treemap_rect(
                painter,
                r.shrink(1.0),
                !horizontal,
                &node.children,
                child_total,
                selected_path,
                hits,
                depth + 1,
            );
        }
    }
}

/// Coloration déterministe en fonction du chemin (pour la treemap).
fn color_for_path(path: &Path, depth: usize) -> egui::Color32 {
    let mut hasher = DefaultHasher::new();
    path.hash(&mut hasher);
    depth.hash(&mut hasher);
    let hash = hasher.finish();

    let h = (hash % 360) as f32 / 360.0;
    let s = 0.5 + ((hash >> 8) % 50) as f32 / 100.0;
    let v = 0.4 + ((hash >> 16) % 60) as f32 / 100.0;

    let hsva =
        egui::epaint::Hsva::new(h, s.min(1.0), v.min(1.0), 1.0);
    hsva.into()
}

/// Utilise une boîte de dialogue native pour choisir un dossier.
fn pick_directory() -> Option<PathBuf> {
    rfd::FileDialog::new().pick_folder()
}

/// Scan récursif avec parallélisation et support d'annulation.
fn scan_directory_parallel(root: &Path, cancel: &AtomicBool) -> ScanResult {
    if cancel.load(Ordering::Relaxed) {
        return ScanResult {
            root_path: root.to_path_buf(),
            root_node: None,
            error: Some("Scan annulé".to_string()),
        };
    }

    if !root.exists() {
        return ScanResult {
            root_path: root.to_path_buf(),
            root_node: None,
            error: Some("Le dossier / lecteur n'existe pas".to_string()),
        };
    }

    if !root.is_dir() {
        return ScanResult {
            root_path: root.to_path_buf(),
            root_node: None,
            error: Some(
                "Chemin sélectionné n'est pas un dossier".to_string(),
            ),
        };
    }

    let mut direct_children: Vec<PathBuf> = Vec::new();
    match root.read_dir() {
        Ok(read_dir) => {
            for entry in read_dir.flatten() {
                direct_children.push(entry.path());
            }
        }
        Err(e) => {
            return ScanResult {
                root_path: root.to_path_buf(),
                root_node: None,
                error: Some(format!(
                    "Impossible de lire le dossier racine : {e}"
                )),
            };
        }
    }

    let children_nodes: Vec<Node> = direct_children
        .par_iter()
        .filter_map(|path| {
            if cancel.load(Ordering::Relaxed) {
                return None;
            }
            build_node(path, cancel).ok()
        })
        .collect();

    if cancel.load(Ordering::Relaxed) {
        return ScanResult {
            root_path: root.to_path_buf(),
            root_node: None,
            error: Some("Scan annulé".to_string()),
        };
    }

    let name = root
        .file_name()
        .map(|os| os.to_string_lossy().to_string())
        .unwrap_or_else(|| root.to_string_lossy().to_string());

    let root_node = Node::new_dir(name, root.to_path_buf(), children_nodes);

    ScanResult {
        root_path: root.to_path_buf(),
        root_node: Some(root_node),
        error: None,
    }
}

/// Construit un Node (fichier ou dossier) pour un chemin donné.
fn build_node(path: &Path, cancel: &AtomicBool) -> Result<Node, String> {
    if cancel.load(Ordering::Relaxed) {
        return Err("Annulé".to_string());
    }

    if path.is_file() {
        let size = path.metadata().map(|m| m.len()).unwrap_or(0);
        let name = path
            .file_name()
            .map(|os| os.to_string_lossy().to_string())
            .unwrap_or_else(|| path.to_string_lossy().to_string());
        Ok(Node::new_file(name, path.to_path_buf(), size))
    } else if path.is_dir() {
        let name = path
            .file_name()
            .map(|os| os.to_string_lossy().to_string())
            .unwrap_or_else(|| path.to_string_lossy().to_string());

        let mut entries: Vec<PathBuf> = Vec::new();
        if let Ok(read_dir) = path.read_dir() {
            for entry in read_dir.flatten() {
                entries.push(entry.path());
            }
        }

        let children: Vec<Node> = entries
            .par_iter()
            .filter_map(|p| {
                if cancel.load(Ordering::Relaxed) {
                    return None;
                }
                build_node(p, cancel).ok()
            })
            .collect();

        Ok(Node::new_dir(name, path.to_path_buf(), children))
    } else {
        Err("Type de fichier non pris en charge".to_string())
    }
}

/// Suppression d'un fichier ou dossier (récursif pour les dossiers).
fn delete_path(path: &Path) -> std::io::Result<()> {
    let meta = fs::symlink_metadata(path)?;
    if meta.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
}

#[cfg(target_os = "windows")]
fn is_path_too_long(path: &Path) -> bool {
    // Windows classique ~260 caractères (MAX_PATH)
    path.to_string_lossy().len() > 260
}

#[cfg(not(target_os = "windows"))]
fn is_path_too_long(_path: &Path) -> bool {
    false
}

/// Copie/déplacement de fichier ou dossier dans un dossier cible.
fn copy_or_move(
    src: &Path,
    dest_dir: &Path,
    is_cut: bool,
) -> Result<(), String> {
    if !dest_dir.exists() || !dest_dir.is_dir() {
        return Err(format!(
            "Destination invalide : {}",
            dest_dir.to_string_lossy()
        ));
    }
    if !src.exists() {
        return Err(format!(
            "Source introuvable : {}",
            src.to_string_lossy()
        ));
    }

    if is_cut && dest_dir.starts_with(src) {
        return Err(
            "Impossible de déplacer un dossier dans lui-même ou un sous-dossier."
                .to_string(),
        );
    }

    let file_name =
        src.file_name().unwrap_or_else(|| OsStr::new("unnamed"));
    let dest_path = dest_dir.join(file_name);

    if is_path_too_long(&dest_path) {
        return Err(format!(
            "Chemin de destination trop long pour le système ({} caractères).\n{}",
            dest_path.to_string_lossy().len(),
            dest_path.to_string_lossy()
        ));
    }

    if is_cut {
        match fs::rename(src, &dest_path) {
            Ok(()) => return Ok(()),
            Err(_) => {
                // cross-device, fallback copy + delete
            }
        }
    }

    let metadata = fs::symlink_metadata(src).map_err(|e| e.to_string())?;
    if metadata.is_dir() {
        copy_dir_recursive(src, &dest_path).map_err(|e| e.to_string())?;
    } else {
        fs::copy(src, &dest_path).map_err(|e| e.to_string())?;
    }

    if is_cut {
        delete_path(src).map_err(|e| e.to_string())?;
    }

    Ok(())
}

/// Copie récursive de dossier.
fn copy_dir_recursive(src: &Path, dest: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dest)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let entry_path = entry.path();
        let file_type = entry.file_type()?;
        let dest_path = dest.join(entry.file_name());

        if file_type.is_dir() {
            copy_dir_recursive(&entry_path, &dest_path)?;
        } else {
            let _ = fs::copy(&entry_path, &dest_path)?;
        }
    }
    Ok(())
}

/// Recherche d'un Node par chemin.
fn find_node_by_path<'a>(node: &'a Node, path: &Path) -> Option<&'a Node> {
    if node.path == path {
        return Some(node);
    }
    for child in &node.children {
        if let Some(found) = find_node_by_path(child, path) {
            return Some(found);
        }
    }
    None
}

/// Format taille en bytes en Ko/Mo/Go lisible.
fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["o", "Ko", "Mo", "Go", "To"];

    let mut size = bytes as f64;
    let mut unit = 0;

    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }

    format!("{:.1} {}", size, UNITS[unit])
}

/// Liste des racines/lecteurs disponibles selon l'OS.
fn list_roots() -> Vec<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        let mut roots = Vec::new();
        for letter in b'C'..=b'Z' {
            let drive = format!("{}:\\", letter as char);
            let path = PathBuf::from(&drive);
            if path.is_dir() {
                roots.push(path);
            }
        }
        roots
    }

    #[cfg(not(target_os = "windows"))]
    {
        let mut roots = Vec::new();
        let root = PathBuf::from("/");
        if root.is_dir() {
            roots.push(root);
        }
        if let Some(home) = dirs::home_dir() {
            if home.is_dir() {
                roots.push(home);
            }
        }
        roots
    }
}
