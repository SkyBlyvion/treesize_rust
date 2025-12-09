use std::{
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread,
};

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

struct TreeSizeApp {
    root_path: Option<PathBuf>,
    root_node: Option<Node>,
    is_scanning: bool,
    status: String,
    scan_receiver: Option<Receiver<ScanResult>>,
    cancel_flag: Option<Arc<AtomicBool>>,

    // UI
    available_roots: Vec<PathBuf>,
    selected_root_index: usize,
    scan_mode: ScanMode,
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

                ui.heading("Infos");
                ui.small(
                    "• Les erreurs d'accès (permissions, fichiers spéciaux...) \
                     sont ignorées sans faire planter l'application.\n\
                     • L'arrêt du scan est coopératif : le scan se termine dès \
                     que possible.",
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
                ui.heading("Arborescence (triée par taille)");

                ui.add_space(8.0);

                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        draw_node_recursive(ui, root, total_size, 0);
                    });
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
}

/// Dessin récursif d'un Node en arbre.
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
