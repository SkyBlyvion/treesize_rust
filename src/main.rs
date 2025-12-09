use std::collections::hash_map::DefaultHasher;
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
        Box::new(|_cc| Box::new(TreeSizeApp::default())),
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
        }
    }
}

impl eframe::App for TreeSizeApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
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

        // Panneau de configuration (gauche)
        egui::SidePanel::left("left_panel")
            .resizable(true)
            .default_width(320.0)
            .show(ctx, |ui| {
                ui.heading("Options de scan");
                ui.separator();

                ui.label("Mode de scan :");
                ui.horizontal(|ui| {
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

                ui.add_space(8.0);

                match self.scan_mode {
                    ScanMode::Folder => {
                        ui.label("Dossier sélectionné :");
                        if let Some(path) = &self.root_path {
                            ui.monospace(path.to_string_lossy());
                        } else {
                            ui.weak("(aucun dossier sélectionné)");
                        }

                        if ui
                            .add_enabled(!self.is_scanning, egui::Button::new("Choisir un dossier…"))
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
                            let current_index = self.selected_root_index.min(
                                self.available_roots.len().saturating_sub(1),
                            );
                            self.selected_root_index = current_index;

                            let current_root =
                                &self.available_roots[self.selected_root_index];

                            egui::ComboBox::from_label("Choisir un lecteur")
                                .selected_text(current_root.to_string_lossy())
                                .show_ui(ui, |ui| {
                                    for (i, root) in
                                        self.available_roots.iter().enumerate()
                                    {
                                        let label =
                                            root.to_string_lossy().to_string();
                                        ui.selectable_value(
                                            &mut self.selected_root_index,
                                            i,
                                            label,
                                        );
                                    }
                                });

                            // Met à jour root_path pour le scan
                            self.root_path =
                                Some(current_root.to_path_buf());
                        }
                    }
                }

                ui.add_space(16.0);
                ui.separator();

                ui.horizontal(|ui| {
                    let can_scan =
                        !self.is_scanning && self.root_path.is_some();
                    if ui
                        .add_enabled(can_scan, egui::Button::new("Lancer le scan"))
                        .clicked()
                    {
                        if let Some(path) = self.root_path.clone() {
                            self.start_scan(path);
                        } else {
                            self.status =
                                "Aucun dossier / lecteur sélectionné".to_string();
                        }
                    }

                    let can_stop = self.is_scanning
                        && self.cancel_flag.is_some();
                    if ui
                        .add_enabled(
                            can_stop,
                            egui::Button::new("Arrêter le scan"),
                        )
                        .clicked()
                    {
                        if let Some(flag) = &self.cancel_flag {
                            flag.store(true, Ordering::Relaxed);
                            self.status =
                                "Arrêt du scan demandé, patiente..."
                                    .to_string();
                        }
                    }
                });

                ui.add_space(8.0);
                ui.separator();

                ui.label("Statut :");
                if self.is_scanning {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(&self.status);
                    });
                } else {
                    ui.label(&self.status);
                }

                ui.add_space(12.0);
                ui.separator();

                ui.heading("Vue");
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

                ui.add_space(12.0);
                ui.separator();

                ui.heading("Sélection");
                if let Some(node) = self.get_selected_node() {
                    ui.label(format!("Nom : {}", node.name));
                    ui.monospace(node.path.to_string_lossy());
                    ui.label(format!("Taille : {}", format_bytes(node.size)));
                    ui.label(format!("Fichiers : {}", node.file_count));
                } else {
                    ui.weak("Aucun élément sélectionné.");
                }

                ui.add_space(12.0);
                ui.separator();

                ui.heading("Infos");
                ui.small(
                    "• Les erreurs d'accès (permissions, fichiers spéciaux...) \
                     sont ignorées sans faire planter l'application.\n\
                     • L'arrêt du scan est coopératif : le scan se termine dès \
                     que possible.\n\
                     • Dans la treemap, clique sur un bloc pour sélectionner \
                     un dossier/fichier.",
                );
            });

        // Panneau principal (résultats)
        egui::CentralPanel::default().show(ctx, |ui| {
            if let Some(root) = &self.root_node {
                let total_size = root.size;
                let total_files = root.file_count;

                ui.heading("Résultats du scan");
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label(format!(
                        "Racine : {}",
                        root.path.to_string_lossy()
                    ));
                });

                ui.add_space(6.0);

                ui.horizontal(|ui| {
                    ui.label(format!(
                        "Taille totale : {}",
                        format_bytes(total_size)
                    ));
                    ui.separator();
                    ui.label(format!("Nombre de fichiers : {}", total_files));
                });

                ui.add_space(12.0);
                ui.separator();

                match self.view_mode {
                    ViewMode::Tree => {
                        ui.heading("Arborescence (triée par taille)");
                        ui.add_space(8.0);

                        egui::ScrollArea::vertical()
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                draw_node_recursive(
                                    ui,
                                    root,
                                    total_size,
                                    0,
                                );
                            });
                    }
                    ViewMode::Treemap => {
                        ui.heading("Treemap façon WinDirStat");
                        ui.small(
                            "Chaque bloc représente un dossier/fichier, \
                             proportionnel à sa taille.",
                        );
                        ui.add_space(8.0);

                        draw_treemap(ui, root, &mut self.selected_node_path);
                    }
                }
            } else {
                ui.centered_and_justified(|ui| {
                    ui.label(
                        "Choisis un dossier ou un lecteur, puis lance un scan.",
                    );
                });
            }
        });
    }
}

impl TreeSizeApp {
    fn start_scan(&mut self, path: PathBuf) {
        self.is_scanning = true;
        self.status = format!(
            "Scan en cours pour : {}",
            path.to_string_lossy()
        );
        self.root_node = None;
        self.selected_node_path = None;

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
}

/// Dessin récursif d'un Node en arbre (vue arborescence).
fn draw_node_recursive(
    ui: &mut egui::Ui,
    node: &Node,
    root_size: u64,
    indent_level: usize,
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

    if node.is_dir {
        let header = egui::CollapsingHeader::new(label)
            .default_open(indent_level == 0)
            .id_source(node.path.to_string_lossy().to_string());

        header.show(ui, |ui| {
            for child in &node.children {
                ui.horizontal(|ui| {
                    ui.add_space(indent + 10.0);
                    draw_node_recursive(ui, child, root_size, indent_level + 1);
                });
            }
        });
    } else {
        ui.horizontal(|ui| {
            ui.add_space(indent + 10.0);
            ui.label(label);
        });
    }
}

/// Un bloc cliquable dans la treemap.
struct Hit {
    rect: egui::Rect,
    path: PathBuf,
    name: String,
    size: u64,
}

/// Dessin de la treemap façon WinDirStat.
fn draw_treemap(
    ui: &mut egui::Ui,
    root: &Node,
    selected_path: &mut Option<PathBuf>,
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
        // Sélection au clic
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

        let mut r = rect;
        if horizontal {
            let w = rect.width() * fraction;
            let x1 = offset;
            let x2 = (offset + w).min(rect.right());
            r = egui::Rect::from_min_max(
                egui::pos2(x1, rect.top()),
                egui::pos2(x2, rect.bottom()),
            );
            offset += w;
        } else {
            let h = rect.height() * fraction;
            let y1 = offset;
            let y2 = (offset + h).min(rect.bottom());
            r = egui::Rect::from_min_max(
                egui::pos2(rect.left(), y1),
                egui::pos2(rect.right(), y2),
            );
            offset += h;
        }

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

        // Label si suffisamment de place
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
        });

        // Descente récursive limitée en profondeur
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

    let h = (hash % 360) as f32 / 360.0; // [0,1]
    let s = 0.5 + ((hash >> 8) % 50) as f32 / 100.0; // [0.5,1.0]
    let v = 0.4 + ((hash >> 16) % 60) as f32 / 100.0; // [0.4,1.0]

    let hsva = egui::epaint::Hsva::new(
        h,
        s.min(1.0),
        v.min(1.0),
        1.0,
    );
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
            error: Some("Chemin sélectionné n'est pas un dossier".to_string()),
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
                error: Some(format!("Impossible de lire le dossier racine : {e}")),
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
/// Pour les dossiers, on scanne récursivement (en parallèle pour les sous-éléments),
/// en vérifiant régulièrement le flag d'annulation.
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
        } else {
            // On ignore les dossiers illisibles sans faire planter l'app
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
